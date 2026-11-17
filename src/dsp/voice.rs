//! 16-voice polyphonic voice allocator & per-voice synth pipeline supporting
//! `Poly`, `Mono 1..4`, `Legato`, configurable `1..16` voice limit, `Transpose`,
//! `Microtuning` (Equal + 5 scales + custom cents), `Glide` (LOG / linear),
//! `Bender` up/down, analog pitch `DRIFT`, velocity sensitivity, CC64 sustain,
//! click-free voice stealing, and per-voice modulation hooks for `mod_engine`.

use crate::dsp::filter::{
    DualFilter, DualFilterModInput, FilterRoutingSettings, FilterSettings,
};
use crate::dsp::osc::{flush_denormal, OscModInput, OscSettings, Oscillator, NUM_OSCS};
use crate::params::{MicrotuningMode, PhantomSpireParams, PolyMode};
use crate::SharedSynthState;
use std::f32::consts::{FRAC_PI_4, SQRT_2, TAU};

pub const MAX_VOICES: usize = 16;
const HELD_STACK_CAP: usize = 32;

/// Built-in 12-pitch-class microtuning offsets in cents relative to 12-TET.
const JUST_CENTS: [f32; 12] = [
    0.0, 11.73, 3.91, 15.64, -13.69, -1.96, -9.78, 1.96, 13.69, -15.64, 17.60, -11.73,
];
const PYTHAGOREAN_CENTS: [f32; 12] = [
    0.0, -9.78, 3.91, -5.87, 7.82, -1.96, 11.73, 1.96, -7.82, 5.87, -3.91, 9.78,
];
const WERCKMEISTER3_CENTS: [f32; 12] = [
    0.0, -9.78, -7.82, -5.87, -9.78, -1.96, -11.73, -3.91, -7.82, -11.73, -3.91, -7.82,
];
const MAQAM_CENTS: [f32; 12] = [
    0.0, -10.0, -4.0, -25.0, -50.0, -2.0, -18.0, 2.0, -12.0, -14.0, -28.0, -50.0,
];

/// Compute the effective continuous MIDI pitch (in 12-TET semitone units)
/// after applying Spire's `MicrotuningMode` and optional custom 12-note cent offsets.
#[inline]
pub fn microtuned_midi_note(
    note: u8,
    mode: MicrotuningMode,
    custom_cents: Option<&[f32; 12]>,
) -> f32 {
    let n = note as f32;
    let pc = (note % 12) as usize;
    let base = match mode {
        MicrotuningMode::Equal => n,
        MicrotuningMode::Just => n + JUST_CENTS[pc] * 0.01,
        MicrotuningMode::Pythagorean => n + PYTHAGOREAN_CENTS[pc] * 0.01,
        MicrotuningMode::Werckmeister3 => n + WERCKMEISTER3_CENTS[pc] * 0.01,
        // 24-EDO quarter-tone scale anchored at C4 (MIDI 60)
        MicrotuningMode::QuarterTone => 60.0 + (n - 60.0) * 0.5,
        MicrotuningMode::Maqam => n + MAQAM_CENTS[pc] * 0.01,
    };
    if let Some(cents) = custom_cents {
        let c = cents[pc];
        if c.is_finite() {
            base + c.clamp(-100.0, 100.0) * 0.01
        } else {
            base
        }
    } else {
        base
    }
}

/// Snapshot of global voice-control parameters.
#[derive(Debug, Clone, Copy)]
pub struct GlobalVoiceSettings {
    pub max_voices: usize,
    pub poly_mode: PolyMode,
    pub transpose: i32,
    pub glide_ms: f32,
    pub glide_log: bool,
    pub bender_up: i32,
    pub bender_down: i32,
    pub drift: bool,
    pub pitch_wheel: f32,
    pub microtuning: MicrotuningMode,
    pub custom_microtuning_cents: [f32; 12],
    pub velocity_sensitivity: f32,
}

impl Default for GlobalVoiceSettings {
    fn default() -> Self {
        Self {
            max_voices: MAX_VOICES,
            poly_mode: PolyMode::Poly,
            transpose: 0,
            glide_ms: 0.0,
            glide_log: false,
            bender_up: 2,
            bender_down: 2,
            drift: false,
            pitch_wheel: 0.0,
            microtuning: MicrotuningMode::Equal,
            custom_microtuning_cents: [0.0; 12],
            velocity_sensitivity: 0.75,
        }
    }
}

impl GlobalVoiceSettings {
    /// Extract global voice settings from plugin parameters and lock-free shared state.
    pub fn from_params(
        params: &PhantomSpireParams,
        shared_synth: &SharedSynthState,
        midi_pitch_bend: f32,
    ) -> Self {
        let mut custom = [0.0_f32; 12];
        for (pc, slot) in custom.iter_mut().enumerate() {
            *slot = shared_synth.read_microtuning_cents(pc);
        }
        let combined_bend = (params.global.pitch_wheel.value() + midi_pitch_bend).clamp(-1.0, 1.0);
        Self {
            max_voices: (params.global.voices.value().clamp(1, MAX_VOICES as i32)) as usize,
            poly_mode: params.global.poly_mode.value(),
            transpose: params.global.transpose.value().clamp(-24, 24),
            glide_ms: params.global.glide.value().clamp(0.0, 2000.0),
            glide_log: params.global.glide_log.value(),
            bender_up: params.global.bender_up.value().clamp(0, 24),
            bender_down: params.global.bender_down.value().clamp(0, 24),
            drift: params.global.drift.value(),
            pitch_wheel: combined_bend,
            microtuning: params.global.microtuning.value(),
            custom_microtuning_cents: custom,
            velocity_sensitivity: params.master.velocity.value().clamp(0.0, 1.0),
        }
    }
}

/// Complete per-sample voice synthesis settings snapshot (oscillators + dual filter + global).
#[derive(Debug, Clone, Copy)]
pub struct VoiceRenderSettings {
    pub global: GlobalVoiceSettings,
    pub oscs: [OscSettings; NUM_OSCS],
    pub filters: [FilterSettings; 2],
    pub filter_routing: FilterRoutingSettings,
}

impl Default for VoiceRenderSettings {
    fn default() -> Self {
        let mut oscs = [OscSettings::default(); NUM_OSCS];
        for osc in oscs.iter_mut().skip(1) {
            osc.mix_level = 0.0;
        }
        let mut f2 = FilterSettings::default();
        f2.filter_type = crate::params::FilterType::HP4;
        f2.cutoff = 20.0;
        Self {
            global: GlobalVoiceSettings::default(),
            oscs,
            filters: [FilterSettings::default(), f2],
            filter_routing: FilterRoutingSettings::default(),
        }
    }
}

impl VoiceRenderSettings {
    pub fn from_params(
        params: &PhantomSpireParams,
        shared_synth: &SharedSynthState,
        midi_pitch_bend: f32,
    ) -> Self {
        Self {
            global: GlobalVoiceSettings::from_params(params, shared_synth, midi_pitch_bend),
            oscs: [
                OscSettings::from_params(&params.oscs[0]),
                OscSettings::from_params(&params.oscs[1]),
                OscSettings::from_params(&params.oscs[2]),
                OscSettings::from_params(&params.oscs[3]),
            ],
            filters: [
                FilterSettings::from_params(&params.filters[0]),
                FilterSettings::from_params(&params.filters[1]),
            ],
            filter_routing: FilterRoutingSettings::from_params(&params.filter_routing),
        }
    }
}

/// Per-voice modulation offsets supplied by `mod_engine` (`ENV 1..4`, `LFO 1..4`,
/// `STP 1..2`, `Mod Matrix`) for each active voice on each sample.
#[derive(Debug, Clone, Copy)]
pub struct VoiceModulation {
    /// Amplitude envelope gain (typically ENV 1, range 0.0..=1.0+).
    pub amp_env: f32,
    /// Shared pitch modulation applied to all 4 oscillators (semitones).
    pub all_osc_pitch_st: f32,
    /// Individual oscillator pitch modulation (semitones).
    pub osc_pitch_st: [f32; NUM_OSCS],
    /// Individual oscillator `ctrlA` additive modulation (-1..1).
    pub osc_ctrl_a: [f32; NUM_OSCS],
    /// Individual oscillator `ctrlB` additive modulation (-1..1).
    pub osc_ctrl_b: [f32; NUM_OSCS],
    /// Individual oscillator `wt_mix` additive modulation (-1..1).
    pub osc_wt_mix: [f32; NUM_OSCS],
    /// Individual oscillator `mix_level` additive modulation (-1..1).
    pub osc_level: [f32; NUM_OSCS],
    /// Individual oscillator `pan` additive modulation (-1..1).
    pub osc_pan: [f32; NUM_OSCS],
    /// Individual oscillator `unison_detune` additive modulation (-1..1).
    pub osc_detune: [f32; NUM_OSCS],
    /// Filter 1 & 2 cutoff modulation in octaves (e.g., -8..+8).
    pub filter_cutoff_oct: [f32; 2],
    /// Filter 1 & 2 resonance additive modulation (-1..1).
    pub filter_res: [f32; 2],
    /// Dual-filter balance additive modulation (-1..1).
    pub filter_balance: f32,
    /// Voice-level stereo pan modulation (-1..1).
    pub voice_pan: f32,
}

impl Default for VoiceModulation {
    fn default() -> Self {
        Self::unity()
    }
}

impl VoiceModulation {
    /// Neutral modulation with unity amplitude (`amp_env = 1.0`) and all offsets at `0.0`.
    #[inline]
    pub const fn unity() -> Self {
        Self {
            amp_env: 1.0,
            all_osc_pitch_st: 0.0,
            osc_pitch_st: [0.0; NUM_OSCS],
            osc_ctrl_a: [0.0; NUM_OSCS],
            osc_ctrl_b: [0.0; NUM_OSCS],
            osc_wt_mix: [0.0; NUM_OSCS],
            osc_level: [0.0; NUM_OSCS],
            osc_pan: [0.0; NUM_OSCS],
            osc_detune: [0.0; NUM_OSCS],
            filter_cutoff_oct: [0.0; 2],
            filter_res: [0.0; 2],
            filter_balance: 0.0,
            voice_pan: 0.0,
        }
    }
}

/// Lifecycle notification returned by `VoiceAllocator::note_on` / `note_off`
/// so `mod_engine` can trigger, legato-hold, or release per-voice envelopes & LFOs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoiceTriggerEvent {
    None,
    NoteOn {
        voice_idx: usize,
        note: u8,
        velocity: f32,
        retrigger_env: bool,
        stolen: bool,
    },
    NoteOff {
        voice_idx: usize,
    },
    PitchChange {
        voice_idx: usize,
        note: u8,
        velocity: f32,
        retrigger_env: bool,
    },
}

/// Single polyphonic synth voice containing 4 oscillators, dual stereo filter,
/// portamento/glide, analog pitch drift, and click-free steal de-clicker.
pub struct Voice {
    voice_idx: usize,
    sample_rate: f32,
    active: bool,
    gate_open: bool,
    sustained: bool,
    note: u8,
    channel: u8,
    velocity: f32,
    age: u64,

    // Pitch & glide state (continuous MIDI note scale)
    current_pitch: f32,
    target_pitch: f32,
    glide_linear_step: f32,
    glide_linear_remaining: u32,
    glide_log_coeff: f32,
    glide_is_log: bool,

    // Analog pitch drift state
    drift_phase_a: f32,
    drift_phase_b: f32,
    drift_inc_a: f32,
    drift_inc_b: f32,
    rng_state: u32,
    random_bipolar: f32,

    // Built-in fallback de-click envelope (used when caller calls `process_sample_internal`)
    internal_env: f32,

    // Anti-click crossfade when stealing an active voice
    steal_fade_l: f32,
    steal_fade_r: f32,
    steal_fade_pos: f32,
    steal_fade_step: f32,
    last_out_l: f32,
    last_out_r: f32,

    oscs: [Oscillator; NUM_OSCS],
    dual_filter: DualFilter,
}

impl Voice {
    pub fn new(voice_idx: usize, sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() && sample_rate >= 8000.0 {
            sample_rate
        } else {
            44100.0
        };
        let seed = ((voice_idx as u32) + 1)
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add(0x1337_BEEF);
        let oscs = [
            Oscillator::new(sr, seed.wrapping_add(11)),
            Oscillator::new(sr, seed.wrapping_add(23)),
            Oscillator::new(sr, seed.wrapping_add(37)),
            Oscillator::new(sr, seed.wrapping_add(53)),
        ];

        // Slow inharmonic analog drift frequencies (~0.17..0.75 Hz)
        let fa = 0.17 + (voice_idx as f32) * 0.023;
        let fb = 0.41 + (voice_idx as f32) * 0.037;

        Self {
            voice_idx,
            sample_rate: sr,
            active: false,
            gate_open: false,
            sustained: false,
            note: 60,
            channel: 0,
            velocity: 0.0,
            age: 0,
            current_pitch: 60.0,
            target_pitch: 60.0,
            glide_linear_step: 0.0,
            glide_linear_remaining: 0,
            glide_log_coeff: 1.0,
            glide_is_log: false,
            drift_phase_a: ((voice_idx as f32) * 0.618_034).fract() * TAU,
            drift_phase_b: ((voice_idx as f32) * 0.381_966).fract() * TAU,
            drift_inc_a: TAU * fa / sr,
            drift_inc_b: TAU * fb / sr,
            rng_state: seed | 1,
            random_bipolar: 0.0,
            internal_env: 0.0,
            steal_fade_l: 0.0,
            steal_fade_r: 0.0,
            steal_fade_pos: 0.0,
            steal_fade_step: 1.0 / (0.003 * sr).max(1.0),
            last_out_l: 0.0,
            last_out_r: 0.0,
            oscs,
            dual_filter: DualFilter::new(sr),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate >= 8000.0 {
            self.sample_rate = sample_rate;
            let fa = 0.17 + (self.voice_idx as f32) * 0.023;
            let fb = 0.41 + (self.voice_idx as f32) * 0.037;
            self.drift_inc_a = TAU * fa / sample_rate;
            self.drift_inc_b = TAU * fb / sample_rate;
            self.steal_fade_step = 1.0 / (0.003 * sample_rate).max(1.0);
            for osc in &mut self.oscs {
                osc.set_sample_rate(sample_rate);
            }
            self.dual_filter.set_sample_rate(sample_rate);
        }
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.gate_open = false;
        self.sustained = false;
        self.velocity = 0.0;
        self.age = 0;
        self.glide_linear_remaining = 0;
        self.internal_env = 0.0;
        self.steal_fade_pos = 0.0;
        self.steal_fade_l = 0.0;
        self.steal_fade_r = 0.0;
        self.last_out_l = 0.0;
        self.last_out_r = 0.0;
        for osc in &mut self.oscs {
            osc.reset();
        }
        self.dual_filter.reset();
    }

    #[inline]
    pub fn voice_idx(&self) -> usize {
        self.voice_idx
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[inline]
    pub fn is_gate_open(&self) -> bool {
        self.gate_open
    }

    #[inline]
    pub fn note(&self) -> u8 {
        self.note
    }

    #[inline]
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Normalized keytrack modulation source in [-1.0, 1.0] centered at C4 (MIDI 60).
    #[inline]
    pub fn keytrack(&self) -> f32 {
        ((self.current_pitch - 60.0) / 48.0).clamp(-1.0, 1.0)
    }

    /// Current instantaneous pitch (including glide & microtuning) in MIDI semitones.
    #[inline]
    pub fn current_pitch(&self) -> f32 {
        self.current_pitch
    }

    /// Per-note random bipolar value in [-1.0, 1.0] for `ModSource::Random`.
    #[inline]
    pub fn random_bipolar(&self) -> f32 {
        self.random_bipolar
    }

    #[inline]
    pub fn age(&self) -> u64 {
        self.age
    }

    /// Mark voice inactive once external amp envelope (`ENV 1`) finishes release.
    #[inline]
    pub fn mark_finished(&mut self) {
        self.active = false;
        self.gate_open = false;
        self.sustained = false;
        self.internal_env = 0.0;
        self.last_out_l = 0.0;
        self.last_out_r = 0.0;
    }

    fn next_random_bipolar(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x | 1;
        (x as f32) * (2.0 / u32::MAX as f32) - 1.0
    }

    #[allow(clippy::too_many_arguments)]
    fn trigger(
        &mut self,
        note: u8,
        velocity: f32,
        channel: u8,
        age: u64,
        start_pitch: Option<f32>,
        retrigger: bool,
        stolen: bool,
        settings: &VoiceRenderSettings,
    ) {
        if stolen && self.active {
            // Capture current output for click-free 3 ms micro-crossfade
            self.steal_fade_l = self.last_out_l;
            self.steal_fade_r = self.last_out_r;
            self.steal_fade_pos = 1.0;
        } else if !self.active {
            self.steal_fade_pos = 0.0;
        }

        self.active = true;
        self.gate_open = true;
        self.sustained = false;
        self.note = note & 0x7F;
        self.channel = channel;
        self.velocity = velocity.clamp(0.0, 1.0);
        self.age = age;

        let target = microtuned_midi_note(
            self.note,
            settings.global.microtuning,
            Some(&settings.global.custom_microtuning_cents),
        );
        self.target_pitch = target;

        if let Some(from_p) = start_pitch {
            self.current_pitch = from_p;
            self.configure_glide(settings.global.glide_ms, settings.global.glide_log);
        } else {
            self.current_pitch = target;
            self.glide_linear_remaining = 0;
            self.glide_linear_step = 0.0;
            self.glide_log_coeff = 1.0;
            self.glide_is_log = false;
        }

        if retrigger {
            self.random_bipolar = self.next_random_bipolar();
            // Advance drift phase slightly per trigger for organic per-note variation
            self.drift_phase_a = (self.drift_phase_a + 1.13).rem_euclid(TAU);
            self.drift_phase_b = (self.drift_phase_b + 0.79).rem_euclid(TAU);

            for (osc, osc_cfg) in self.oscs.iter_mut().zip(settings.oscs.iter()) {
                osc.note_on(osc_cfg);
            }
            if !stolen {
                self.dual_filter.reset();
            }
        }
    }

    fn retarget_pitch(
        &mut self,
        note: u8,
        velocity: f32,
        retrigger: bool,
        should_glide: bool,
        settings: &VoiceRenderSettings,
    ) {
        self.note = note & 0x7F;
        self.velocity = velocity.clamp(0.0, 1.0);
        self.gate_open = true;
        self.sustained = false;

        let target = microtuned_midi_note(
            self.note,
            settings.global.microtuning,
            Some(&settings.global.custom_microtuning_cents),
        );
        self.target_pitch = target;

        if should_glide && settings.global.glide_ms > 0.1 {
            self.configure_glide(settings.global.glide_ms, settings.global.glide_log);
        } else {
            self.current_pitch = target;
            self.glide_linear_remaining = 0;
            self.glide_linear_step = 0.0;
            self.glide_log_coeff = 1.0;
            self.glide_is_log = false;
        }

        if retrigger {
            self.random_bipolar = self.next_random_bipolar();
            for (osc, osc_cfg) in self.oscs.iter_mut().zip(settings.oscs.iter()) {
                osc.note_on(osc_cfg);
            }
        }
    }

    fn configure_glide(&mut self, glide_ms: f32, glide_log: bool) {
        let clamped_ms = glide_ms.clamp(0.0, 2000.0);
        if clamped_ms <= 0.1 || (self.target_pitch - self.current_pitch).abs() < 1e-5 {
            self.current_pitch = self.target_pitch;
            self.glide_linear_remaining = 0;
            self.glide_linear_step = 0.0;
            self.glide_log_coeff = 1.0;
            self.glide_is_log = false;
            return;
        }

        let samples = (clamped_ms * 0.001 * self.sample_rate).max(1.0);
        self.glide_is_log = glide_log;
        if glide_log {
            // Exponential RC curve: reaches ~99% of target in `glide_ms`
            self.glide_log_coeff = (1.0 - (-4.605_17 / samples).exp()).clamp(1e-6, 1.0);
            self.glide_linear_remaining = 0;
            self.glide_linear_step = 0.0;
        } else {
            // Exact constant-rate linear semitone ramp
            let steps = (samples.round() as u32).max(1);
            self.glide_linear_step = (self.target_pitch - self.current_pitch) / (steps as f32);
            self.glide_linear_remaining = steps;
            self.glide_log_coeff = 1.0;
        }
    }

    #[inline]
    fn advance_pitch_and_drift(&mut self, settings: &GlobalVoiceSettings) -> f32 {
        // Recompute target live in case microtuning mode changed while note is held
        self.target_pitch = microtuned_midi_note(
            self.note,
            settings.microtuning,
            Some(&settings.custom_microtuning_cents),
        );

        if self.glide_is_log {
            let diff = self.target_pitch - self.current_pitch;
            if diff.abs() < 1e-4 {
                self.current_pitch = self.target_pitch;
            } else {
                self.current_pitch += diff * self.glide_log_coeff;
            }
        } else if self.glide_linear_remaining > 0 {
            self.current_pitch += self.glide_linear_step;
            self.glide_linear_remaining -= 1;
            if self.glide_linear_remaining == 0 {
                self.current_pitch = self.target_pitch;
            }
        } else {
            self.current_pitch = self.target_pitch;
        }

        // Pitch wheel with asymmetric up/down bender range
        let bend_st = if settings.pitch_wheel >= 0.0 {
            settings.pitch_wheel * (settings.bender_up as f32)
        } else {
            settings.pitch_wheel * (settings.bender_down as f32)
        };

        // Analog pitch drift (smooth dual-oscillator wander, +/- 18 cents)
        self.drift_phase_a += self.drift_inc_a;
        if self.drift_phase_a >= TAU {
            self.drift_phase_a -= TAU;
        }
        self.drift_phase_b += self.drift_inc_b;
        if self.drift_phase_b >= TAU {
            self.drift_phase_b -= TAU;
        }
        let drift_st = if settings.drift {
            (0.65 * self.drift_phase_a.sin() + 0.35 * self.drift_phase_b.sin()) * 0.18
        } else {
            0.0
        };

        self.current_pitch + (settings.transpose as f32) + bend_st + drift_st
    }

    /// Render one stereo sample for this voice using external modulation (`mod_in`).
    /// This is the primary hot-path called by `SynthEngine` once `mod_engine` has
    /// evaluated per-voice ENV/LFO/STP/ModMatrix targets.
    #[inline]
    pub fn render_sample(
        &mut self,
        settings: &VoiceRenderSettings,
        mod_in: &VoiceModulation,
    ) -> (f32, f32) {
        if !self.active {
            return (0.0, 0.0);
        }

        let base_pitch = self.advance_pitch_and_drift(&settings.global);

        let mut f1_l = 0.0_f32;
        let mut f1_r = 0.0_f32;
        let mut f2_l = 0.0_f32;
        let mut f2_r = 0.0_f32;

        for i in 0..NUM_OSCS {
            let osc_mod = OscModInput {
                pitch_st: mod_in.all_osc_pitch_st + mod_in.osc_pitch_st[i],
                ctrl_a: mod_in.osc_ctrl_a[i],
                ctrl_b: mod_in.osc_ctrl_b[i],
                wt_mix: mod_in.osc_wt_mix[i],
                level: mod_in.osc_level[i],
                pan: mod_in.osc_pan[i],
                detune: mod_in.osc_detune[i],
            };
            let o = self.oscs[i].process_sample(base_pitch, &settings.oscs[i], &osc_mod);
            f1_l += o.f1_l;
            f1_r += o.f1_r;
            f2_l += o.f2_l;
            f2_r += o.f2_r;
        }

        let filt_mod = DualFilterModInput {
            cutoff_oct: mod_in.filter_cutoff_oct,
            res: mod_in.filter_res,
            balance: mod_in.filter_balance,
        };
        let (filt_l, filt_r) = self.dual_filter.process_stereo(
            f1_l,
            f1_r,
            f2_l,
            f2_r,
            base_pitch,
            &settings.filters[0],
            &settings.filters[1],
            &settings.filter_routing,
            &filt_mod,
        );

        // Velocity scaling & amp envelope
        let vel_sens = settings.global.velocity_sensitivity.clamp(0.0, 1.0);
        let vel_gain = (1.0 - vel_sens) + vel_sens * self.velocity;
        let env_gain = mod_in.amp_env.clamp(0.0, 2.0);
        // Per-voice nominal headroom factor keeps multi-voice chords clean
        let gain = vel_gain * env_gain * 0.45;

        // Voice-level stereo pan modulation
        let pan = mod_in.voice_pan.clamp(-1.0, 1.0);
        let pan_angle = (pan + 1.0) * FRAC_PI_4;
        let pan_l = pan_angle.cos() * SQRT_2;
        let pan_r = pan_angle.sin() * SQRT_2;

        let mut out_l = flush_denormal(filt_l * gain * pan_l);
        let mut out_r = flush_denormal(filt_r * gain * pan_r);

        // Click-free 3 ms crossfade if this voice just stole a sounding note
        if self.steal_fade_pos > 0.0 {
            let old_w = self.steal_fade_pos;
            let new_w = 1.0 - old_w;
            out_l = self.steal_fade_l * old_w + out_l * new_w;
            out_r = self.steal_fade_r * old_w + out_r * new_w;
            self.steal_fade_pos = (self.steal_fade_pos - self.steal_fade_step).max(0.0);
        }

        self.last_out_l = out_l;
        self.last_out_r = out_r;
        (out_l, out_r)
    }

    /// Standalone per-sample render using a lightweight built-in 2 ms attack / 25 ms
    /// release envelope (used when `VoiceAllocator` is driven without external `mod_engine`).
    #[inline]
    pub fn render_sample_internal(&mut self, settings: &VoiceRenderSettings) -> (f32, f32) {
        if !self.active {
            return (0.0, 0.0);
        }
        if self.gate_open {
            let att = 1.0 / (0.002 * self.sample_rate).max(1.0);
            self.internal_env = (self.internal_env + att).min(1.0);
        } else {
            let rel = 1.0 / (0.025 * self.sample_rate).max(1.0);
            self.internal_env = (self.internal_env - rel).max(0.0);
            if self.internal_env <= 1e-5 {
                self.mark_finished();
                return (0.0, 0.0);
            }
        }
        let mut m = VoiceModulation::unity();
        m.amp_env = self.internal_env;
        self.render_sample(settings, &m)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct HeldNote {
    note: u8,
    velocity: f32,
}

/// 16-voice polyphonic/monophonic/legato voice allocator with held-note stack,
/// CC64 sustain, and click-free voice stealing.
pub struct VoiceAllocator {
    sample_rate: f32,
    voices: [Voice; MAX_VOICES],
    age_counter: u64,
    last_triggered_pitch: Option<f32>,
    last_note: i8,
    sustain_down: bool,
    held_stack: [HeldNote; HELD_STACK_CAP],
    held_len: usize,
    input_note_counts: [u8; 128],
}

impl VoiceAllocator {
    pub fn new(sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() && sample_rate >= 8000.0 {
            sample_rate
        } else {
            44100.0
        };
        let voices = std::array::from_fn(|i| Voice::new(i, sr));
        Self {
            sample_rate: sr,
            voices,
            age_counter: 0,
            last_triggered_pitch: None,
            last_note: -1,
            sustain_down: false,
            held_stack: [HeldNote::default(); HELD_STACK_CAP],
            held_len: 0,
            input_note_counts: [0; 128],
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate >= 8000.0 {
            self.sample_rate = sample_rate;
            for v in &mut self.voices {
                v.set_sample_rate(sample_rate);
            }
        }
    }

    pub fn reset(&mut self) {
        for v in &mut self.voices {
            v.reset();
        }
        self.age_counter = 0;
        self.last_triggered_pitch = None;
        self.last_note = -1;
        self.sustain_down = false;
        self.held_len = 0;
        self.input_note_counts = [0; 128];
    }

    #[inline]
    pub fn voices(&self) -> &[Voice; MAX_VOICES] {
        &self.voices
    }

    #[inline]
    pub fn voices_mut(&mut self) -> &mut [Voice; MAX_VOICES] {
        &mut self.voices
    }

    #[inline]
    pub fn active_voice_count(&self) -> u32 {
        self.voices.iter().filter(|v| v.active).count() as u32
    }

    #[inline]
    pub fn last_note(&self) -> i8 {
        self.last_note
    }

    /// Bitmask of currently sounding (active) output notes across all voices.
    pub fn active_output_note_mask(&self) -> u128 {
        let mut mask = 0u128;
        for v in &self.voices {
            if v.active {
                mask |= 1u128 << (v.note & 0x7F);
            }
        }
        mask
    }

    /// Bitmask of currently held MIDI input notes.
    pub fn active_input_note_mask(&self) -> u128 {
        let mut mask = 0u128;
        for (i, &c) in self.input_note_counts.iter().enumerate() {
            if c > 0 {
                mask |= 1u128 << i;
            }
        }
        mask
    }

    /// Notify allocator that an external amp envelope (`ENV 1`) completed release.
    #[inline]
    pub fn finish_voice(&mut self, voice_idx: usize) {
        if voice_idx < MAX_VOICES {
            self.voices[voice_idx].mark_finished();
        }
    }

    fn push_held(&mut self, note: u8, velocity: f32) {
        let n = note & 0x7F;
        // Remove existing duplicate entry if present
        self.remove_held(n);
        if self.held_len == HELD_STACK_CAP {
            // Drop oldest held note
            for i in 1..HELD_STACK_CAP {
                self.held_stack[i - 1] = self.held_stack[i];
            }
            self.held_len -= 1;
        }
        self.held_stack[self.held_len] = HeldNote { note: n, velocity };
        self.held_len += 1;
    }

    fn remove_held(&mut self, note: u8) {
        let n = note & 0x7F;
        let mut write = 0;
        for read in 0..self.held_len {
            if self.held_stack[read].note != n {
                self.held_stack[write] = self.held_stack[read];
                write += 1;
            }
        }
        self.held_len = write;
    }

    #[inline]
    fn is_mono_mode(mode: PolyMode) -> bool {
        !matches!(mode, PolyMode::Poly)
    }

    /// Handle MIDI NoteOn (velocity <= 0 delegates to `note_off`).
    pub fn note_on(
        &mut self,
        note: u8,
        velocity: f32,
        channel: u8,
        settings: &VoiceRenderSettings,
    ) -> VoiceTriggerEvent {
        if !velocity.is_finite() || velocity <= 0.0 {
            return self.note_off(note, channel, settings);
        }

        let n = note & 0x7F;
        let vel = velocity.clamp(0.0, 1.0);
        self.input_note_counts[n as usize] = self.input_note_counts[n as usize].saturating_add(1);
        self.last_note = n as i8;

        let target_pitch = microtuned_midi_note(
            n,
            settings.global.microtuning,
            Some(&settings.global.custom_microtuning_cents),
        );

        let poly_mode = settings.global.poly_mode;

        if Self::is_mono_mode(poly_mode) {
            let was_held = self.held_len > 0 && self.voices[0].active && self.voices[0].gate_open;
            self.push_held(n, vel);

            // Silence any stray poly voices > 0 when switching into Mono/Legato
            for v in self.voices.iter_mut().skip(1) {
                if v.active {
                    v.gate_open = false;
                    v.sustained = false;
                }
            }

            let retrigger = match poly_mode {
                PolyMode::Mono1 | PolyMode::Mono2 => true,
                PolyMode::Mono3 | PolyMode::Mono4 | PolyMode::Legato => !was_held,
                PolyMode::Poly => true,
            };
            let should_glide = match poly_mode {
                PolyMode::Mono1 | PolyMode::Mono3 => {
                    self.last_triggered_pitch.is_some() && settings.global.glide_ms > 0.1
                }
                PolyMode::Mono2 | PolyMode::Mono4 | PolyMode::Legato => {
                    was_held && settings.global.glide_ms > 0.1
                }
                PolyMode::Poly => false,
            };

            let start_pitch = if should_glide {
                if self.voices[0].active {
                    Some(self.voices[0].current_pitch)
                } else {
                    self.last_triggered_pitch
                }
            } else {
                None
            };

            self.age_counter = self.age_counter.wrapping_add(1);
            let age = self.age_counter;
            let stolen = self.voices[0].active && retrigger;

            if self.voices[0].active && !retrigger {
                self.voices[0].retarget_pitch(n, vel, false, should_glide, settings);
                self.last_triggered_pitch = Some(target_pitch);
                return VoiceTriggerEvent::PitchChange {
                    voice_idx: 0,
                    note: n,
                    velocity: vel,
                    retrigger_env: false,
                };
            }

            self.voices[0].trigger(n, vel, channel, age, start_pitch, true, stolen, settings);
            self.last_triggered_pitch = Some(target_pitch);
            return VoiceTriggerEvent::NoteOn {
                voice_idx: 0,
                note: n,
                velocity: vel,
                retrigger_env: true,
                stolen,
            };
        }

        // Polyphonic mode (`Poly`)
        self.push_held(n, vel);
        let limit = settings.global.max_voices.clamp(1, MAX_VOICES);

        // Release voices beyond reduced max_voices limit
        for v in self.voices.iter_mut().skip(limit) {
            if v.active {
                v.gate_open = false;
                v.sustained = false;
            }
        }

        // 1. Reuse same note if already active
        let mut chosen_idx = None;
        let mut stolen = false;

        for i in 0..limit {
            if self.voices[i].active && self.voices[i].note == n {
                chosen_idx = Some(i);
                stolen = true;
                break;
            }
        }

        // 2. Otherwise pick first free voice
        if chosen_idx.is_none() {
            for i in 0..limit {
                if !self.voices[i].active {
                    chosen_idx = Some(i);
                    stolen = false;
                    break;
                }
            }
        }

        // 3. Otherwise steal best candidate: oldest releasing voice, else oldest active voice
        if chosen_idx.is_none() {
            let mut best_rel: Option<(usize, u64)> = None;
            let mut best_any: Option<(usize, u64)> = None;
            for i in 0..limit {
                let v = &self.voices[i];
                if !v.gate_open {
                    if best_rel.map_or(true, |(_, a)| v.age < a) {
                        best_rel = Some((i, v.age));
                    }
                }
                if best_any.map_or(true, |(_, a)| v.age < a) {
                    best_any = Some((i, v.age));
                }
            }
            chosen_idx = best_rel.or(best_any).map(|(i, _)| i);
            stolen = true;
        }

        let idx = chosen_idx.unwrap_or(0);
        let start_pitch = if settings.global.glide_ms > 0.1 {
            self.last_triggered_pitch
        } else {
            None
        };

        self.age_counter = self.age_counter.wrapping_add(1);
        let age = self.age_counter;
        self.voices[idx].trigger(n, vel, channel, age, start_pitch, true, stolen, settings);
        self.last_triggered_pitch = Some(target_pitch);

        VoiceTriggerEvent::NoteOn {
            voice_idx: idx,
            note: n,
            velocity: vel,
            retrigger_env: true,
            stolen,
        }
    }

    /// Handle MIDI NoteOff.
    pub fn note_off(
        &mut self,
        note: u8,
        _channel: u8,
        settings: &VoiceRenderSettings,
    ) -> VoiceTriggerEvent {
        let n = note & 0x7F;
        self.input_note_counts[n as usize] = self.input_note_counts[n as usize].saturating_sub(1);
        self.remove_held(n);

        if Self::is_mono_mode(settings.global.poly_mode) {
            if self.held_len > 0 && self.voices[0].active {
                // Return to previously held note on the stack (last-note priority)
                let prev = self.held_stack[self.held_len - 1];
                if self.voices[0].note != prev.note {
                    let retrigger = matches!(
                        settings.global.poly_mode,
                        PolyMode::Mono1 | PolyMode::Mono2
                    );
                    let should_glide = settings.global.glide_ms > 0.1;
                    self.voices[0].retarget_pitch(
                        prev.note,
                        prev.velocity,
                        retrigger,
                        should_glide,
                        settings,
                    );
                    self.last_note = prev.note as i8;
                    return VoiceTriggerEvent::PitchChange {
                        voice_idx: 0,
                        note: prev.note,
                        velocity: prev.velocity,
                        retrigger_env: retrigger,
                    };
                }
                return VoiceTriggerEvent::None;
            }
            if self.voices[0].active && self.voices[0].gate_open {
                if self.sustain_down {
                    self.voices[0].sustained = true;
                    return VoiceTriggerEvent::None;
                }
                self.voices[0].gate_open = false;
                return VoiceTriggerEvent::NoteOff { voice_idx: 0 };
            }
            return VoiceTriggerEvent::None;
        }

        // Poly mode: release matching voice
        for i in 0..MAX_VOICES {
            if self.voices[i].active && self.voices[i].gate_open && self.voices[i].note == n {
                if self.sustain_down {
                    self.voices[i].sustained = true;
                    return VoiceTriggerEvent::None;
                }
                self.voices[i].gate_open = false;
                return VoiceTriggerEvent::NoteOff { voice_idx: i };
            }
        }
        VoiceTriggerEvent::None
    }

    /// Handle MIDI CC64 sustain pedal. Returns `(released_indices, count)` when pedal lifts
    /// so `mod_engine` can release envelopes for any previously sustained voices.
    pub fn set_sustain_pedal(&mut self, down: bool) -> ([usize; MAX_VOICES], usize) {
        self.sustain_down = down;
        let mut released = [0usize; MAX_VOICES];
        let mut count = 0usize;
        if !down {
            for i in 0..MAX_VOICES {
                if self.voices[i].active && self.voices[i].sustained {
                    self.voices[i].sustained = false;
                    self.voices[i].gate_open = false;
                    released[count] = i;
                    count += 1;
                }
            }
        }
        (released, count)
    }

    /// Release all active voices cleanly (keeps release tails alive).
    pub fn all_notes_off(&mut self) -> ([usize; MAX_VOICES], usize) {
        self.held_len = 0;
        self.input_note_counts = [0; 128];
        self.sustain_down = false;
        let mut released = [0usize; MAX_VOICES];
        let mut count = 0usize;
        for i in 0..MAX_VOICES {
            if self.voices[i].active && self.voices[i].gate_open {
                self.voices[i].gate_open = false;
                self.voices[i].sustained = false;
                released[count] = i;
                count += 1;
            }
        }
        (released, count)
    }

    /// Immediately silence all voices (hard panic/reset).
    pub fn all_sound_off(&mut self) {
        self.reset();
    }

    /// Standalone render across all active voices using each voice's built-in de-click
    /// envelope (useful when testing `VoiceAllocator` without an external `mod_engine`).
    #[inline]
    pub fn process_sample_internal(&mut self, settings: &VoiceRenderSettings) -> (f32, f32) {
        let mut sum_l = 0.0_f32;
        let mut sum_r = 0.0_f32;
        for v in &mut self.voices {
            if v.active {
                let (l, r) = v.render_sample_internal(settings);
                sum_l += l;
                sum_r += r;
            }
        }
        (flush_denormal(sum_l), flush_denormal(sum_r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_diff(a: &[(f32, f32)], b: &[(f32, f32)]) -> f32 {
        let sum: f32 = a
            .iter()
            .zip(b.iter())
            .map(|((al, ar), (bl, br))| (al - bl).powi(2) + (ar - br).powi(2))
            .sum();
        (sum / (2.0 * a.len() as f32)).sqrt()
    }

    #[test]
    fn polyphony_max_voices_and_click_free_stealing() {
        let sr = 48000.0;
        let mut alloc = VoiceAllocator::new(sr);
        let mut settings = VoiceRenderSettings::default();
        settings.global.max_voices = 4;

        for n in 60..64 {
            let ev = alloc.note_on(n, 0.8, 0, &settings);
            assert!(matches!(ev, VoiceTriggerEvent::NoteOn { stolen: false, .. }));
        }
        assert_eq!(alloc.active_voice_count(), 4);

        // Render a short block so voices have non-zero output
        let mut last = (0.0_f32, 0.0_f32);
        for _ in 0..256 {
            last = alloc.process_sample_internal(&settings);
        }

        // Trigger a 5th note -> must steal oldest voice without huge discontinuity
        let steal_ev = alloc.note_on(72, 0.9, 0, &settings);
        assert!(matches!(steal_ev, VoiceTriggerEvent::NoteOn { stolen: true, .. }));
        assert_eq!(alloc.active_voice_count(), 4);

        let first_after = alloc.process_sample_internal(&settings);
        assert!(first_after.0.is_finite() && first_after.1.is_finite());
        // Steal crossfade ensures no abrupt jump > 0.35 on the steal boundary
        assert!(
            (first_after.0 - last.0).abs() < 0.35,
            "Click on voice steal: delta={}",
            (first_after.0 - last.0).abs()
        );
    }

    #[test]
    fn mono_and_legato_modes_retrigger_and_return_to_held_notes() {
        let sr = 48000.0;
        let mut settings = VoiceRenderSettings::default();
        settings.global.glide_ms = 50.0;

        // Mono1 retriggers on overlap
        settings.global.poly_mode = PolyMode::Mono1;
        let mut alloc = VoiceAllocator::new(sr);
        let e1 = alloc.note_on(60, 0.8, 0, &settings);
        assert!(matches!(e1, VoiceTriggerEvent::NoteOn { voice_idx: 0, retrigger_env: true, .. }));
        let e2 = alloc.note_on(67, 0.8, 0, &settings);
        assert!(matches!(e2, VoiceTriggerEvent::NoteOn { voice_idx: 0, retrigger_env: true, .. }));
        assert_eq!(alloc.active_voice_count(), 1);

        // Legato does NOT retrigger envelope on overlap and returns to note 60 on release of 67
        settings.global.poly_mode = PolyMode::Legato;
        let mut leg = VoiceAllocator::new(sr);
        let l1 = leg.note_on(60, 0.8, 0, &settings);
        assert!(matches!(l1, VoiceTriggerEvent::NoteOn { voice_idx: 0, retrigger_env: true, .. }));
        let l2 = leg.note_on(67, 0.8, 0, &settings);
        assert!(matches!(
            l2,
            VoiceTriggerEvent::PitchChange {
                voice_idx: 0,
                note: 67,
                retrigger_env: false,
                ..
            }
        ));
        let l3 = leg.note_off(67, 0, &settings);
        assert!(matches!(
            l3,
            VoiceTriggerEvent::PitchChange {
                voice_idx: 0,
                note: 60,
                retrigger_env: false,
                ..
            }
        ));
        assert_eq!(leg.active_voice_count(), 1);
    }

    #[test]
    fn glide_log_vs_linear_produce_different_pitch_trajectories() {
        let sr = 48000.0;
        let mut s_lin = VoiceRenderSettings::default();
        s_lin.global.poly_mode = PolyMode::Mono1;
        s_lin.global.glide_ms = 100.0;
        s_lin.global.glide_log = false;

        let mut s_log = s_lin;
        s_log.global.glide_log = true;

        let mut a_lin = VoiceAllocator::new(sr);
        let mut a_log = VoiceAllocator::new(sr);
        a_lin.note_on(60, 0.8, 0, &s_lin);
        a_log.note_on(60, 0.8, 0, &s_log);
        for _ in 0..64 {
            a_lin.process_sample_internal(&s_lin);
            a_log.process_sample_internal(&s_log);
        }

        a_lin.note_on(72, 0.8, 0, &s_lin);
        a_log.note_on(72, 0.8, 0, &s_log);

        // After 25% of glide duration (25 ms = 1200 samples), log glide moves faster initially than linear!
        for _ in 0..1200 {
            a_lin.process_sample_internal(&s_lin);
            a_log.process_sample_internal(&s_log);
        }
        let p_lin = a_lin.voices()[0].current_pitch();
        let p_log = a_log.voices()[0].current_pitch();
        assert!(p_lin > 62.0 && p_lin < 64.5, "Linear pitch at 25% expected ~63, got {p_lin}");
        assert!(p_log > p_lin + 1.5, "Log glide must lead linear initially: log={p_log}, lin={p_lin}");
    }

    #[test]
    fn drift_microtuning_transpose_and_bender_all_affect_audio() {
        let sr = 48000.0;

        // Microtuning modes produce distinct pitches for E4 (MIDI 64)
        let modes = [
            MicrotuningMode::Equal,
            MicrotuningMode::Just,
            MicrotuningMode::Pythagorean,
            MicrotuningMode::Werckmeister3,
            MicrotuningMode::QuarterTone,
            MicrotuningMode::Maqam,
        ];
        let pitches: Vec<f32> = modes
            .iter()
            .map(|&m| microtuned_midi_note(64, m, None))
            .collect();
        for i in 0..pitches.len() {
            for j in (i + 1)..pitches.len() {
                assert!(
                    (pitches[i] - pitches[j]).abs() > 0.01,
                    "Microtuning {:?} and {:?} matched on E4: {} vs {}",
                    modes[i],
                    modes[j],
                    pitches[i],
                    pitches[j]
                );
            }
        }

        // Drift ON vs OFF produces measurable audio difference
        let mut s_nodrift = VoiceRenderSettings::default();
        s_nodrift.global.drift = false;
        let mut s_drift = s_nodrift;
        s_drift.global.drift = true;

        let mut v0 = VoiceAllocator::new(sr);
        let mut v1 = VoiceAllocator::new(sr);
        v0.note_on(60, 0.8, 0, &s_nodrift);
        v1.note_on(60, 0.8, 0, &s_drift);

        let mut b0 = Vec::with_capacity(4096);
        let mut b1 = Vec::with_capacity(4096);
        for _ in 0..4096 {
            b0.push(v0.process_sample_internal(&s_nodrift));
            b1.push(v1.process_sample_internal(&s_drift));
        }
        assert!(rms_diff(&b0, &b1) > 0.05, "DRIFT must measurably modulate pitch");

        // Bender up/down + Transpose check
        let mut s_bend = s_nodrift;
        s_bend.global.bender_up = 12;
        s_bend.global.pitch_wheel = 1.0;
        let mut vb = VoiceAllocator::new(sr);
        vb.note_on(60, 0.8, 0, &s_bend);
        let mut bb = Vec::with_capacity(4096);
        for _ in 0..4096 {
            bb.push(vb.process_sample_internal(&s_bend));
        }
        assert!(rms_diff(&b0, &bb) > 0.1, "Bender must shift pitch audibly");
    }

    #[test]
    fn sustain_pedal_holds_and_releases_voices_cleanly() {
        let sr = 48000.0;
        let settings = VoiceRenderSettings::default();
        let mut alloc = VoiceAllocator::new(sr);
        alloc.note_on(60, 0.8, 0, &settings);
        alloc.set_sustain_pedal(true);
        let ev = alloc.note_off(60, 0, &settings);
        assert_eq!(ev, VoiceTriggerEvent::None);
        assert!(alloc.voices()[0].is_gate_open());

        let (rel, count) = alloc.set_sustain_pedal(false);
        assert_eq!(count, 1);
        assert_eq!(rel[0], 0);
        assert!(!alloc.voices()[0].is_gate_open());
    }
}
