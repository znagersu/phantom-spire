pub mod arp;
pub mod filter;
pub mod fx;
pub mod master;
pub mod mod_engine;
pub mod osc;
pub mod voice;

use crate::params::PhantomSpireParams;
use crate::{SharedSynthState, SharedUiState};
use arp::{ArpEvent, ArpEvents, Arpeggiator};
use fx::FxRack;
use master::MasterSection;
use mod_engine::{GlobalModState, VoiceModState};
use nih_plug::prelude::*;
use std::sync::atomic::Ordering;
use voice::{VoiceAllocator, VoiceRenderSettings, VoiceTriggerEvent, MAX_VOICES};

const PENDING_MIDI_CAP: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct TransportInfo {
    pub tempo_bpm: f32,
    pub playing: bool,
    pub sample_rate: f32,
    pub ppq_pos: Option<f64>,
    pub pos_samples: Option<i64>,
}

impl Default for TransportInfo {
    fn default() -> Self {
        Self {
            tempo_bpm: 120.0,
            playing: false,
            sample_rate: 44100.0,
            ppq_pos: None,
            pos_samples: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PendingMidiEvent {
    NoteOn { note: u8, velocity: f32, channel: u8 },
    NoteOff { note: u8, channel: u8 },
    Sustain(bool),
    AllNotesOff,
}

/// Complete real-time safe polyphonic Phantom Spire synthesis engine:
/// `Arpeggiator -> VoiceAllocator (4x Multi-Mode Osc + Dual Filter) + ModEngine (ENV1..4, LFO1..4, STP1..2, ModMatrix) -> 5-Unit FX Rack -> Master`.
pub struct SynthEngine {
    sample_rate: f32,
    arp: Arpeggiator,
    allocator: VoiceAllocator,
    voice_mods: [VoiceModState; MAX_VOICES],
    mod_gate_open: [bool; MAX_VOICES],
    global_mod: GlobalModState,
    fx_rack: FxRack,
    master: MasterSection,

    poly_headroom_gain: f32,
    poly_headroom_slew: f32,

    pitch_bend: f32,
    mod_wheel: f32,
    channel_pressure: f32,
    last_env_levels: [f32; 4],
    last_lfo_phases: [f32; 4],
    last_note_hint: i8,

    pending: [PendingMidiEvent; PENDING_MIDI_CAP],
    pending_len: usize,
}

impl Default for SynthEngine {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl SynthEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() && sample_rate >= 8000.0 {
            sample_rate
        } else {
            44100.0
        };
        Self {
            sample_rate: sr,
            arp: Arpeggiator::new(sr),
            allocator: VoiceAllocator::new(sr),
            voice_mods: std::array::from_fn(|i| VoiceModState::new(sr, (i as u32) + 1)),
            mod_gate_open: [false; MAX_VOICES],
            global_mod: GlobalModState::new(sr),
            fx_rack: FxRack::new(sr),
            master: MasterSection::new(sr),
            poly_headroom_gain: 1.0,
            poly_headroom_slew: 1.0 - (-1.0 / (0.015 * sr).max(1.0)).exp(),
            pitch_bend: 0.0,
            mod_wheel: 0.0,
            channel_pressure: 0.0,
            last_env_levels: [0.0; 4],
            last_lfo_phases: [0.0; 4],
            last_note_hint: -1,
            pending: [PendingMidiEvent::AllNotesOff; PENDING_MIDI_CAP],
            pending_len: 0,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate >= 8000.0 {
            self.sample_rate = sample_rate;
            self.poly_headroom_slew = 1.0 - (-1.0 / (0.015 * sample_rate).max(1.0)).exp();
            self.arp.set_sample_rate(sample_rate);
            self.allocator.set_sample_rate(sample_rate);
            for vm in &mut self.voice_mods {
                vm.set_sample_rate(sample_rate);
            }
            self.global_mod.set_sample_rate(sample_rate);
            self.fx_rack.set_sample_rate(sample_rate);
            self.master.set_sample_rate(sample_rate);
        }
    }

    pub fn reset(&mut self) {
        self.arp.reset();
        self.allocator.reset();
        for vm in &mut self.voice_mods {
            vm.reset();
        }
        self.mod_gate_open = [false; MAX_VOICES];
        self.global_mod.reset();
        self.fx_rack.reset();
        self.master.reset();
        self.poly_headroom_gain = 1.0;
        self.pitch_bend = 0.0;
        self.mod_wheel = 0.0;
        self.channel_pressure = 0.0;
        self.last_env_levels = [0.0; 4];
        self.last_lfo_phases = [0.0; 4];
        self.last_note_hint = -1;
        self.pending_len = 0;
    }

    #[inline]
    fn push_pending(&mut self, ev: PendingMidiEvent) {
        if self.pending_len < PENDING_MIDI_CAP {
            self.pending[self.pending_len] = ev;
            self.pending_len += 1;
        }
    }

    /// Queue a NoteOn to be processed at the start of the next `process()` call with live params.
    pub fn note_on(&mut self, note: u8, velocity: f32, channel: u8) {
        if !velocity.is_finite() || velocity <= 0.0 {
            self.note_off(note, channel);
            return;
        }
        let n = note & 0x7F;
        self.last_note_hint = n as i8;
        self.push_pending(PendingMidiEvent::NoteOn {
            note: n,
            velocity: velocity.clamp(0.0, 1.0),
            channel,
        });
    }

    /// Queue a NoteOff to be processed at the start of the next `process()` call with live params.
    pub fn note_off(&mut self, note: u8, channel: u8) {
        self.push_pending(PendingMidiEvent::NoteOff {
            note: note & 0x7F,
            channel,
        });
    }

    /// Queue CC64 sustain pedal change.
    pub fn set_sustain_pedal(&mut self, down: bool) {
        self.push_pending(PendingMidiEvent::Sustain(down));
    }

    /// Immediately release all held/sounding notes and clear pending MIDI events.
    pub fn all_notes_off(&mut self) {
        self.pending_len = 0;
        let _ = self.arp.all_notes_off();
        let (released, count) = self.allocator.all_notes_off();
        for &idx in &released[..count] {
            self.voice_mods[idx].note_off();
            self.mod_gate_open[idx] = false;
        }
    }

    /// Sample-accurate NoteOn using live plugin parameters.
    pub fn note_on_with_params(
        &mut self,
        note: u8,
        velocity: f32,
        channel: u8,
        params: &PhantomSpireParams,
        voice_settings: &VoiceRenderSettings,
    ) {
        if !velocity.is_finite() || velocity <= 0.0 {
            self.note_off_with_params(note, channel, params, voice_settings);
            return;
        }
        let n = note & 0x7F;
        self.last_note_hint = n as i8;
        let events = self.arp.note_on(n, velocity.clamp(0.0, 1.0), channel, &params.arp);
        self.dispatch_arp_events(&events, params, voice_settings);
    }

    /// Sample-accurate NoteOff using live plugin parameters.
    pub fn note_off_with_params(
        &mut self,
        note: u8,
        channel: u8,
        params: &PhantomSpireParams,
        voice_settings: &VoiceRenderSettings,
    ) {
        let events = self.arp.note_off(note & 0x7F, channel, &params.arp);
        self.dispatch_arp_events(&events, params, voice_settings);
    }

    /// Sample-accurate CC64 sustain pedal using live plugin parameters.
    pub fn set_sustain_pedal_with_params(&mut self, down: bool) {
        let (released, count) = self.allocator.set_sustain_pedal(down);
        for &idx in &released[..count] {
            self.voice_mods[idx].note_off();
            self.mod_gate_open[idx] = false;
        }
    }

    #[inline]
    fn dispatch_arp_events(
        &mut self,
        events: &ArpEvents,
        params: &PhantomSpireParams,
        voice_settings: &VoiceRenderSettings,
    ) {
        for ev in events.iter() {
            match *ev {
                ArpEvent::NoteOn {
                    note,
                    velocity,
                    channel,
                } => {
                    let was_silent = self.allocator.active_voice_count() == 0;
                    let trig = self
                        .allocator
                        .note_on(note, velocity, channel, voice_settings);
                    self.apply_voice_trigger(trig, was_silent, params);
                }
                ArpEvent::NoteOff { note, channel } => {
                    let trig = self.allocator.note_off(note, channel, voice_settings);
                    self.apply_voice_trigger(trig, false, params);
                }
            }
        }
    }

    #[inline]
    fn apply_voice_trigger(
        &mut self,
        trig: VoiceTriggerEvent,
        was_silent: bool,
        params: &PhantomSpireParams,
    ) {
        match trig {
            VoiceTriggerEvent::NoteOn {
                voice_idx,
                note,
                velocity,
                retrigger_env,
                ..
            } => {
                if was_silent {
                    self.global_mod.on_first_note_on(params);
                }
                self.voice_mods[voice_idx].note_on(note, velocity, !retrigger_env, params);
                self.mod_gate_open[voice_idx] = true;
            }
            VoiceTriggerEvent::PitchChange {
                voice_idx,
                note,
                velocity,
                retrigger_env,
            } => {
                self.voice_mods[voice_idx].note_on(note, velocity, !retrigger_env, params);
                self.mod_gate_open[voice_idx] = true;
            }
            VoiceTriggerEvent::NoteOff { voice_idx } => {
                self.voice_mods[voice_idx].note_off();
                self.mod_gate_open[voice_idx] = false;
            }
            VoiceTriggerEvent::None => {}
        }
    }

    #[inline]
    fn drain_pending(
        &mut self,
        params: &PhantomSpireParams,
        voice_settings: &VoiceRenderSettings,
    ) {
        if self.pending_len == 0 {
            return;
        }
        let count = self.pending_len;
        let copy = self.pending;
        self.pending_len = 0;
        for ev in &copy[..count] {
            match *ev {
                PendingMidiEvent::NoteOn {
                    note,
                    velocity,
                    channel,
                } => self.note_on_with_params(note, velocity, channel, params, voice_settings),
                PendingMidiEvent::NoteOff { note, channel } => {
                    self.note_off_with_params(note, channel, params, voice_settings)
                }
                PendingMidiEvent::Sustain(down) => self.set_sustain_pedal_with_params(down),
                PendingMidiEvent::AllNotesOff => self.all_notes_off(),
            }
        }
    }

    pub fn set_pitch_bend(&mut self, value: f32) {
        if value.is_finite() {
            self.pitch_bend = value.clamp(-1.0, 1.0);
        }
    }

    #[inline]
    pub fn pitch_bend(&self) -> f32 {
        self.pitch_bend
    }

    pub fn set_mod_wheel(&mut self, value: f32) {
        if value.is_finite() {
            self.mod_wheel = value.clamp(0.0, 1.0);
        }
    }

    pub fn set_channel_pressure(&mut self, value: f32) {
        if value.is_finite() {
            self.channel_pressure = value.clamp(0.0, 1.0);
        }
    }

    pub fn active_voice_count(&self) -> u32 {
        self.allocator.active_voice_count()
    }

    pub fn is_arp_active(&self) -> bool {
        self.arp.current_step() < 16
    }

    pub fn active_note_mask(&self) -> u128 {
        self.allocator.active_output_note_mask()
    }

    pub fn last_note(&self) -> i8 {
        let alloc_last = self.allocator.last_note();
        if alloc_last >= 0 {
            alloc_last
        } else {
            self.last_note_hint
        }
    }

    #[inline]
    pub fn make_voice_settings(
        &self,
        params: &PhantomSpireParams,
        shared_synth: &SharedSynthState,
    ) -> VoiceRenderSettings {
        VoiceRenderSettings::from_params(params, shared_synth, self.pitch_bend)
    }

    /// Render a single stereo sample through the full Phantom Spire audio pipeline:
    /// `Arp -> VoiceAllocator + ModEngine -> FX Rack -> Master`.
    #[inline]
    pub fn process_sample_full(
        &mut self,
        params: &PhantomSpireParams,
        shared_synth: &SharedSynthState,
        transport: &TransportInfo,
        voice_settings: &VoiceRenderSettings,
        master_volume_gain: f32,
    ) -> (f32, f32) {
        self.drain_pending(params, voice_settings);

        let arp_events = self
            .arp
            .process_sample(&params.arp, shared_synth, transport);
        if !arp_events.is_empty() {
            self.dispatch_arp_events(&arp_events, params, voice_settings);
        }

        self.global_mod.pitch_bend = self.pitch_bend;
        self.global_mod.mod_wheel = self.mod_wheel;
        self.global_mod.aftertouch = self.channel_pressure;

        let bpm = transport.tempo_bpm;
        let global_frame = self.global_mod.begin_sample(params, shared_synth, bpm);

        let mut sum_l = 0.0_f32;
        let mut sum_r = 0.0_f32;
        let mut env_peaks = [0.0_f32; 4];
        let mut active_energy_voices = 0.0_f32;

        for i in 0..MAX_VOICES {
            if !self.allocator.voices()[i].is_active() {
                self.mod_gate_open[i] = false;
                continue;
            }
            // Keep envelope release synced if voice allocator closed the gate (e.g. Poly -> Mono switch)
            if self.mod_gate_open[i] && !self.allocator.voices()[i].is_gate_open() {
                self.voice_mods[i].note_off();
                self.mod_gate_open[i] = false;
            }

            let (v_mods, v_global_mods, amp_gain) =
                self.voice_mods[i].process_sample(params, shared_synth, &global_frame, bpm);
            self.global_mod.accumulate_voice_global(&v_global_mods);

            let v_mod_in = v_mods.to_voice_modulation(amp_gain);
            let (vl, vr) = self.allocator.voices_mut()[i].render_sample(voice_settings, &v_mod_in);
            sum_l += vl;
            sum_r += vr;
            active_energy_voices += amp_gain.clamp(0.0, 1.0);

            let levels = self.voice_mods[i].env_levels();
            for e in 0..4 {
                if levels[e] > env_peaks[e] {
                    env_peaks[e] = levels[e];
                }
            }

            if !self.voice_mods[i].is_active() {
                self.allocator.finish_voice(i);
                self.mod_gate_open[i] = false;
            }
        }

        // Smooth polyphonic headroom scaling so playing 4-6+ note chords doesn't push FX/Master into clipping
        let n_eff = active_energy_voices.max(1.0);
        let target_poly_scale = n_eff.powf(-0.28).clamp(0.42, 1.0);
        self.poly_headroom_gain +=
            (target_poly_scale - self.poly_headroom_gain) * self.poly_headroom_slew;
        sum_l *= self.poly_headroom_gain;
        sum_r *= self.poly_headroom_gain;

        self.last_env_levels = env_peaks;
        let global_mods = self.global_mod.finish_sample();
        self.last_lfo_phases = self.global_mod.lfo_phases();

        let (fx_l, fx_r) = self
            .fx_rack
            .process_sample(sum_l, sum_r, &params.fx, &global_mods, bpm);
        self.master
            .process_sample(fx_l, fx_r, &params.master, &global_mods, master_volume_gain)
    }

    pub fn sync_ui_state(&self, ui_state: &SharedUiState, peak_l: f32, peak_r: f32) {
        let in_mask = self.arp.held_note_mask();
        let out_mask = self.allocator.active_output_note_mask();
        ui_state.set_input_mask(in_mask);
        ui_state.set_output_mask(out_mask);
        ui_state
            .active_voice_count
            .store(self.allocator.active_voice_count(), Ordering::Relaxed);
        ui_state
            .last_note
            .store(self.last_note(), Ordering::Relaxed);
        ui_state.set_peaks(peak_l, peak_r);
        for i in 0..4 {
            ui_state.set_env_level(i, self.last_env_levels[i]);
            ui_state.set_lfo_phase(i, self.last_lfo_phases[i]);
        }
        ui_state
            .arp_step
            .store(self.arp.current_step(), Ordering::Relaxed);
        let stp_steps = self.global_mod.stepper_steps();
        ui_state.stp_steps[0].store(stp_steps[0], Ordering::Relaxed);
        ui_state.stp_steps[1].store(stp_steps[1], Ordering::Relaxed);
    }

    /// Block-level processing entrypoint (used by headless/integration tests and standalone callers).
    pub fn process(
        &mut self,
        buffer: &mut Buffer,
        params: &PhantomSpireParams,
        shared_synth: &SharedSynthState,
        ui_state: &SharedUiState,
        transport: &TransportInfo,
    ) {
        let num_samples = buffer.samples();
        if num_samples == 0 {
            return;
        }

        let mut peak_l = 0.0_f32;
        let mut peak_r = 0.0_f32;
        let mut voice_settings = self.make_voice_settings(params, shared_synth);

        for (sample_idx, channel_samples) in buffer.iter_samples().enumerate() {
            if sample_idx > 0 && (sample_idx & 15) == 0 {
                voice_settings = self.make_voice_settings(params, shared_synth);
            }
            voice_settings.filters[0].cutoff = params.filters[0].cutoff.smoothed.next();
            voice_settings.filters[0].resonance = params.filters[0].resonance.smoothed.next();
            voice_settings.filters[1].cutoff = params.filters[1].cutoff.smoothed.next();
            voice_settings.filters[1].resonance = params.filters[1].resonance.smoothed.next();
            let gain = params.master.volume.smoothed.next();
            let (out_l, out_r) = self.process_sample_full(
                params,
                shared_synth,
                transport,
                &voice_settings,
                gain,
            );
            peak_l = peak_l.max(out_l.abs());
            peak_r = peak_r.max(out_r.abs());

            let mut ch = channel_samples.into_iter();
            if let Some(l) = ch.next() {
                *l = out_l;
            }
            if let Some(r) = ch.next() {
                *r = out_r;
            }
            for extra in ch {
                *extra = 0.0;
            }
        }

        self.sync_ui_state(ui_state, peak_l, peak_r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_block(
        engine: &mut SynthEngine,
        params: &PhantomSpireParams,
        shared_synth: &SharedSynthState,
        ui_state: &SharedUiState,
        transport: &TransportInfo,
        len: usize,
    ) -> (Vec<f32>, Vec<f32>) {
        let mut ch_l = vec![0.0_f32; len];
        let mut ch_r = vec![0.0_f32; len];
        let mut buf = Buffer::default();
        unsafe {
            buf.set_slices(len, |s| {
                s.clear();
                s.push(&mut ch_l);
                s.push(&mut ch_r);
            });
        }
        engine.process(&mut buf, params, shared_synth, ui_state, transport);
        (ch_l, ch_r)
    }

    #[test]
    fn note_lifecycle_and_zero_velocity_behave_correctly() {
        let mut engine = SynthEngine::new(48000.0);
        let params = PhantomSpireParams::default();
        let shared = SharedSynthState::default();
        let ui = SharedUiState::default();
        let transport = TransportInfo {
            sample_rate: 48000.0,
            ..Default::default()
        };
        assert_eq!(engine.active_voice_count(), 0);

        engine.note_on(60, 0.8, 0);
        run_block(&mut engine, &params, &shared, &ui, &transport, 64);
        assert_eq!(engine.active_voice_count(), 1);
        assert_eq!(engine.last_note(), 60);

        // MIDI NoteOn with velocity 0 acts as NoteOff; after release tail, voice finishes
        engine.note_on(60, 0.0, 0);
        run_block(&mut engine, &params, &shared, &ui, &transport, 16000);
        assert_eq!(engine.active_voice_count(), 0);

        engine.note_on(64, 0.9, 0);
        engine.note_on(67, 0.7, 0);
        run_block(&mut engine, &params, &shared, &ui, &transport, 64);
        assert_eq!(engine.active_voice_count(), 2);

        engine.all_notes_off();
        run_block(&mut engine, &params, &shared, &ui, &transport, 16000);
        assert_eq!(engine.active_voice_count(), 0);
    }

    #[test]
    fn process_outputs_finite_audio_and_handles_zero_length_buffer() {
        let mut engine = SynthEngine::new(44100.0);
        let params = PhantomSpireParams::default();
        let shared_synth = SharedSynthState::default();
        let ui_state = SharedUiState::default();
        let transport = TransportInfo::default();

        // Zero-length buffer test
        let mut empty_l = [];
        let mut empty_r = [];
        let mut empty_buf = Buffer::default();
        unsafe {
            empty_buf.set_slices(0, |s| {
                s.clear();
                s.push(&mut empty_l);
                s.push(&mut empty_r);
            });
        }
        engine.process(&mut empty_buf, &params, &shared_synth, &ui_state, &transport);

        // Active note finite output test
        engine.note_on(69, 0.8, 0);
        let (ch_l, ch_r) = run_block(&mut engine, &params, &shared_synth, &ui_state, &transport, 512);

        assert!(ch_l.iter().all(|s| s.is_finite() && s.abs() <= 0.995));
        assert!(ch_r.iter().all(|s| s.is_finite() && s.abs() <= 0.995));
        assert!(ch_l.iter().any(|s| s.abs() > 0.01));
        assert_eq!(ui_state.active_voice_count.load(Ordering::Relaxed), 1);
        assert!(ui_state.get_env_level(0) > 0.0);
    }

    #[test]
    fn six_note_chord_headroom_and_limiter_prevent_clipping() {
        let sr = 44100.0;
        let mut engine = SynthEngine::new(sr);
        let mut params = PhantomSpireParams::default();
        let shared = SharedSynthState::default();
        let ui = SharedUiState::default();
        let transport = TransportInfo {
            sample_rate: sr,
            ..Default::default()
        };

        // Drive heavy 2-oscillator unison supersaw stack + master Boost
        crate::params::set_int_param(&mut params.oscs[0].unison_count, 7);
        crate::params::set_float_param(&mut params.oscs[0].mix_level, 0.9);
        crate::params::set_float_param(&mut params.oscs[1].mix_level, 0.6);
        crate::params::set_bool_param(&mut params.master.boost, true);
        crate::params::set_bool_param(&mut params.master.soft_limit, true);

        // 6-note rich jazz chord all triggered at velocity 1.0
        for &n in &[48u8, 55, 60, 64, 67, 71] {
            engine.note_on(n, 1.0, 0);
        }

        let mut peak = 0.0_f32;
        for _ in 0..40 {
            let (l, r) = run_block(&mut engine, &params, &shared, &ui, &transport, 512);
            for &s in l.iter().chain(r.iter()) {
                assert!(s.is_finite());
                peak = peak.max(s.abs());
            }
        }
        assert!(
            peak <= 0.985 && peak > 0.25,
            "6-note hot chord peak out of clean range: {peak}"
        );
    }
}
