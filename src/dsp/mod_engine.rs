//! Modulation engine for Phantom Spire:
//! - `Envelope6Stage` (ENV 1..4): 6-stage Spire envelopes (`att`, `dec`, `sus`, `slt`, `sll`, `rel`)
//!   plus 2 direct modulation slots (`target`, `amt`, `vel`).
//! - `LfoEngine` (LFO 1..4): free Hz or tempo-synced rates, 8 waveforms, `morph`, `sym`, `phase`,
//!   `fade`, `unipolar`, `mono`, plus 2 direct modulation slots.
//! - `StepperEngine` (STP 1..2): 16-step tempo-synced/free-rate steppers with `gate`, `ties`,
//!   `smooth` interpolation, `steps_count`, and `loop_mode`.
//! - 15-slot bipolar `Mod Matrix` (`ModSource -> ModTarget`) + `VoiceToGlobalModBridge` so
//!   per-voice sources (e.g. `ENV1 -> FxReverbMix`) smoothly modulate the global FX & Master bus.
//!
//! Real-time safe: zero heap allocations, no panics, finite/denormal-flushed outputs at
//! 44.1 kHz, 48 kHz, 96 kHz, and arbitrary valid sample rates.

use crate::params::{
    EnvParams, LfoParams, LfoWaveform, ModSource, ModTarget, PhantomSpireParams, StepperParams,
    SyncDiv,
};
use crate::SharedSynthState;
use std::f32::consts::{PI, TAU};

// ============================================================================
// Common DSP Utilities
// ============================================================================

/// Flush subnormals and non-finite values to zero.
#[inline]
pub fn flush_denormal(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

/// Sanitize sample rate to a valid positive finite range.
#[inline]
pub fn sanitize_sample_rate(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() && sample_rate >= 8000.0 && sample_rate <= 768_000.0 {
        sample_rate
    } else {
        44100.0
    }
}

/// Sanitize tempo BPM into a safe musical range.
#[inline]
pub fn sanitize_bpm(bpm: f32) -> f32 {
    if bpm.is_finite() {
        bpm.clamp(20.0, 400.0)
    } else {
        120.0
    }
}

/// Convert a `SyncDiv` musical note division to quarter-note beats.
#[inline]
pub fn sync_div_to_beats(div: SyncDiv) -> f32 {
    match div {
        SyncDiv::D1_64 => 4.0 / 64.0,
        SyncDiv::D1_32 => 4.0 / 32.0,
        SyncDiv::D1_16T => (4.0 / 16.0) * (2.0 / 3.0),
        SyncDiv::D1_16 => 0.25,
        SyncDiv::D1_16D => 0.375,
        SyncDiv::D1_8T => (4.0 / 8.0) * (2.0 / 3.0),
        SyncDiv::D1_8 => 0.5,
        SyncDiv::D1_8D => 0.75,
        SyncDiv::D1_4T => 2.0 / 3.0,
        SyncDiv::D1_4 => 1.0,
        SyncDiv::D1_4D => 1.5,
        SyncDiv::D1_2T => 4.0 / 3.0,
        SyncDiv::D1_2 => 2.0,
        SyncDiv::D1_2D => 3.0,
        SyncDiv::D1_1 => 4.0,
        SyncDiv::D2_1 => 8.0,
        SyncDiv::D4_1 => 16.0,
        SyncDiv::D8_1 => 32.0,
    }
}

/// Convert a `SyncDiv` and tempo in BPM to period duration in seconds.
#[inline]
pub fn sync_div_to_seconds(div: SyncDiv, bpm: f32) -> f32 {
    let beats = sync_div_to_beats(div);
    beats * (60.0 / sanitize_bpm(bpm))
}

/// Convert a `SyncDiv` and tempo in BPM to frequency in Hz.
#[inline]
pub fn sync_div_to_hz(div: SyncDiv, bpm: f32) -> f32 {
    let period = sync_div_to_seconds(div, bpm);
    if period > 1e-6 {
        1.0 / period
    } else {
        1.0
    }
}

/// Deterministic fast xorshift32 PRNG returning `[-1.0, 1.0]`.
#[inline]
pub fn next_random_bipolar(state: &mut u32) -> f32 {
    let mut x = *state;
    if x == 0 {
        x = 0x6D2B_79F5;
    }
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    // Map upper 24 bits to [-1.0, 1.0]
    let norm = ((x >> 8) as f32) * (1.0 / 16_777_215.0);
    norm * 2.0 - 1.0
}

/// Map a persisted `u8` index (from `ExtraSynthState` / `SharedSynthState`) to `ModSource`.
#[inline]
pub fn mod_source_from_u8(v: u8) -> ModSource {
    match v {
        1 => ModSource::Env1,
        2 => ModSource::Env2,
        3 => ModSource::Env3,
        4 => ModSource::Env4,
        5 => ModSource::Lfo1,
        6 => ModSource::Lfo2,
        7 => ModSource::Lfo3,
        8 => ModSource::Lfo4,
        9 => ModSource::Stp1,
        10 => ModSource::Stp2,
        11 => ModSource::Macro1,
        12 => ModSource::Macro2,
        13 => ModSource::Macro3,
        14 => ModSource::Macro4,
        15 => ModSource::Velocity,
        16 => ModSource::Keytrack,
        17 => ModSource::ModWheel,
        18 => ModSource::PitchBend,
        19 => ModSource::Aftertouch,
        20 => ModSource::Random,
        _ => ModSource::Off,
    }
}

/// Map a persisted `u8` index (from `ExtraSynthState` / `SharedSynthState`) to `ModTarget`.
#[inline]
pub fn mod_target_from_u8(v: u8) -> ModTarget {
    match v {
        1 => ModTarget::Osc1Pitch,
        2 => ModTarget::Osc2Pitch,
        3 => ModTarget::Osc3Pitch,
        4 => ModTarget::Osc4Pitch,
        5 => ModTarget::AllOscPitch,
        6 => ModTarget::Osc1CtrlA,
        7 => ModTarget::Osc2CtrlA,
        8 => ModTarget::Osc3CtrlA,
        9 => ModTarget::Osc4CtrlA,
        10 => ModTarget::Osc1CtrlB,
        11 => ModTarget::Osc2CtrlB,
        12 => ModTarget::Osc3CtrlB,
        13 => ModTarget::Osc4CtrlB,
        14 => ModTarget::Osc1WtMix,
        15 => ModTarget::Osc2WtMix,
        16 => ModTarget::Osc3WtMix,
        17 => ModTarget::Osc4WtMix,
        18 => ModTarget::Osc1Level,
        19 => ModTarget::Osc2Level,
        20 => ModTarget::Osc3Level,
        21 => ModTarget::Osc4Level,
        22 => ModTarget::Osc1Pan,
        23 => ModTarget::Osc2Pan,
        24 => ModTarget::Osc3Pan,
        25 => ModTarget::Osc4Pan,
        26 => ModTarget::Osc1Detune,
        27 => ModTarget::Osc2Detune,
        28 => ModTarget::Osc3Detune,
        29 => ModTarget::Osc4Detune,
        30 => ModTarget::Filter1Cutoff,
        31 => ModTarget::Filter1Res,
        32 => ModTarget::Filter2Cutoff,
        33 => ModTarget::Filter2Res,
        34 => ModTarget::FilterBalance,
        35 => ModTarget::Env1Attack,
        36 => ModTarget::Env1Decay,
        37 => ModTarget::Env2Attack,
        38 => ModTarget::Env2Decay,
        39 => ModTarget::Lfo1Rate,
        40 => ModTarget::Lfo1Amt,
        41 => ModTarget::Lfo2Rate,
        42 => ModTarget::Lfo2Amt,
        43 => ModTarget::FxShaperDrive,
        44 => ModTarget::FxShaperMix,
        45 => ModTarget::FxPhaserFreq,
        46 => ModTarget::FxPhaserMix,
        47 => ModTarget::FxChorusDepth,
        48 => ModTarget::FxChorusMix,
        49 => ModTarget::FxDelayFeedback,
        50 => ModTarget::FxDelayMix,
        51 => ModTarget::FxReverbDecay,
        52 => ModTarget::FxReverbMix,
        53 => ModTarget::MasterVolume,
        54 => ModTarget::Pan,
        _ => ModTarget::Off,
    }
}

/// Returns `true` if `target` applies to the global FX / Master bus rather than a single voice.
#[inline]
pub fn is_global_target(target: ModTarget) -> bool {
    matches!(
        target,
        ModTarget::FxShaperDrive
            | ModTarget::FxShaperMix
            | ModTarget::FxPhaserFreq
            | ModTarget::FxPhaserMix
            | ModTarget::FxChorusDepth
            | ModTarget::FxChorusMix
            | ModTarget::FxDelayFeedback
            | ModTarget::FxDelayMix
            | ModTarget::FxReverbDecay
            | ModTarget::FxReverbMix
            | ModTarget::MasterVolume
            | ModTarget::Pan
    )
}

// ============================================================================
// 6-Stage Envelope (ENV 1..4)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvStage {
    Idle,
    Attack,
    Decay,
    Slope,
    Hold,
    Release,
}

/// Reveal Sound Spire-style 6-stage envelope (`att`, `dec`, `sus`, `slt`, `sll`, `rel`).
///
/// Spire `sll` (Slope Level) convention:
/// - `sll == 0.5`: neutral (holds flat at `sus`, matching default presets and plucks).
/// - `sll < 0.5`: slopes down from `sus` toward `sus * (sll * 2.0)` (down to `0.0` at `sll = 0.0`).
/// - `sll > 0.5`: slopes up from `sus` toward `sus + (1.0 - sus) * ((sll - 0.5) * 2.0)`
///   (up to `1.0` at `sll = 1.0`, matching the rising swell shown in Spire ENV1).
#[derive(Debug, Clone, Copy)]
pub struct Envelope6Stage {
    sample_rate: f32,
    stage: EnvStage,
    level: f32,
    velocity: f32,
    slope_progress: f32,
    slope_start: f32,
}

impl Default for Envelope6Stage {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl Envelope6Stage {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sanitize_sample_rate(sample_rate),
            stage: EnvStage::Idle,
            level: 0.0,
            velocity: 1.0,
            slope_progress: 0.0,
            slope_start: 0.0,
        }
    }

    #[inline]
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitize_sample_rate(sample_rate);
    }

    #[inline]
    pub fn reset(&mut self) {
        self.stage = EnvStage::Idle;
        self.level = 0.0;
        self.velocity = 1.0;
        self.slope_progress = 0.0;
        self.slope_start = 0.0;
    }

    /// Trigger the envelope. Starts from the current `level` to guarantee click-free retriggering.
    /// When `legato_continue` is true and the envelope is already active (not Idle/Release),
    /// keeps the current stage smoothly without re-attacking.
    #[inline]
    pub fn note_on(&mut self, velocity: f32, legato_continue: bool) {
        let vel = if velocity.is_finite() {
            velocity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.velocity = vel;
        if legato_continue && !matches!(self.stage, EnvStage::Idle | EnvStage::Release) {
            return;
        }
        self.stage = EnvStage::Attack;
        self.slope_progress = 0.0;
    }

    /// Begin the release phase from the current level.
    #[inline]
    pub fn note_off(&mut self) {
        if self.stage != EnvStage::Idle {
            self.stage = EnvStage::Release;
        }
    }

    /// Compute the Spire slope destination level from `sustain` and `sll`.
    #[inline]
    pub fn compute_slope_target(sus: f32, sll: f32) -> f32 {
        let sus = sus.clamp(0.0, 1.0);
        let sll = sll.clamp(0.0, 1.0);
        if sll < 0.5 {
            sus * (sll * 2.0)
        } else if sll > 0.5 {
            sus + (1.0 - sus) * ((sll - 0.5) * 2.0)
        } else {
            sus
        }
    }

    #[inline]
    pub fn stage(&self) -> EnvStage {
        self.stage
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.stage != EnvStage::Idle
    }

    #[inline]
    pub fn value(&self) -> f32 {
        self.level
    }

    #[inline]
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Returns envelope value scaled by velocity sensitivity (`vel_amt` in `0.0..=1.0`).
    #[inline]
    pub fn velocity_scaled_value(&self, vel_amt: f32) -> f32 {
        let v = vel_amt.clamp(0.0, 1.0);
        self.level * (1.0 - v + v * self.velocity)
    }

    /// Advance one sample using `EnvParams` and optional bipolar Attack/Decay modulations (`-1..1`).
    #[inline]
    pub fn process_sample(&mut self, params: &EnvParams, att_mod: f32, dec_mod: f32) -> f32 {
        let att_scale = 2.0_f32.powf(att_mod.clamp(-2.0, 2.0) * 2.0);
        let dec_scale = 2.0_f32.powf(dec_mod.clamp(-2.0, 2.0) * 2.0);
        let att = (params.attack.value() * att_scale).clamp(0.0003, 20.0);
        let dec = (params.decay.value() * dec_scale).clamp(0.0005, 25.0);
        let sus = params.sustain.value().clamp(0.0, 1.0);
        let slt = params.slt.value().clamp(0.001, 20.0);
        let sll = params.sll.value().clamp(0.0, 1.0);
        let rel = params.release.value().clamp(0.0005, 25.0);
        self.process_sample_raw(att, dec, sus, slt, sll, rel)
    }

    /// Advance one sample with explicit envelope times (seconds) and levels (`0..1`).
    #[inline]
    pub fn process_sample_raw(
        &mut self,
        att_s: f32,
        dec_s: f32,
        sus: f32,
        slt_s: f32,
        sll: f32,
        rel_s: f32,
    ) -> f32 {
        let sr = self.sample_rate;
        let sus = sus.clamp(0.0, 1.0);
        let slope_target = Self::compute_slope_target(sus, sll);

        match self.stage {
            EnvStage::Idle => {
                self.level = 0.0;
            }
            EnvStage::Attack => {
                let att_samples = (att_s.max(0.0002) * sr).max(1.0);
                // Punchy exponential rise toward 1.0528 so it reaches 1.0 in ~att_s seconds.
                let coeff = 1.0 - (-3.0 / att_samples).exp();
                self.level += (1.0528 - self.level) * coeff;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
            }
            EnvStage::Decay => {
                let dec_samples = (dec_s.max(0.0005) * sr).max(1.0);
                // Exponential decay with time constant dec_s / 4.6 (~99% reached in dec_s).
                let coeff = 1.0 - (-4.6 / dec_samples).exp();
                let diff = self.level - sus;
                if diff.abs() <= 1e-4 {
                    self.level = sus;
                    if (slope_target - sus).abs() <= 1e-4 {
                        self.stage = EnvStage::Hold;
                    } else {
                        self.slope_start = self.level;
                        self.slope_progress = 0.0;
                        self.stage = EnvStage::Slope;
                    }
                } else {
                    self.level -= diff * coeff;
                }
            }
            EnvStage::Slope => {
                let slt_samples = (slt_s.max(0.001) * sr).max(1.0);
                self.slope_progress += 1.0 / slt_samples;
                if self.slope_progress >= 1.0 {
                    self.slope_progress = 1.0;
                    self.level = slope_target;
                    self.stage = EnvStage::Hold;
                } else {
                    // Smooth musical S/exponential curve from slope_start -> slope_target
                    let t = self.slope_progress;
                    let curve = if slope_target >= self.slope_start {
                        // Gentle S-curve swell
                        t * t * (3.0 - 2.0 * t)
                    } else {
                        // Natural exponential droop normalized to [0, 1]
                        (1.0 - (-3.5 * t).exp()) / (1.0 - (-3.5_f32).exp())
                    };
                    self.level = self.slope_start + (slope_target - self.slope_start) * curve;
                }
            }
            EnvStage::Hold => {
                // Smoothly follow live sustain/sll automation without clicks
                let track_coeff = 1.0 - (-1.0 / (0.005 * sr).max(1.0)).exp();
                self.level += (slope_target - self.level) * track_coeff;
            }
            EnvStage::Release => {
                let rel_samples = (rel_s.max(0.0005) * sr).max(1.0);
                let coeff = 1.0 - (-5.0 / rel_samples).exp();
                self.level -= self.level * coeff;
                if self.level <= 1e-4 {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                }
            }
        }

        self.level = flush_denormal(self.level).clamp(0.0, 1.0);
        self.level
    }
}

// ============================================================================
// LFO Engine (LFO 1..4)
// ============================================================================

/// Multi-waveform Spire LFO with free Hz / tempo-sync, morph, symmetry, phase offset,
/// fade-in envelope, unipolar mode, and deterministic S&H / Smooth Random.
#[derive(Debug, Clone, Copy)]
pub struct LfoEngine {
    sample_rate: f32,
    phase: f32,
    fade_level: f32,
    rng_state: u32,
    snh_curr: f32,
    snh_prev: f32,
    last_output: f32,
}

impl Default for LfoEngine {
    fn default() -> Self {
        Self::new(44100.0, 0x1234_5678)
    }
}

impl LfoEngine {
    pub fn new(sample_rate: f32, seed: u32) -> Self {
        let mut rng_state = if seed == 0 { 0x9E37_79B9 } else { seed };
        let snh_prev = next_random_bipolar(&mut rng_state);
        let snh_curr = next_random_bipolar(&mut rng_state);
        Self {
            sample_rate: sanitize_sample_rate(sample_rate),
            phase: 0.0,
            fade_level: 1.0,
            rng_state,
            snh_curr,
            snh_prev,
            last_output: 0.0,
        }
    }

    #[inline]
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitize_sample_rate(sample_rate);
    }

    #[inline]
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.fade_level = 1.0;
        self.last_output = 0.0;
    }

    /// Retrigger LFO phase and fade-in on Note On.
    #[inline]
    pub fn trigger(&mut self, phase_offset: f32, fade_s: f32) {
        self.phase = phase_offset.rem_euclid(1.0);
        self.fade_level = if fade_s > 0.001 { 0.0 } else { 1.0 };
        self.snh_prev = self.snh_curr;
        self.snh_curr = next_random_bipolar(&mut self.rng_state);
    }

    #[inline]
    pub fn current_phase(&self) -> f32 {
        self.phase
    }

    #[inline]
    pub fn last_value(&self) -> f32 {
        self.last_output
    }

    /// Warp phase `[0, 1)` smoothly around pivot `sym` in `[0.05, 0.95]`.
    #[inline]
    fn warp_phase(p: f32, sym: f32) -> f32 {
        let s = sym.clamp(0.05, 0.95);
        if p < s {
            0.5 * (p / s)
        } else {
            0.5 + 0.5 * ((p - s) / (1.0 - s))
        }
    }

    /// Evaluate the LFO waveform at normalized phase `p` in `[0, 1)`.
    #[inline]
    fn eval_wave(&self, waveform: LfoWaveform, p: f32, sym: f32, morph: f32) -> f32 {
        let sym = sym.clamp(0.05, 0.95);
        let morph = morph.clamp(0.0, 1.0);

        let raw = match waveform {
            LfoWaveform::Sin => {
                let wp = Self::warp_phase(p, sym);
                let sin_v = (wp * TAU).sin();
                // Morph adds warm odd harmonics / soft square character
                let shaped = (sin_v * (1.0 + morph * 3.5)).tanh() / (1.0 + morph * 3.5).tanh();
                sin_v * (1.0 - morph) + shaped * morph
            }
            LfoWaveform::Tri => {
                // Skewed triangle: rises 0..sym, falls sym..1
                let tri = if p < sym {
                    -1.0 + 2.0 * (p / sym)
                } else {
                    1.0 - 2.0 * ((p - sym) / (1.0 - sym))
                };
                // Morph blends triangle toward sine-rounded / clipped wave
                let curved = (tri * PI * 0.5).sin();
                tri * (1.0 - morph) + curved * morph
            }
            LfoWaveform::SawUp => {
                // Symmetry controls curvature exponent (0.5 = linear)
                let exp = 2.0_f32.powf((0.5 - sym) * 3.0);
                let saw = p.powf(exp) * 2.0 - 1.0;
                let sq = if p < 0.5 { -1.0 } else { 1.0 };
                saw * (1.0 - morph) + sq * morph
            }
            LfoWaveform::SawDn => {
                let exp = 2.0_f32.powf((0.5 - sym) * 3.0);
                let saw = (1.0 - p).powf(exp) * 2.0 - 1.0;
                let sq = if p < 0.5 { 1.0 } else { -1.0 };
                saw * (1.0 - morph) + sq * morph
            }
            LfoWaveform::Square => {
                let sq = if p < sym { 1.0 } else { -1.0 };
                // Morph softens edges toward warped sine
                let wp = Self::warp_phase(p, sym);
                let soft = (wp * TAU).sin();
                sq * (1.0 - morph) + soft * morph
            }
            LfoWaveform::Snh => {
                // Morph interpolates between stepped S&H and smooth cosine transition
                let smooth_t = 0.5 - 0.5 * (p * PI).cos();
                let smooth = self.snh_prev + (self.snh_curr - self.snh_prev) * smooth_t;
                self.snh_curr * (1.0 - morph) + smooth * morph
            }
            LfoWaveform::SmoothRnd => {
                let wp = Self::warp_phase(p, sym);
                let smooth_t = wp * wp * (3.0 - 2.0 * wp);
                let smooth = self.snh_prev + (self.snh_curr - self.snh_prev) * smooth_t;
                // Morph blends toward stepped S&H
                smooth * (1.0 - morph) + self.snh_curr * morph
            }
            LfoWaveform::Ramp => {
                // Exponential rise then fast decay (peak at sym)
                let base = if p < sym {
                    let t = p / sym;
                    (t * t) * 2.0 - 1.0
                } else {
                    let t = (p - sym) / (1.0 - sym);
                    (1.0 - t) * (1.0 - t) * 2.0 - 1.0
                };
                // Morph pushes toward sharp exponential impulse
                let sharp = (base * 0.5 + 0.5).powf(1.0 + morph * 3.0) * 2.0 - 1.0;
                base * (1.0 - morph) + sharp * morph
            }
        };
        flush_denormal(raw).clamp(-1.0, 1.0)
    }

    /// Process one sample using `LfoParams`, tempo BPM, and optional rate/amt modulations.
    #[inline]
    pub fn process_sample(
        &mut self,
        params: &LfoParams,
        bpm: f32,
        rate_mod: f32,
        amt_mod: f32,
    ) -> f32 {
        let base_hz = if params.sync.value() {
            sync_div_to_hz(params.sync_div.value(), bpm)
        } else {
            params.rate.value()
        };
        let eff_hz = (base_hz * 2.0_f32.powf(rate_mod.clamp(-2.0, 2.0) * 2.5)).clamp(0.005, 100.0);
        let amp = (1.0 + amt_mod).clamp(0.0, 2.0);
        self.process_sample_raw(
            eff_hz,
            params.waveform.value(),
            params.morph.value(),
            params.sym.value(),
            params.phase.value(),
            params.fade.value(),
            params.unipolar.value(),
            amp,
        )
    }

    /// Process one sample with explicit raw parameters (ideal for DSP tests).
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn process_sample_raw(
        &mut self,
        freq_hz: f32,
        waveform: LfoWaveform,
        morph: f32,
        sym: f32,
        phase_offset: f32,
        fade_s: f32,
        unipolar: bool,
        amp_scale: f32,
    ) -> f32 {
        let sr = self.sample_rate;

        // Fade-in ramp
        if fade_s > 0.001 && self.fade_level < 1.0 {
            self.fade_level = (self.fade_level + 1.0 / (fade_s * sr).max(1.0)).min(1.0);
        } else {
            self.fade_level = 1.0;
        }

        let eval_p = (self.phase + phase_offset).rem_euclid(1.0);
        let mut out = self.eval_wave(waveform, eval_p, sym, morph);

        if unipolar {
            out = (out * 0.5 + 0.5).clamp(0.0, 1.0);
        }

        out = flush_denormal(out * self.fade_level * amp_scale.clamp(0.0, 2.0)).clamp(-2.0, 2.0);
        self.last_output = out;

        // Advance phase
        let inc = (freq_hz.clamp(0.001, 200.0)) / sr;
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase = self.phase.fract();
            self.snh_prev = self.snh_curr;
            self.snh_curr = next_random_bipolar(&mut self.rng_state);
        }

        out
    }
}

// ============================================================================
// Stepper Engine (STP 1..2)
// ============================================================================

/// 16-step tempo-synced or free-rate stepper modulator with gate, ties, and smooth interpolation.
///
/// Step values in `SharedSynthState` are stored in `[0.0, 1.0]` with `0.5` neutral,
/// mapped here to bipolar `[-1.0, 1.0]` (`(val - 0.5) * 2.0`) so Init (`0.5`) contributes `0.0`.
#[derive(Debug, Clone, Copy)]
pub struct StepperEngine {
    sample_rate: f32,
    step_index: usize,
    step_phase: f32,
    smoothed_output: f32,
    finished_oneshot: bool,
}

impl Default for StepperEngine {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl StepperEngine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sanitize_sample_rate(sample_rate),
            step_index: 0,
            step_phase: 0.0,
            smoothed_output: 0.0,
            finished_oneshot: false,
        }
    }

    #[inline]
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitize_sample_rate(sample_rate);
    }

    #[inline]
    pub fn reset(&mut self) {
        self.step_index = 0;
        self.step_phase = 0.0;
        self.smoothed_output = 0.0;
        self.finished_oneshot = false;
    }

    #[inline]
    pub fn retrigger(&mut self) {
        self.step_index = 0;
        self.step_phase = 0.0;
        self.finished_oneshot = false;
    }

    #[inline]
    pub fn current_step(&self) -> u8 {
        (self.step_index & 0x0F) as u8
    }

    #[inline]
    pub fn value(&self) -> f32 {
        self.smoothed_output
    }

    /// Process one sample reading pattern from `SharedSynthState`.
    #[inline]
    pub fn process_sample(
        &mut self,
        stepper_idx: usize,
        params: &StepperParams,
        shared: &SharedSynthState,
        bpm: f32,
    ) -> f32 {
        let mut steps = [0.5_f32; 16];
        for (i, slot) in steps.iter_mut().enumerate() {
            *slot = shared.read_stepper_step(stepper_idx, i);
        }
        let ties = if stepper_idx < 2 {
            shared.stepper_ties[stepper_idx].load(std::sync::atomic::Ordering::Relaxed)
        } else {
            0
        };

        let step_hz = if params.sync.value() {
            sync_div_to_hz(params.sync_div.value(), bpm)
        } else {
            params.rate.value()
        };

        self.process_sample_raw(
            step_hz,
            params.gate.value(),
            params.smooth.value(),
            params.steps_count.value().clamp(2, 16) as usize,
            params.loop_mode.value(),
            &steps,
            ties,
        )
    }

    /// Process one sample with explicit raw step array `[0..1; 16]` and `ties` bitmask.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn process_sample_raw(
        &mut self,
        step_hz: f32,
        gate: f32,
        smooth: f32,
        steps_count: usize,
        loop_mode: bool,
        steps: &[f32; 16],
        ties: u16,
    ) -> f32 {
        let count = steps_count.clamp(2, 16);
        if self.step_index >= count {
            self.step_index %= count;
        }
        let gate = gate.clamp(0.05, 1.0);
        let smooth = smooth.clamp(0.0, 1.0);
        let step_hz = step_hz.clamp(0.05, 100.0);

        let is_tied = (ties & (1u16 << self.step_index)) != 0;
        let step_val = steps[self.step_index].clamp(0.0, 1.0);
        let bipolar_val = (step_val - 0.5) * 2.0;

        let target = if is_tied || self.step_phase < gate {
            bipolar_val
        } else {
            0.0
        };

        // Smooth interpolation: 0.0 -> crisp 1ms anti-click; 1.0 -> full step-length glide
        let step_period_s = 1.0 / step_hz;
        let smooth_time_s = 0.001 + smooth * smooth * (step_period_s * 0.85);
        let coeff = 1.0 - (-1.0 / (smooth_time_s * self.sample_rate).max(1.0)).exp();
        self.smoothed_output += (target - self.smoothed_output) * coeff;
        self.smoothed_output = flush_denormal(self.smoothed_output).clamp(-1.0, 1.0);

        // Advance step clock
        if !self.finished_oneshot {
            self.step_phase += step_hz / self.sample_rate;
            if self.step_phase >= 1.0 {
                self.step_phase -= 1.0;
                if self.step_index + 1 < count {
                    self.step_index += 1;
                } else if loop_mode {
                    self.step_index = 0;
                } else {
                    self.finished_oneshot = true;
                    self.step_phase = 1.0;
                }
            }
        }

        self.smoothed_output
    }
}

// ============================================================================
// Modulation Target Offset Structs (Shared Contract with @dsp-core & @foundation)
// ============================================================================

/// Per-voice modulation offsets computed every sample by `VoiceModState`.
///
/// All offsets are bipolar normalized values (typically in `[-1.0, 1.0]`, clamped safely to
/// `[-2.0, 2.0]`) except where noted:
/// - `osc_pitch[i]`: normalized (`1.0` = `+24.0` semitones; helper `osc_pitch_semitones(i)`).
/// - `filter_cutoff[i]`: normalized (`1.0` = `+6.0` octaves; helper `apply_cutoff_mod(hz, mod)`).
/// - `osc_ctrl_a/b`, `osc_wt_mix`, `osc_level`, `osc_pan`, `osc_detune`, `filter_res`,
///   `filter_balance`, `env_attack`, `env_decay`, `lfo_rate`, `lfo_amt`, `pan`: additive normalized.
#[derive(Debug, Clone, Copy, Default)]
pub struct VoiceModOffsets {
    pub osc_pitch: [f32; 4],
    pub osc_ctrl_a: [f32; 4],
    pub osc_ctrl_b: [f32; 4],
    pub osc_wt_mix: [f32; 4],
    pub osc_level: [f32; 4],
    pub osc_pan: [f32; 4],
    pub osc_detune: [f32; 4],
    pub filter_cutoff: [f32; 2],
    pub filter_res: [f32; 2],
    pub filter_balance: f32,
    pub env_attack: [f32; 2],
    pub env_decay: [f32; 2],
    pub lfo_rate: [f32; 2],
    pub lfo_amt: [f32; 2],
    pub pan: f32,
}

impl VoiceModOffsets {
    #[inline]
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Pitch offset for oscillator `idx` in semitones (`±24` st per unit modulation).
    #[inline]
    pub fn osc_pitch_semitones(&self, idx: usize) -> f32 {
        if idx < 4 {
            self.osc_pitch[idx].clamp(-2.0, 2.0) * 24.0
        } else {
            0.0
        }
    }

    /// Apply filter cutoff modulation (`±6` octaves per unit modulation) to `base_hz`.
    #[inline]
    pub fn apply_cutoff_hz(base_hz: f32, cutoff_mod: f32) -> f32 {
        let octaves = cutoff_mod.clamp(-2.0, 2.0) * 6.0;
        (base_hz * 2.0_f32.powf(octaves)).clamp(20.0, 20000.0)
    }

    /// Convert normalized `VoiceModOffsets` + amplitude envelope gain into `voice::VoiceModulation`
    /// with musical scaling (`±24` semitones pitch, `±6` octaves filter cutoff).
    #[inline]
    pub fn to_voice_modulation(&self, amp_env: f32) -> crate::dsp::voice::VoiceModulation {
        crate::dsp::voice::VoiceModulation {
            amp_env: flush_denormal(amp_env).clamp(0.0, 2.0),
            all_osc_pitch_st: 0.0,
            osc_pitch_st: [
                self.osc_pitch[0].clamp(-2.0, 2.0) * 24.0,
                self.osc_pitch[1].clamp(-2.0, 2.0) * 24.0,
                self.osc_pitch[2].clamp(-2.0, 2.0) * 24.0,
                self.osc_pitch[3].clamp(-2.0, 2.0) * 24.0,
            ],
            osc_ctrl_a: self.osc_ctrl_a,
            osc_ctrl_b: self.osc_ctrl_b,
            osc_wt_mix: self.osc_wt_mix,
            osc_level: self.osc_level,
            osc_pan: self.osc_pan,
            osc_detune: self.osc_detune,
            filter_cutoff_oct: [
                self.filter_cutoff[0].clamp(-2.0, 2.0) * 6.0,
                self.filter_cutoff[1].clamp(-2.0, 2.0) * 6.0,
            ],
            filter_res: self.filter_res,
            filter_balance: self.filter_balance,
            voice_pan: self.pan,
        }
    }

    #[inline]
    pub fn add_target(&mut self, target: ModTarget, delta: f32) {
        if !delta.is_finite() || delta == 0.0 {
            return;
        }
        match target {
            ModTarget::Osc1Pitch => self.osc_pitch[0] += delta,
            ModTarget::Osc2Pitch => self.osc_pitch[1] += delta,
            ModTarget::Osc3Pitch => self.osc_pitch[2] += delta,
            ModTarget::Osc4Pitch => self.osc_pitch[3] += delta,
            ModTarget::AllOscPitch => {
                for p in &mut self.osc_pitch {
                    *p += delta;
                }
            }
            ModTarget::Osc1CtrlA => self.osc_ctrl_a[0] += delta,
            ModTarget::Osc2CtrlA => self.osc_ctrl_a[1] += delta,
            ModTarget::Osc3CtrlA => self.osc_ctrl_a[2] += delta,
            ModTarget::Osc4CtrlA => self.osc_ctrl_a[3] += delta,
            ModTarget::Osc1CtrlB => self.osc_ctrl_b[0] += delta,
            ModTarget::Osc2CtrlB => self.osc_ctrl_b[1] += delta,
            ModTarget::Osc3CtrlB => self.osc_ctrl_b[2] += delta,
            ModTarget::Osc4CtrlB => self.osc_ctrl_b[3] += delta,
            ModTarget::Osc1WtMix => self.osc_wt_mix[0] += delta,
            ModTarget::Osc2WtMix => self.osc_wt_mix[1] += delta,
            ModTarget::Osc3WtMix => self.osc_wt_mix[2] += delta,
            ModTarget::Osc4WtMix => self.osc_wt_mix[3] += delta,
            ModTarget::Osc1Level => self.osc_level[0] += delta,
            ModTarget::Osc2Level => self.osc_level[1] += delta,
            ModTarget::Osc3Level => self.osc_level[2] += delta,
            ModTarget::Osc4Level => self.osc_level[3] += delta,
            ModTarget::Osc1Pan => self.osc_pan[0] += delta,
            ModTarget::Osc2Pan => self.osc_pan[1] += delta,
            ModTarget::Osc3Pan => self.osc_pan[2] += delta,
            ModTarget::Osc4Pan => self.osc_pan[3] += delta,
            ModTarget::Osc1Detune => self.osc_detune[0] += delta,
            ModTarget::Osc2Detune => self.osc_detune[1] += delta,
            ModTarget::Osc3Detune => self.osc_detune[2] += delta,
            ModTarget::Osc4Detune => self.osc_detune[3] += delta,
            ModTarget::Filter1Cutoff => self.filter_cutoff[0] += delta,
            ModTarget::Filter1Res => self.filter_res[0] += delta,
            ModTarget::Filter2Cutoff => self.filter_cutoff[1] += delta,
            ModTarget::Filter2Res => self.filter_res[1] += delta,
            ModTarget::FilterBalance => self.filter_balance += delta,
            ModTarget::Env1Attack => self.env_attack[0] += delta,
            ModTarget::Env1Decay => self.env_decay[0] += delta,
            ModTarget::Env2Attack => self.env_attack[1] += delta,
            ModTarget::Env2Decay => self.env_decay[1] += delta,
            ModTarget::Lfo1Rate => self.lfo_rate[0] += delta,
            ModTarget::Lfo1Amt => self.lfo_amt[0] += delta,
            ModTarget::Lfo2Rate => self.lfo_rate[1] += delta,
            ModTarget::Lfo2Amt => self.lfo_amt[1] += delta,
            ModTarget::Pan => self.pan += delta,
            _ => {}
        }
    }

    #[inline]
    pub fn clamp_safe(&mut self) {
        for v in self
            .osc_pitch
            .iter_mut()
            .chain(self.osc_ctrl_a.iter_mut())
            .chain(self.osc_ctrl_b.iter_mut())
            .chain(self.osc_wt_mix.iter_mut())
            .chain(self.osc_level.iter_mut())
            .chain(self.osc_pan.iter_mut())
            .chain(self.osc_detune.iter_mut())
            .chain(self.filter_cutoff.iter_mut())
            .chain(self.filter_res.iter_mut())
            .chain(self.env_attack.iter_mut())
            .chain(self.env_decay.iter_mut())
            .chain(self.lfo_rate.iter_mut())
            .chain(self.lfo_amt.iter_mut())
        {
            *v = flush_denormal(*v).clamp(-2.0, 2.0);
        }
        self.filter_balance = flush_denormal(self.filter_balance).clamp(-2.0, 2.0);
        self.pan = flush_denormal(self.pan).clamp(-1.0, 1.0);
    }
}

/// Global FX & Master modulation offsets produced by `VoiceToGlobalModBridge` + `GlobalModState`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalModOffsets {
    pub fx_shaper_drive: f32,
    pub fx_shaper_mix: f32,
    pub fx_phaser_freq: f32,
    pub fx_phaser_mix: f32,
    pub fx_chorus_depth: f32,
    pub fx_chorus_mix: f32,
    pub fx_delay_feedback: f32,
    pub fx_delay_mix: f32,
    pub fx_reverb_decay: f32,
    pub fx_reverb_mix: f32,
    pub master_volume: f32,
    pub global_pan: f32,
}

impl GlobalModOffsets {
    #[inline]
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    #[inline]
    pub fn add_target(&mut self, target: ModTarget, delta: f32) {
        if !delta.is_finite() || delta == 0.0 {
            return;
        }
        match target {
            ModTarget::FxShaperDrive => self.fx_shaper_drive += delta,
            ModTarget::FxShaperMix => self.fx_shaper_mix += delta,
            ModTarget::FxPhaserFreq => self.fx_phaser_freq += delta,
            ModTarget::FxPhaserMix => self.fx_phaser_mix += delta,
            ModTarget::FxChorusDepth => self.fx_chorus_depth += delta,
            ModTarget::FxChorusMix => self.fx_chorus_mix += delta,
            ModTarget::FxDelayFeedback => self.fx_delay_feedback += delta,
            ModTarget::FxDelayMix => self.fx_delay_mix += delta,
            ModTarget::FxReverbDecay => self.fx_reverb_decay += delta,
            ModTarget::FxReverbMix => self.fx_reverb_mix += delta,
            ModTarget::MasterVolume => self.master_volume += delta,
            ModTarget::Pan => self.global_pan += delta,
            _ => {}
        }
    }

    /// Merge another `GlobalModOffsets` using max-magnitude signed reduction (preserves 1.0x
    /// full depth for 1 voice and prevents multi-voice chord blow-up).
    #[inline]
    pub fn merge_max_abs(&mut self, other: &GlobalModOffsets) {
        #[inline]
        fn max_abs(a: f32, b: f32) -> f32 {
            if b.abs() > a.abs() {
                b
            } else {
                a
            }
        }
        self.fx_shaper_drive = max_abs(self.fx_shaper_drive, other.fx_shaper_drive);
        self.fx_shaper_mix = max_abs(self.fx_shaper_mix, other.fx_shaper_mix);
        self.fx_phaser_freq = max_abs(self.fx_phaser_freq, other.fx_phaser_freq);
        self.fx_phaser_mix = max_abs(self.fx_phaser_mix, other.fx_phaser_mix);
        self.fx_chorus_depth = max_abs(self.fx_chorus_depth, other.fx_chorus_depth);
        self.fx_chorus_mix = max_abs(self.fx_chorus_mix, other.fx_chorus_mix);
        self.fx_delay_feedback = max_abs(self.fx_delay_feedback, other.fx_delay_feedback);
        self.fx_delay_mix = max_abs(self.fx_delay_mix, other.fx_delay_mix);
        self.fx_reverb_decay = max_abs(self.fx_reverb_decay, other.fx_reverb_decay);
        self.fx_reverb_mix = max_abs(self.fx_reverb_mix, other.fx_reverb_mix);
        self.master_volume = max_abs(self.master_volume, other.master_volume);
        self.global_pan = max_abs(self.global_pan, other.global_pan);
    }

    #[inline]
    pub fn add_assign(&mut self, other: &GlobalModOffsets) {
        self.fx_shaper_drive += other.fx_shaper_drive;
        self.fx_shaper_mix += other.fx_shaper_mix;
        self.fx_phaser_freq += other.fx_phaser_freq;
        self.fx_phaser_mix += other.fx_phaser_mix;
        self.fx_chorus_depth += other.fx_chorus_depth;
        self.fx_chorus_mix += other.fx_chorus_mix;
        self.fx_delay_feedback += other.fx_delay_feedback;
        self.fx_delay_mix += other.fx_delay_mix;
        self.fx_reverb_decay += other.fx_reverb_decay;
        self.fx_reverb_mix += other.fx_reverb_mix;
        self.master_volume += other.master_volume;
        self.global_pan += other.global_pan;
    }

    #[inline]
    pub fn clamp_safe(&mut self) {
        self.fx_shaper_drive = flush_denormal(self.fx_shaper_drive).clamp(-1.0, 1.0);
        self.fx_shaper_mix = flush_denormal(self.fx_shaper_mix).clamp(-1.0, 1.0);
        self.fx_phaser_freq = flush_denormal(self.fx_phaser_freq).clamp(-1.0, 1.0);
        self.fx_phaser_mix = flush_denormal(self.fx_phaser_mix).clamp(-1.0, 1.0);
        self.fx_chorus_depth = flush_denormal(self.fx_chorus_depth).clamp(-1.0, 1.0);
        self.fx_chorus_mix = flush_denormal(self.fx_chorus_mix).clamp(-1.0, 1.0);
        self.fx_delay_feedback = flush_denormal(self.fx_delay_feedback).clamp(-1.0, 1.0);
        self.fx_delay_mix = flush_denormal(self.fx_delay_mix).clamp(-1.0, 1.0);
        self.fx_reverb_decay = flush_denormal(self.fx_reverb_decay).clamp(-1.0, 1.0);
        self.fx_reverb_mix = flush_denormal(self.fx_reverb_mix).clamp(-1.0, 1.0);
        self.master_volume = flush_denormal(self.master_volume).clamp(-1.0, 1.0);
        self.global_pan = flush_denormal(self.global_pan).clamp(-1.0, 1.0);
    }

    #[inline]
    pub fn smooth_towards(&mut self, target: &GlobalModOffsets, coeff: f32) {
        let c = coeff.clamp(0.0, 1.0);
        self.fx_shaper_drive =
            flush_denormal(self.fx_shaper_drive + (target.fx_shaper_drive - self.fx_shaper_drive) * c);
        self.fx_shaper_mix =
            flush_denormal(self.fx_shaper_mix + (target.fx_shaper_mix - self.fx_shaper_mix) * c);
        self.fx_phaser_freq =
            flush_denormal(self.fx_phaser_freq + (target.fx_phaser_freq - self.fx_phaser_freq) * c);
        self.fx_phaser_mix =
            flush_denormal(self.fx_phaser_mix + (target.fx_phaser_mix - self.fx_phaser_mix) * c);
        self.fx_chorus_depth =
            flush_denormal(self.fx_chorus_depth + (target.fx_chorus_depth - self.fx_chorus_depth) * c);
        self.fx_chorus_mix =
            flush_denormal(self.fx_chorus_mix + (target.fx_chorus_mix - self.fx_chorus_mix) * c);
        self.fx_delay_feedback = flush_denormal(
            self.fx_delay_feedback + (target.fx_delay_feedback - self.fx_delay_feedback) * c,
        );
        self.fx_delay_mix =
            flush_denormal(self.fx_delay_mix + (target.fx_delay_mix - self.fx_delay_mix) * c);
        self.fx_reverb_decay =
            flush_denormal(self.fx_reverb_decay + (target.fx_reverb_decay - self.fx_reverb_decay) * c);
        self.fx_reverb_mix =
            flush_denormal(self.fx_reverb_mix + (target.fx_reverb_mix - self.fx_reverb_mix) * c);
        self.master_volume =
            flush_denormal(self.master_volume + (target.master_volume - self.master_volume) * c);
        self.global_pan =
            flush_denormal(self.global_pan + (target.global_pan - self.global_pan) * c);
    }
}

// ============================================================================
// Voice-to-Global FX Modulation Reduction Bridge
// ============================================================================

/// Lock-free, zipper-free reduction bridge aggregating per-voice global target modulations
/// (such as `ENV1 -> FX Reverb Mix`, like Spire Pic 1) with global-only sources (Macros,
/// Mod Wheel, Pitch Bend, Aftertouch, Mono LFOs, Steppers).
#[derive(Debug, Clone, Copy)]
pub struct VoiceToGlobalModBridge {
    sample_rate: f32,
    voice_acc: GlobalModOffsets,
    smoothed: GlobalModOffsets,
    has_voice_contrib: bool,
}

impl Default for VoiceToGlobalModBridge {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl VoiceToGlobalModBridge {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sanitize_sample_rate(sample_rate),
            voice_acc: GlobalModOffsets::default(),
            smoothed: GlobalModOffsets::default(),
            has_voice_contrib: false,
        }
    }

    #[inline]
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitize_sample_rate(sample_rate);
    }

    #[inline]
    pub fn reset(&mut self) {
        self.voice_acc.clear();
        self.smoothed.clear();
        self.has_voice_contrib = false;
    }

    /// Call once per audio sample before iterating active voices.
    #[inline]
    pub fn begin_sample(&mut self) {
        self.voice_acc.clear();
        self.has_voice_contrib = false;
    }

    /// Accumulate a single active voice's global-target contributions.
    #[inline]
    pub fn accumulate_voice(&mut self, voice_global: &GlobalModOffsets) {
        if !self.has_voice_contrib {
            self.voice_acc = *voice_global;
            self.has_voice_contrib = true;
        } else {
            self.voice_acc.merge_max_abs(voice_global);
        }
    }

    /// Finalize the sample: combine reduced per-voice offsets with global-source offsets
    /// and apply a smooth ~4 ms de-zipper filter so voice stealing/release never clicks FX.
    #[inline]
    pub fn finish_sample(&mut self, global_only: &GlobalModOffsets) -> GlobalModOffsets {
        let mut combined = *global_only;
        if self.has_voice_contrib {
            combined.add_assign(&self.voice_acc);
        }
        combined.clamp_safe();

        let smooth_coeff = 1.0 - (-1.0 / (0.004 * self.sample_rate).max(1.0)).exp();
        self.smoothed.smooth_towards(&combined, smooth_coeff);
        self.smoothed.clamp_safe();
        self.smoothed
    }

    #[inline]
    pub fn current(&self) -> GlobalModOffsets {
        self.smoothed
    }
}

// ============================================================================
// Global Modulator State (Mono LFOs, Steppers, Controllers, Bridge)
// ============================================================================

/// Snapshot of global modulator outputs for the current sample, passed cheaply to each voice.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalModFrame {
    pub mono_lfo: [f32; 4],
    pub stp: [f32; 2],
    pub macros: [f32; 4],
    pub pitch_bend: f32,
    pub mod_wheel: f32,
    pub aftertouch: f32,
}

/// Global modulation controller owned by `SynthEngine`.
pub struct GlobalModState {
    sample_rate: f32,
    pub mono_lfos: [LfoEngine; 4],
    pub steppers: [StepperEngine; 2],
    pub bridge: VoiceToGlobalModBridge,
    pub pitch_bend: f32,
    pub mod_wheel: f32,
    pub aftertouch: f32,
    last_frame: GlobalModFrame,
    last_global_only: GlobalModOffsets,
}

impl Default for GlobalModState {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl GlobalModState {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        Self {
            sample_rate: sr,
            mono_lfos: [
                LfoEngine::new(sr, 0xA1B2_C3D4),
                LfoEngine::new(sr, 0xB2C3_D4E5),
                LfoEngine::new(sr, 0xC3D4_E5F6),
                LfoEngine::new(sr, 0xD4E5_F607),
            ],
            steppers: [StepperEngine::new(sr), StepperEngine::new(sr)],
            bridge: VoiceToGlobalModBridge::new(sr),
            pitch_bend: 0.0,
            mod_wheel: 0.0,
            aftertouch: 0.0,
            last_frame: GlobalModFrame::default(),
            last_global_only: GlobalModOffsets::default(),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sanitize_sample_rate(sample_rate);
        self.sample_rate = sr;
        for lfo in &mut self.mono_lfos {
            lfo.set_sample_rate(sr);
        }
        for stp in &mut self.steppers {
            stp.set_sample_rate(sr);
        }
        self.bridge.set_sample_rate(sr);
    }

    pub fn reset(&mut self) {
        for lfo in &mut self.mono_lfos {
            lfo.reset();
        }
        for stp in &mut self.steppers {
            stp.reset();
        }
        self.bridge.reset();
        self.pitch_bend = 0.0;
        self.mod_wheel = 0.0;
        self.aftertouch = 0.0;
        self.last_frame = GlobalModFrame::default();
        self.last_global_only = GlobalModOffsets::default();
    }

    /// Retrigger global steppers (and mono LFO fade-in) when starting from silence.
    #[inline]
    pub fn on_first_note_on(&mut self, params: &PhantomSpireParams) {
        for (i, lfo) in self.mono_lfos.iter_mut().enumerate() {
            if params.lfos[i].mono.value() {
                lfo.trigger(params.lfos[i].phase.value(), params.lfos[i].fade.value());
            }
        }
        for stp in &mut self.steppers {
            stp.retrigger();
        }
    }

    /// Step global modulators once per sample and prepare the voice-to-global bridge.
    #[inline]
    pub fn begin_sample(
        &mut self,
        params: &PhantomSpireParams,
        shared: &SharedSynthState,
        bpm: f32,
    ) -> GlobalModFrame {
        self.bridge.begin_sample();

        let mut frame = GlobalModFrame {
            mono_lfo: [0.0; 4],
            stp: [0.0; 2],
            macros: [
                params.global.macros[0].value.value(),
                params.global.macros[1].value.value(),
                params.global.macros[2].value.value(),
                params.global.macros[3].value.value(),
            ],
            pitch_bend: (self.pitch_bend + params.global.pitch_wheel.value()).clamp(-1.0, 1.0),
            mod_wheel: (self.mod_wheel + params.global.mod_wheel.value()).clamp(0.0, 1.0),
            aftertouch: self.aftertouch.clamp(0.0, 1.0),
        };

        for i in 0..4 {
            frame.mono_lfo[i] = self.mono_lfos[i].process_sample(&params.lfos[i], bpm, 0.0, 0.0);
        }
        for i in 0..2 {
            frame.stp[i] = self.steppers[i].process_sample(i, &params.steppers[i], shared, bpm);
        }

        // Evaluate global-only modulation contributions (active even when 0 voices are playing)
        let mut global_only = GlobalModOffsets::default();

        // Mono LFO direct slots -> global targets
        for lfo_idx in 0..4 {
            if params.lfos[lfo_idx].mono.value() {
                let v = frame.mono_lfo[lfo_idx];
                for slot in &params.lfos[lfo_idx].mod_slots {
                    let tgt = slot.target.value();
                    if is_global_target(tgt) {
                        global_only.add_target(tgt, v * slot.amt.value());
                    }
                }
            }
        }

        // Mod matrix slots whose source is purely global -> global targets
        for slot_idx in 0..15 {
            let (src, tgt, amt) = resolve_mod_slot(slot_idx, params, shared);
            if amt == 0.0 || !is_global_target(tgt) {
                continue;
            }
            if let Some(src_val) = eval_global_only_source(src, &frame, params) {
                global_only.add_target(tgt, src_val * amt);
            }
        }

        global_only.clamp_safe();
        self.last_frame = frame;
        self.last_global_only = global_only;
        frame
    }

    /// Accumulate a voice's per-voice global-target modulations into the bridge.
    #[inline]
    pub fn accumulate_voice_global(&mut self, voice_global: &GlobalModOffsets) {
        self.bridge.accumulate_voice(voice_global);
    }

    /// Finalize global modulation offsets for FX & Master after all active voices have run.
    #[inline]
    pub fn finish_sample(&mut self) -> GlobalModOffsets {
        self.bridge.finish_sample(&self.last_global_only)
    }

    #[inline]
    pub fn lfo_phases(&self) -> [f32; 4] {
        [
            self.mono_lfos[0].current_phase(),
            self.mono_lfos[1].current_phase(),
            self.mono_lfos[2].current_phase(),
            self.mono_lfos[3].current_phase(),
        ]
    }

    #[inline]
    pub fn stepper_steps(&self) -> [u8; 2] {
        [
            self.steppers[0].current_step(),
            self.steppers[1].current_step(),
        ]
    }
}

// ============================================================================
// Per-Voice Modulator State (ENV 1..4, Poly LFO 1..4, Mod Matrix Evaluator)
// ============================================================================

/// Complete per-voice modulation state owned by each synth voice.
#[derive(Debug, Clone, Copy)]
pub struct VoiceModState {
    pub envs: [Envelope6Stage; 4],
    pub lfos: [LfoEngine; 4],
    pub velocity: f32,
    pub keytrack: f32,
    pub voice_random: f32,
    rng_state: u32,
    prev_env_att_mod: [f32; 2],
    prev_env_dec_mod: [f32; 2],
    prev_lfo_rate_mod: [f32; 2],
    prev_lfo_amt_mod: [f32; 2],
}

impl Default for VoiceModState {
    fn default() -> Self {
        Self::new(44100.0, 1)
    }
}

impl VoiceModState {
    pub fn new(sample_rate: f32, voice_id: u32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        let base_seed = voice_id.wrapping_mul(0x9E37_79B9).wrapping_add(0x243F_6A88);
        Self {
            envs: [
                Envelope6Stage::new(sr),
                Envelope6Stage::new(sr),
                Envelope6Stage::new(sr),
                Envelope6Stage::new(sr),
            ],
            lfos: [
                LfoEngine::new(sr, base_seed ^ 0x1111_2222),
                LfoEngine::new(sr, base_seed ^ 0x3333_4444),
                LfoEngine::new(sr, base_seed ^ 0x5555_6666),
                LfoEngine::new(sr, base_seed ^ 0x7777_8888),
            ],
            velocity: 1.0,
            keytrack: 0.0,
            voice_random: 0.0,
            rng_state: base_seed | 1,
            prev_env_att_mod: [0.0; 2],
            prev_env_dec_mod: [0.0; 2],
            prev_lfo_rate_mod: [0.0; 2],
            prev_lfo_amt_mod: [0.0; 2],
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sanitize_sample_rate(sample_rate);
        for env in &mut self.envs {
            env.set_sample_rate(sr);
        }
        for lfo in &mut self.lfos {
            lfo.set_sample_rate(sr);
        }
    }

    pub fn reset(&mut self) {
        for env in &mut self.envs {
            env.reset();
        }
        for lfo in &mut self.lfos {
            lfo.reset();
        }
        self.velocity = 1.0;
        self.keytrack = 0.0;
        self.voice_random = 0.0;
        self.prev_env_att_mod = [0.0; 2];
        self.prev_env_dec_mod = [0.0; 2];
        self.prev_lfo_rate_mod = [0.0; 2];
        self.prev_lfo_amt_mod = [0.0; 2];
    }

    /// Trigger all 4 voice envelopes and poly LFOs on Note On.
    #[inline]
    pub fn note_on(
        &mut self,
        note: u8,
        velocity: f32,
        legato_continue: bool,
        params: &PhantomSpireParams,
    ) {
        self.velocity = if velocity.is_finite() {
            velocity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        // Centered at middle C (MIDI 60), spanning roughly [-1.0, +1.0] across 120 semitones
        self.keytrack = ((note as f32 - 60.0) / 60.0).clamp(-1.0, 1.0);
        if !legato_continue {
            self.voice_random = next_random_bipolar(&mut self.rng_state);
        }
        for env in &mut self.envs {
            env.note_on(self.velocity, legato_continue);
        }
        if !legato_continue {
            for (i, lfo) in self.lfos.iter_mut().enumerate() {
                if !params.lfos[i].mono.value() {
                    lfo.trigger(params.lfos[i].phase.value(), params.lfos[i].fade.value());
                }
            }
        }
    }

    /// Trigger release on all 4 voice envelopes.
    #[inline]
    pub fn note_off(&mut self) {
        for env in &mut self.envs {
            env.note_off();
        }
    }

    /// Returns `true` while the amplitude envelope (ENV1) is still active.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.envs[0].is_active()
    }

    /// Current amplitude envelope (ENV1) value with velocity sensitivity applied.
    #[inline]
    pub fn amp_env_value(&self, params: &PhantomSpireParams) -> f32 {
        self.envs[0].velocity_scaled_value(params.envs[0].vel.value())
    }

    /// Current raw levels of ENV 1..4 (useful for UI metering in `SharedUiState`).
    #[inline]
    pub fn env_levels(&self) -> [f32; 4] {
        [
            self.envs[0].value(),
            self.envs[1].value(),
            self.envs[2].value(),
            self.envs[3].value(),
        ]
    }

    /// Advance all voice modulators by one sample and evaluate:
    /// 1. ENV 1..4 direct mod slots (2 per env, with per-slot `vel` scaling)
    /// 2. LFO 1..4 direct mod slots (2 per lfo)
    /// 3. All 15 Mod Matrix slots
    ///
    /// Returns `(VoiceModOffsets, GlobalModOffsets, amp_gain)` where `GlobalModOffsets` contains
    /// this voice's per-voice contributions to global FX/Master targets for `VoiceToGlobalModBridge`.
    #[inline]
    pub fn process_sample(
        &mut self,
        params: &PhantomSpireParams,
        shared: &SharedSynthState,
        global: &GlobalModFrame,
        bpm: f32,
    ) -> (VoiceModOffsets, GlobalModOffsets, f32) {
        // 1. Step ENV 1..4 (using 1-sample-delayed attack/decay mod offsets for zero-cycle stability)
        let mut env_vals = [0.0_f32; 4];
        for i in 0..4 {
            let att_m = if i < 2 { self.prev_env_att_mod[i] } else { 0.0 };
            let dec_m = if i < 2 { self.prev_env_dec_mod[i] } else { 0.0 };
            env_vals[i] = self.envs[i].process_sample(&params.envs[i], att_m, dec_m);
        }

        // 2. Step LFO 1..4 (mono LFOs come from GlobalModFrame; poly LFOs step per voice)
        let mut lfo_vals = [0.0_f32; 4];
        for i in 0..4 {
            let rate_m = if i < 2 { self.prev_lfo_rate_mod[i] } else { 0.0 };
            let amt_m = if i < 2 { self.prev_lfo_amt_mod[i] } else { 0.0 };
            if params.lfos[i].mono.value() {
                lfo_vals[i] = (global.mono_lfo[i] * (1.0 + amt_m).clamp(0.0, 2.0)).clamp(-2.0, 2.0);
            } else {
                lfo_vals[i] = self.lfos[i].process_sample(&params.lfos[i], bpm, rate_m, amt_m);
            }
        }

        let mut voice_mods = VoiceModOffsets::default();
        let mut voice_global_mods = GlobalModOffsets::default();

        // 3. Evaluate ENV 1..4 direct modulation slots (2 slots each)
        for env_idx in 0..4 {
            let raw_env = env_vals[env_idx];
            let env_vel_scale = 1.0 - params.envs[env_idx].vel.value()
                + params.envs[env_idx].vel.value() * self.velocity;
            for slot in &params.envs[env_idx].mod_slots {
                let tgt = slot.target.value();
                let amt = slot.amt.value();
                if tgt == ModTarget::Off || amt == 0.0 {
                    continue;
                }
                let slot_vel = slot.vel.value().clamp(0.0, 1.0);
                let slot_vel_scale = 1.0 - slot_vel + slot_vel * self.velocity;
                let delta = raw_env * env_vel_scale * slot_vel_scale * amt;
                if is_global_target(tgt) {
                    voice_global_mods.add_target(tgt, delta);
                } else {
                    voice_mods.add_target(tgt, delta);
                }
            }
        }

        // 4. Evaluate LFO 1..4 direct modulation slots (2 slots each)
        for lfo_idx in 0..4 {
            let val = lfo_vals[lfo_idx];
            let is_mono = params.lfos[lfo_idx].mono.value();
            for slot in &params.lfos[lfo_idx].mod_slots {
                let tgt = slot.target.value();
                let amt = slot.amt.value();
                if tgt == ModTarget::Off || amt == 0.0 {
                    continue;
                }
                let delta = val * amt;
                if is_global_target(tgt) {
                    // Mono LFO -> global target is already summed in GlobalModState::begin_sample
                    if !is_mono {
                        voice_global_mods.add_target(tgt, delta);
                    }
                } else {
                    voice_mods.add_target(tgt, delta);
                }
            }
        }

        // 5. Evaluate 15-slot Mod Matrix
        for slot_idx in 0..15 {
            let (src, tgt, amt) = resolve_mod_slot(slot_idx, params, shared);
            if src == ModSource::Off || tgt == ModTarget::Off || amt == 0.0 {
                continue;
            }
            let src_is_global_only = eval_global_only_source(src, global, params).is_some();
            if is_global_target(tgt) && src_is_global_only {
                // Already accounted for once in GlobalModState::begin_sample
                continue;
            }
            let src_val = self.eval_source(src, &env_vals, &lfo_vals, global);
            let delta = src_val * amt;
            if is_global_target(tgt) {
                voice_global_mods.add_target(tgt, delta);
            } else {
                voice_mods.add_target(tgt, delta);
            }
        }

        voice_mods.clamp_safe();
        voice_global_mods.clamp_safe();

        // Store feedback targets for next sample
        self.prev_env_att_mod = voice_mods.env_attack;
        self.prev_env_dec_mod = voice_mods.env_decay;
        self.prev_lfo_rate_mod = voice_mods.lfo_rate;
        self.prev_lfo_amt_mod = voice_mods.lfo_amt;

        let amp_gain = self.envs[0].velocity_scaled_value(params.envs[0].vel.value());
        (voice_mods, voice_global_mods, amp_gain)
    }

    #[inline]
    fn eval_source(
        &self,
        src: ModSource,
        env_vals: &[f32; 4],
        lfo_vals: &[f32; 4],
        global: &GlobalModFrame,
    ) -> f32 {
        match src {
            ModSource::Off => 0.0,
            ModSource::Env1 => env_vals[0],
            ModSource::Env2 => env_vals[1],
            ModSource::Env3 => env_vals[2],
            ModSource::Env4 => env_vals[3],
            ModSource::Lfo1 => lfo_vals[0],
            ModSource::Lfo2 => lfo_vals[1],
            ModSource::Lfo3 => lfo_vals[2],
            ModSource::Lfo4 => lfo_vals[3],
            ModSource::Stp1 => global.stp[0],
            ModSource::Stp2 => global.stp[1],
            ModSource::Macro1 => global.macros[0],
            ModSource::Macro2 => global.macros[1],
            ModSource::Macro3 => global.macros[2],
            ModSource::Macro4 => global.macros[3],
            ModSource::Velocity => self.velocity,
            ModSource::Keytrack => self.keytrack,
            ModSource::ModWheel => global.mod_wheel,
            ModSource::PitchBend => global.pitch_bend,
            ModSource::Aftertouch => global.aftertouch,
            ModSource::Random => self.voice_random,
        }
    }
}

/// Resolve a Mod Matrix slot from `PhantomSpireParams` with fallback to `SharedSynthState`
/// when `params.mod_slots[i]` is `Off` while `extra_state` populated numeric routes.
#[inline]
fn resolve_mod_slot(
    slot_idx: usize,
    params: &PhantomSpireParams,
    shared: &SharedSynthState,
) -> (ModSource, ModTarget, f32) {
    let slot = &params.mod_slots[slot_idx];
    let amt = slot.amount.value();
    let mut src = slot.source.value();
    let mut tgt = slot.target.value();
    if src == ModSource::Off && tgt == ModTarget::Off {
        let (s_u8, t_u8) = shared.read_mod_route(slot_idx);
        if s_u8 != 0 && t_u8 != 0 {
            src = mod_source_from_u8(s_u8);
            tgt = mod_target_from_u8(t_u8);
        }
    }
    (src, tgt, amt)
}

/// Evaluate purely global sources (returns `None` for per-voice sources).
#[inline]
fn eval_global_only_source(
    src: ModSource,
    global: &GlobalModFrame,
    params: &PhantomSpireParams,
) -> Option<f32> {
    match src {
        ModSource::Lfo1 if params.lfos[0].mono.value() => Some(global.mono_lfo[0]),
        ModSource::Lfo2 if params.lfos[1].mono.value() => Some(global.mono_lfo[1]),
        ModSource::Lfo3 if params.lfos[2].mono.value() => Some(global.mono_lfo[2]),
        ModSource::Lfo4 if params.lfos[3].mono.value() => Some(global.mono_lfo[3]),
        ModSource::Stp1 => Some(global.stp[0]),
        ModSource::Stp2 => Some(global.stp[1]),
        ModSource::Macro1 => Some(global.macros[0]),
        ModSource::Macro2 => Some(global.macros[1]),
        ModSource::Macro3 => Some(global.macros[2]),
        ModSource::Macro4 => Some(global.macros[3]),
        ModSource::ModWheel => Some(global.mod_wheel),
        ModSource::PitchBend => Some(global.pitch_bend),
        ModSource::Aftertouch => Some(global.aftertouch),
        _ => None,
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{set_bool_param, set_enum_param, set_float_param};

    #[test]
    fn envelope_6_stage_tracks_att_dec_sus_slope_up_down_and_release() {
        for sr in [44100.0_f32, 48000.0, 96000.0] {
            // Rising slope (sll > 0.5 like Spire Pic 1 ENV1)
            let mut env = Envelope6Stage::new(sr);
            env.note_on(1.0, false);

            let att_n = (0.02 * sr) as usize;
            let mut peak = 0.0_f32;
            for _ in 0..(att_n + 200) {
                let v = env.process_sample_raw(0.02, 0.05, 0.4, 0.05, 0.9, 0.05);
                assert!(v.is_finite() && (0.0..=1.0).contains(&v));
                peak = peak.max(v);
            }
            assert!(peak >= 0.99, "Attack should reach peak at sr={sr}");

            // Decay down toward sustain 0.4
            let mut min_after_dec = 1.0_f32;
            for _ in 0..((0.06 * sr) as usize) {
                let v = env.process_sample_raw(0.02, 0.05, 0.4, 0.05, 0.9, 0.05);
                min_after_dec = min_after_dec.min(v);
            }
            assert!(
                min_after_dec < 0.48,
                "Decay should approach sustain=0.4, got {min_after_dec}"
            );

            // Slope up toward ~0.88 (0.4 + 0.6 * 0.8 = 0.88)
            let mut after_slope = 0.0_f32;
            for _ in 0..((0.08 * sr) as usize) {
                after_slope = env.process_sample_raw(0.02, 0.05, 0.4, 0.05, 0.9, 0.05);
            }
            assert!(
                after_slope > 0.80,
                "Slope up (sll=0.9) should swell above 0.80, got {after_slope}"
            );

            // Release to Idle
            env.note_off();
            for _ in 0..((0.15 * sr) as usize) {
                let v = env.process_sample_raw(0.02, 0.05, 0.4, 0.05, 0.9, 0.05);
                assert!(v.is_finite());
            }
            assert!(!env.is_active(), "Envelope should reach Idle after release");

            // Pluck neutral sll=0.5 with sus=0.0 must stay at 0.0 (no unwanted swell)
            let mut pluck = Envelope6Stage::new(sr);
            pluck.note_on(1.0, false);
            let mut end_val = 1.0_f32;
            for _ in 0..((0.15 * sr) as usize) {
                end_val = pluck.process_sample_raw(0.002, 0.03, 0.0, 0.05, 0.5, 0.05);
            }
            assert!(
                end_val < 0.02,
                "Pluck with sus=0.0, sll=0.5 should decay to 0, got {end_val}"
            );
        }
    }

    #[test]
    fn lfo_waveforms_morph_sym_fade_and_sync_produce_distinct_outputs() {
        let sr = 48000.0;
        let waves = [
            LfoWaveform::Sin,
            LfoWaveform::Tri,
            LfoWaveform::SawUp,
            LfoWaveform::SawDn,
            LfoWaveform::Square,
            LfoWaveform::Snh,
            LfoWaveform::SmoothRnd,
            LfoWaveform::Ramp,
        ];

        let mut signatures = Vec::new();
        for &w in &waves {
            let mut lfo = LfoEngine::new(sr, 0xCAFE_BABE);
            lfo.trigger(0.0, 0.0);
            let mut buf = Vec::with_capacity(4800);
            for _ in 0..4800 {
                let v = lfo.process_sample_raw(5.0, w, 0.2, 0.5, 0.0, 0.0, false, 1.0);
                assert!(v.is_finite() && (-1.0..=1.0).contains(&v));
                buf.push(v);
            }
            let rms = (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt();
            assert!(rms > 0.15, "LFO wave {w:?} should have measurable output");
            signatures.push(buf);
        }

        // Verify all 8 waveforms are pairwise measurably distinct
        for i in 0..signatures.len() {
            for j in (i + 1)..signatures.len() {
                let diff: f32 = signatures[i]
                    .iter()
                    .zip(signatures[j].iter())
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f32>()
                    / signatures[i].len() as f32;
                assert!(
                    diff > 0.02,
                    "Waves {:?} and {:?} too similar (diff={diff})",
                    waves[i],
                    waves[j]
                );
            }
        }

        // Verify fade-in starts near 0 and grows toward full amplitude
        let mut lfo_fade = LfoEngine::new(sr, 42);
        lfo_fade.trigger(0.25, 0.5);
        let early = lfo_fade
            .process_sample_raw(2.0, LfoWaveform::Square, 0.0, 0.5, 0.0, 0.5, false, 1.0)
            .abs();
        for _ in 0..24000 {
            lfo_fade.process_sample_raw(2.0, LfoWaveform::Square, 0.0, 0.5, 0.0, 0.5, false, 1.0);
        }
        let late = lfo_fade
            .process_sample_raw(2.0, LfoWaveform::Square, 0.0, 0.5, 0.0, 0.5, false, 1.0)
            .abs();
        assert!(early < 0.05 && late > 0.9, "LFO fade failed: {early} -> {late}");

        // Verify unipolar stays in [0, 1]
        let mut lfo_uni = LfoEngine::new(sr, 7);
        for _ in 0..2400 {
            let v = lfo_uni.process_sample_raw(4.0, LfoWaveform::Sin, 0.0, 0.5, 0.0, 0.0, true, 1.0);
            assert!((0.0..=1.0).contains(&v));
        }

        // Verify tempo sync Hz math
        assert!((sync_div_to_hz(SyncDiv::D1_4, 120.0) - 2.0).abs() < 1e-4);
        assert!((sync_div_to_hz(SyncDiv::D1_16, 120.0) - 8.0).abs() < 1e-4);
    }

    #[test]
    fn stepper_steps_gate_ties_and_smooth_work_accurately() {
        let sr = 48000.0;
        let mut stp = StepperEngine::new(sr);
        let mut pattern = [0.5_f32; 16];
        pattern[0] = 1.0; // +1.0
        pattern[1] = 0.0; // -1.0
        pattern[2] = 0.75; // +0.5
        pattern[3] = 0.25; // -0.5

        // At 8 Hz (1/16 at 120 BPM), each step is 6000 samples.
        // Mid-step 0 (sample 1500): should be near +1.0
        let mut val_s0 = 0.0;
        for _ in 0..1500 {
            val_s0 = stp.process_sample_raw(8.0, 0.5, 0.0, 4, true, &pattern, 0);
        }
        assert_eq!(stp.current_step(), 0);
        assert!(val_s0 > 0.95, "Step 0 should be +1.0, got {val_s0}");

        // Past gate (sample 4500 of step 0): gate=0.5, untied -> drops to ~0.0
        let mut val_gated = 0.0;
        for _ in 0..3000 {
            val_gated = stp.process_sample_raw(8.0, 0.5, 0.0, 4, true, &pattern, 0);
        }
        assert!(val_gated.abs() < 0.05, "Past gate should drop to 0, got {val_gated}");

        // Advance into step 1 (sample 7500 total): should be near -1.0
        let mut val_s1 = 0.0;
        for _ in 0..3000 {
            val_s1 = stp.process_sample_raw(8.0, 0.5, 0.0, 4, true, &pattern, 0);
        }
        assert_eq!(stp.current_step(), 1);
        assert!(val_s1 < -0.95, "Step 1 should be -1.0, got {val_s1}");

        // Tied step 0 ignores gate drop
        let mut stp_tied = StepperEngine::new(sr);
        let mut val_tied = 0.0;
        for _ in 0..4500 {
            val_tied = stp_tied.process_sample_raw(8.0, 0.5, 0.0, 4, true, &pattern, 0b0001);
        }
        assert!(val_tied > 0.95, "Tied step should stay high past gate, got {val_tied}");
    }

    #[test]
    fn mod_matrix_and_voice_to_global_fx_bridge_work_end_to_end() {
        let sr = 48000.0;
        let mut params = PhantomSpireParams::default();
        let shared = SharedSynthState::default();

        // Route ENV1 direct slot 1 -> FX Reverb Mix (just like Spire Pic 1!)
        set_enum_param(&mut params.envs[0].mod_slots[0].target, ModTarget::FxReverbMix);
        set_float_param(&mut params.envs[0].mod_slots[0].amt, 0.75);
        // Route Mod Matrix slot 0: Macro 1 -> Filter1 Cutoff
        set_enum_param(&mut params.mod_slots[0].source, ModSource::Macro1);
        set_enum_param(&mut params.mod_slots[0].target, ModTarget::Filter1Cutoff);
        set_float_param(&mut params.mod_slots[0].amount, 0.5);
        set_float_param(&mut params.global.macros[0].value, 0.8);
        // Route Mod Matrix slot 1: Macro 2 -> FX Shaper Drive (pure global -> global)
        set_enum_param(&mut params.mod_slots[1].source, ModSource::Macro2);
        set_enum_param(&mut params.mod_slots[1].target, ModTarget::FxShaperDrive);
        set_float_param(&mut params.mod_slots[1].amount, 0.6);
        set_float_param(&mut params.global.macros[1].value, 0.5);

        let mut global_mod = GlobalModState::new(sr);
        let mut voice1 = VoiceModState::new(sr, 1);
        let mut voice2 = VoiceModState::new(sr, 2);
        voice1.note_on(60, 1.0, false, &params);
        voice2.note_on(67, 1.0, false, &params);

        let mut last_voice_mods = VoiceModOffsets::default();
        let mut last_global_mods = GlobalModOffsets::default();
        for _ in 0..1000 {
            let frame = global_mod.begin_sample(&params, &shared, 120.0);
            let (vm1, vg1, _) = voice1.process_sample(&params, &shared, &frame, 120.0);
            let (_vm2, vg2, _) = voice2.process_sample(&params, &shared, &frame, 120.0);
            global_mod.accumulate_voice_global(&vg1);
            global_mod.accumulate_voice_global(&vg2);
            last_voice_mods = vm1;
            last_global_mods = global_mod.finish_sample();
        }

        // Macro1 (0.8) * 0.5 = 0.4 on Filter1 Cutoff
        assert!((last_voice_mods.filter_cutoff[0] - 0.4).abs() < 1e-4);
        // Macro2 (0.5) * 0.6 = 0.3 on FX Shaper Drive
        assert!((last_global_mods.fx_shaper_drive - 0.3).abs() < 0.02);
        // ENV1 (~1.0) * 0.75 -> FX Reverb Mix (~0.75, NOT 1.50 even with 2 voices!)
        assert!(
            last_global_mods.fx_reverb_mix > 0.55 && last_global_mods.fx_reverb_mix <= 0.80,
            "Voice-to-global reverb mix bridge out of expected range: {}",
            last_global_mods.fx_reverb_mix
        );
    }

    #[test]
    fn lfo_and_stepper_follow_host_bpm_90_to_160_without_clicks_or_nans() {
        let sr = 48000.0;
        let mut params = PhantomSpireParams::default();
        let shared = SharedSynthState::default();

        for i in 0..4 {
            set_bool_param(&mut params.lfos[i].sync, true);
            set_enum_param(&mut params.lfos[i].sync_div, SyncDiv::D1_4);
        }
        for i in 0..2 {
            set_bool_param(&mut params.steppers[i].sync, true);
            set_enum_param(&mut params.steppers[i].sync_div, SyncDiv::D1_16);
        }

        // Measure LFO phase advance over 1.0 s at 90 BPM vs 160 BPM
        // At 1/4 div: 90 BPM = 1.5 cycles/s, 160 BPM = 2.667 cycles/s (ratio = 160/90)
        let measure_lfo_cycles = |bpm: f32| -> f32 {
            let mut lfo = LfoEngine::new(sr, 123);
            let mut prev_p = lfo.current_phase();
            let mut cycles = 0.0_f32;
            for _ in 0..48000 {
                let v = lfo.process_sample(&params.lfos[0], bpm, 0.0, 0.0);
                assert!(v.is_finite());
                let cur_p = lfo.current_phase();
                let dp = if cur_p >= prev_p {
                    cur_p - prev_p
                } else {
                    1.0 - prev_p + cur_p
                };
                cycles += dp;
                prev_p = cur_p;
            }
            cycles
        };

        let c_90 = measure_lfo_cycles(90.0);
        let c_160 = measure_lfo_cycles(160.0);
        assert!((c_90 - 1.5).abs() < 0.02, "90 BPM 1/4 LFO expected 1.5 cyc/s, got {c_90}");
        assert!(
            (c_160 - (160.0 / 60.0)).abs() < 0.02,
            "160 BPM 1/4 LFO expected 2.667 cyc/s, got {c_160}"
        );

        // Measure Stepper transitions over 1.0 s at 90 BPM vs 160 BPM (1/16 div)
        // 90 BPM 1/16 = 6 steps/s -> 6 steps in ~1.0 s; 160 BPM 1/16 = 10.67 steps/s -> 10 steps in ~1.0 s
        let count_stp_steps = |bpm: f32| -> usize {
            let mut stp = StepperEngine::new(sr);
            let mut prev = stp.current_step();
            let mut transitions = 0usize;
            for _ in 0..48020 {
                let v = stp.process_sample(0, &params.steppers[0], &shared, bpm);
                assert!(v.is_finite());
                let cur = stp.current_step();
                if cur != prev {
                    transitions += 1;
                    prev = cur;
                }
            }
            transitions
        };

        let s_90 = count_stp_steps(90.0);
        let s_160 = count_stp_steps(160.0);
        assert_eq!(s_90, 6, "90 BPM 1/16 stepper should advance 6 steps in 1.0s");
        assert_eq!(s_160, 10, "160 BPM 1/16 stepper should advance 10 steps in 1.0s");

        // Rapid host BPM sweeps including NaN/extreme values stay finite & click-free
        let mut gmod = GlobalModState::new(sr);
        for (idx, &b) in [90.0_f32, 160.0, 40.0, 240.0, f32::NAN, 120.0].iter().enumerate() {
            for s in 0..2048 {
                let ramp_bpm = if b.is_finite() {
                    b + (s as f32 * 0.01) * if idx % 2 == 0 { 1.0 } else { -1.0 }
                } else {
                    b
                };
                let frame = gmod.begin_sample(&params, &shared, ramp_bpm);
                for v in frame.mono_lfo.iter().chain(frame.stp.iter()) {
                    assert!(v.is_finite() && v.abs() <= 2.0);
                }
            }
        }
    }
}
