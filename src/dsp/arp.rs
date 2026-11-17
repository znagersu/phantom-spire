//! Sample-accurate pre-voice Arpeggiator for Phantom Spire.
//!
//! Features:
//! - Modes: `Up`, `Down`, `UpDown`, `DownUp`, `Random`, `AsPlayed`, `Chord`
//! - `octaves` 1..4, `rate` (free Hz) or tempo-synced `sync_div`, `gate` (5%..100%), `swing` (0%..75%)
//! - `latch` mode, `vel_mode` (`NoteVel`, `PatternVel`, `Mixed`), `steps_count` (1..16)
//! - 16-step pattern (`steps_enabled`, `step_offsets` in semitones, `step_velocities` 0..127)
//!   read lock-free from `SharedSynthState`
//! - Driven by host BPM/PPQ in VST3 (`TransportInfo`) and deterministic internal BPM clock in Standalone
//! - Zero heap allocations on the audio thread; guaranteed clean `NoteOff`s with zero stuck notes.

use crate::dsp::mod_engine::{next_random_bipolar, sanitize_bpm, sanitize_sample_rate, sync_div_to_beats};
use crate::dsp::TransportInfo;
use crate::params::{ArpMode, ArpParams, ArpVelMode};
use crate::SharedSynthState;

const MAX_HELD_NOTES: usize = 32;
const MAX_SEQ_NOTES: usize = 128;
const MAX_EVENTS_PER_SAMPLE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArpEvent {
    NoteOn { note: u8, velocity: f32, channel: u8 },
    NoteOff { note: u8, channel: u8 },
}

/// Fixed-capacity, stack-allocated event list returned per sample (zero heap allocations).
#[derive(Debug, Clone, Copy)]
pub struct ArpEvents {
    items: [ArpEvent; MAX_EVENTS_PER_SAMPLE],
    len: usize,
}

impl Default for ArpEvents {
    fn default() -> Self {
        Self {
            items: [ArpEvent::NoteOff { note: 0, channel: 0 }; MAX_EVENTS_PER_SAMPLE],
            len: 0,
        }
    }
}

impl ArpEvents {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn push(&mut self, ev: ArpEvent) {
        if self.len < MAX_EVENTS_PER_SAMPLE {
            self.items[self.len] = ev;
            self.len += 1;
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn as_slice(&self) -> &[ArpEvent] {
        &self.items[..self.len]
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, ArpEvent> {
        self.items[..self.len].iter()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct HeldNote {
    note: u8,
    velocity: f32,
    channel: u8,
}

#[derive(Debug, Clone, Copy, Default)]
struct SoundingNote {
    note: u8,
    channel: u8,
}

/// Sample-accurate pre-voice arpeggiator.
pub struct Arpeggiator {
    sample_rate: f32,
    /// Physically held keys (in press order for `AsPlayed`).
    held: [HeldNote; MAX_HELD_NOTES],
    held_len: usize,
    /// Active arpeggiation pool (mirrors `held`, or retains last chord when `latch` is on).
    pool: [HeldNote; MAX_HELD_NOTES],
    pool_len: usize,
    /// Currently sounding arp-generated notes so gate-off / disable always sends exact NoteOffs.
    sounding: [SoundingNote; MAX_HELD_NOTES],
    sounding_len: usize,
    /// Pre-allocated scratch buffer for octave-expanded sequence.
    seq_buf: [HeldNote; MAX_SEQ_NOTES],

    was_enabled: bool,
    was_latch: bool,
    latch_armed_new_chord: bool,

    pattern_step: usize,
    seq_cursor: usize,
    step_elapsed_samples: f64,
    current_step_duration: f64,
    current_gate_samples: f64,
    last_base_step_samples: f64,
    gate_is_open: bool,
    needs_immediate_trigger: bool,
    last_host_ppq_step: Option<i64>,
    rng_state: u32,
}

impl Default for Arpeggiator {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl Arpeggiator {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sanitize_sample_rate(sample_rate),
            held: [HeldNote::default(); MAX_HELD_NOTES],
            held_len: 0,
            pool: [HeldNote::default(); MAX_HELD_NOTES],
            pool_len: 0,
            sounding: [SoundingNote::default(); MAX_HELD_NOTES],
            sounding_len: 0,
            seq_buf: [HeldNote::default(); MAX_SEQ_NOTES],
            was_enabled: false,
            was_latch: false,
            latch_armed_new_chord: false,
            pattern_step: 0,
            seq_cursor: 0,
            step_elapsed_samples: 0.0,
            current_step_duration: 5512.5,
            current_gate_samples: 4134.0,
            last_base_step_samples: 5512.5,
            gate_is_open: false,
            needs_immediate_trigger: false,
            last_host_ppq_step: None,
            rng_state: 0x517C_C1B7,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitize_sample_rate(sample_rate);
    }

    pub fn reset(&mut self) {
        self.held_len = 0;
        self.pool_len = 0;
        self.sounding_len = 0;
        self.latch_armed_new_chord = false;
        self.pattern_step = 0;
        self.seq_cursor = 0;
        self.step_elapsed_samples = 0.0;
        self.gate_is_open = false;
        self.needs_immediate_trigger = false;
        self.last_host_ppq_step = None;
    }

    /// Current active arp pattern step (`0..=15`) or `255` when idle/disabled (for `SharedUiState::arp_step`).
    #[inline]
    pub fn current_step(&self) -> u8 {
        if self.was_enabled && self.pool_len > 0 {
            (self.pattern_step & 0x0F) as u8
        } else {
            255
        }
    }

    /// Bitmask of physically held input MIDI notes (for `SharedUiState::active_input_notes`).
    #[inline]
    pub fn held_note_mask(&self) -> u128 {
        let mut mask = 0u128;
        for i in 0..self.held_len {
            mask |= 1u128 << (self.held[i].note & 0x7F);
        }
        mask
    }

    /// Bitmask of currently sounding arp-generated notes.
    #[inline]
    pub fn sounding_note_mask(&self) -> u128 {
        let mut mask = 0u128;
        for i in 0..self.sounding_len {
            mask |= 1u128 << (self.sounding[i].note & 0x7F);
        }
        mask
    }

    /// Handle incoming MIDI Note On.
    /// When `params.enabled` is false, passes the `NoteOn` straight through in the returned `ArpEvents`.
    pub fn note_on(
        &mut self,
        note: u8,
        velocity: f32,
        channel: u8,
        params: &ArpParams,
    ) -> ArpEvents {
        let mut events = ArpEvents::new();
        let note = note & 0x7F;
        if velocity <= 0.0 || !velocity.is_finite() {
            return self.note_off(note, channel, params);
        }
        let vel = velocity.clamp(0.0, 1.0);
        let entry = HeldNote {
            note,
            velocity: vel,
            channel,
        };

        // Update physically held list (deduplicating same note)
        let prev_held_len = self.held_len;
        Self::upsert_note(&mut self.held, &mut self.held_len, entry);

        if !params.enabled.value() {
            // Also keep pool synced in case user toggles Arp ON while holding keys
            Self::copy_notes(&self.held, self.held_len, &mut self.pool, &mut self.pool_len);
            events.push(ArpEvent::NoteOn {
                note,
                velocity: vel,
                channel,
            });
            return events;
        }

        let latch = params.latch.value();
        self.was_enabled = true;
        self.was_latch = latch;
        let pool_was_empty = self.pool_len == 0;

        if latch {
            if prev_held_len == 0 && self.latch_armed_new_chord {
                // Starting a fresh latched chord after releasing the previous chord
                self.release_all_sounding(&mut events);
                self.pool_len = 0;
                self.latch_armed_new_chord = false;
                self.pattern_step = 0;
                self.seq_cursor = 0;
                self.step_elapsed_samples = 0.0;
                self.needs_immediate_trigger = true;
            }
            Self::upsert_note(&mut self.pool, &mut self.pool_len, entry);
        } else {
            Self::copy_notes(&self.held, self.held_len, &mut self.pool, &mut self.pool_len);
        }

        if pool_was_empty && self.pool_len > 0 {
            self.pattern_step = 0;
            self.seq_cursor = 0;
            self.step_elapsed_samples = 0.0;
            self.last_host_ppq_step = None;
            self.needs_immediate_trigger = true;
        }

        events
    }

    /// Handle incoming MIDI Note Off.
    /// When `params.enabled` is false, passes the `NoteOff` straight through.
    pub fn note_off(&mut self, note: u8, channel: u8, params: &ArpParams) -> ArpEvents {
        let mut events = ArpEvents::new();
        let note = note & 0x7F;
        Self::remove_note(&mut self.held, &mut self.held_len, note);

        if !params.enabled.value() {
            Self::copy_notes(&self.held, self.held_len, &mut self.pool, &mut self.pool_len);
            events.push(ArpEvent::NoteOff { note, channel });
            return events;
        }

        if params.latch.value() {
            if self.held_len == 0 && self.pool_len > 0 {
                self.latch_armed_new_chord = true;
            }
        } else {
            Self::copy_notes(&self.held, self.held_len, &mut self.pool, &mut self.pool_len);
            if self.pool_len == 0 {
                self.release_all_sounding(&mut events);
                self.gate_is_open = false;
                self.needs_immediate_trigger = false;
                self.pattern_step = 0;
                self.seq_cursor = 0;
                self.step_elapsed_samples = 0.0;
            }
        }

        events
    }

    /// Release all held, latched, and sounding notes cleanly.
    pub fn all_notes_off(&mut self) -> ArpEvents {
        let mut events = ArpEvents::new();
        self.release_all_sounding(&mut events);
        self.held_len = 0;
        self.pool_len = 0;
        self.latch_armed_new_chord = false;
        self.gate_is_open = false;
        self.needs_immediate_trigger = false;
        self.pattern_step = 0;
        self.seq_cursor = 0;
        self.step_elapsed_samples = 0.0;
        self.last_host_ppq_step = None;
        events
    }

    /// Advance the arpeggiator by exactly one audio sample.
    ///
    /// Emits any sample-accurate `NoteOff` / `NoteOn` events to feed into the voice allocator.
    #[inline]
    pub fn process_sample(
        &mut self,
        params: &ArpParams,
        shared: &SharedSynthState,
        transport: &TransportInfo,
    ) -> ArpEvents {
        let mut events = ArpEvents::new();
        let enabled = params.enabled.value();
        let latch = params.latch.value();

        // Handle Enabled -> Disabled transition cleanly (no stuck notes!)
        if self.was_enabled && !enabled {
            self.release_all_sounding(&mut events);
            self.was_enabled = false;
            self.gate_is_open = false;
            self.needs_immediate_trigger = false;
            self.latch_armed_new_chord = false;
            // Re-trigger physically held notes directly if desired? Leaving voices silent avoids bursts.
            return events;
        }

        // Handle Disabled -> Enabled transition while keys are already held
        if !self.was_enabled && enabled {
            self.was_enabled = true;
            if self.pool_len == 0 {
                Self::copy_notes(&self.held, self.held_len, &mut self.pool, &mut self.pool_len);
            }
            if self.pool_len > 0 {
                self.pattern_step = 0;
                self.seq_cursor = 0;
                self.step_elapsed_samples = 0.0;
                self.last_host_ppq_step = None;
                self.needs_immediate_trigger = true;
            }
        }

        // Handle Latch ON -> OFF transition
        if self.was_latch && !latch {
            Self::copy_notes(&self.held, self.held_len, &mut self.pool, &mut self.pool_len);
            self.latch_armed_new_chord = false;
            if self.pool_len == 0 {
                self.release_all_sounding(&mut events);
                self.gate_is_open = false;
                self.needs_immediate_trigger = false;
            }
        }
        self.was_latch = latch;

        if !enabled || self.pool_len == 0 {
            return events;
        }

        // Effective BPM: strictly follow transport.tempo_bpm when valid (caller resolves host sync vs internal BPM)
        let bpm = if transport.tempo_bpm.is_finite() && transport.tempo_bpm > 0.0 {
            sanitize_bpm(transport.tempo_bpm)
        } else {
            sanitize_bpm(params.internal_bpm.value())
        };

        let base_step_samples = self.compute_base_step_samples(params, bpm);
        // Handle rapid BPM / rate changes cleanly mid-step without phase jumps or NaNs
        if !self.needs_immediate_trigger
            && self.last_base_step_samples > 1.0
            && (base_step_samples - self.last_base_step_samples).abs() > 0.5
        {
            let ratio = (base_step_samples / self.last_base_step_samples).clamp(0.1, 10.0);
            self.step_elapsed_samples = (self.step_elapsed_samples * ratio).max(0.0);
            self.current_step_duration = (self.current_step_duration * ratio).max(32.0);
            self.current_gate_samples = (self.current_gate_samples * ratio)
                .clamp(8.0, (self.current_step_duration - 1.0).max(8.0));
        }
        self.last_base_step_samples = base_step_samples;
        let swing = params.swing.value().clamp(0.0, 0.75) as f64;
        let gate_ratio = params.gate.value().clamp(0.05, 1.0) as f64;
        let steps_count = params.steps_count.value().clamp(1, 16) as usize;

        // Optional host PPQ grid lock when DAW transport is actively playing and sync is enabled
        if params.sync.value() && transport.playing {
            if let Some(ppq) = transport.ppq_pos {
                if ppq.is_finite() && ppq >= 0.0 {
                    let div_beats = sync_div_to_beats(params.sync_div.value()) as f64;
                    if div_beats > 1e-6 {
                        let grid_step = (ppq / div_beats).floor() as i64;
                        if let Some(prev_step) = self.last_host_ppq_step {
                            if grid_step != prev_step && !self.needs_immediate_trigger {
                                // Snap gently to host beat grid if drifted significantly
                                let frac = (ppq / div_beats) - (grid_step as f64);
                                if frac < 0.05 && self.step_elapsed_samples > base_step_samples * 0.5 {
                                    self.step_elapsed_samples = self.current_step_duration;
                                }
                            }
                        }
                        self.last_host_ppq_step = Some(grid_step);
                    }
                }
            }
        }

        // Trigger first step immediately on first note press
        if self.needs_immediate_trigger {
            self.needs_immediate_trigger = false;
            self.trigger_current_step(
                params,
                shared,
                base_step_samples,
                swing,
                gate_ratio,
                &mut events,
            );
            return events;
        }

        // Advance sample clock
        self.step_elapsed_samples += 1.0;

        // Close gate when gate duration expires
        if self.gate_is_open && self.step_elapsed_samples + 1e-4 >= self.current_gate_samples {
            self.release_all_sounding(&mut events);
            self.gate_is_open = false;
        }

        // Advance to next step when step duration expires
        if self.step_elapsed_samples + 1e-4 >= self.current_step_duration {
            self.step_elapsed_samples -= self.current_step_duration;
            if self.step_elapsed_samples < 0.0 || self.step_elapsed_samples > base_step_samples {
                self.step_elapsed_samples = 0.0;
            }
            self.pattern_step = (self.pattern_step + 1) % steps_count;
            self.seq_cursor = self.seq_cursor.wrapping_add(1);
            self.trigger_current_step(
                params,
                shared,
                base_step_samples,
                swing,
                gate_ratio,
                &mut events,
            );
        }

        events
    }

    #[inline]
    fn compute_base_step_samples(&self, params: &ArpParams, bpm: f32) -> f64 {
        let sr = self.sample_rate as f64;
        let step_seconds = if params.sync.value() {
            let beats = sync_div_to_beats(params.sync_div.value()) as f64;
            beats * (60.0 / bpm as f64)
        } else {
            1.0 / (params.rate.value().clamp(0.25, 60.0) as f64)
        };
        (step_seconds * sr).max(32.0)
    }

    fn trigger_current_step(
        &mut self,
        params: &ArpParams,
        shared: &SharedSynthState,
        base_step_samples: f64,
        swing: f64,
        gate_ratio: f64,
        events: &mut ArpEvents,
    ) {
        // Always cleanly close any lingering sounding notes before triggering next step
        if self.sounding_len > 0 {
            self.release_all_sounding(events);
        }

        // Swing: even steps are lengthened by +swing*0.5, odd steps shortened by -swing*0.5
        // so pairs of steps stay locked to the exact BPM!
        let is_odd = (self.pattern_step & 1) == 1;
        let swing_factor = if is_odd {
            (1.0 - swing * 0.5).max(0.25)
        } else {
            1.0 + swing * 0.5
        };
        self.current_step_duration = (base_step_samples * swing_factor).max(32.0);
        // Keep gate strictly < step duration by at least 2 samples when < 1.0, or full step - 1
        self.current_gate_samples = (self.current_step_duration * gate_ratio)
            .clamp(8.0, self.current_step_duration - 1.0);

        let step_idx = self.pattern_step & 0x0F;
        let (step_enabled, step_offset, step_vel_u8) = shared.read_arp_step(step_idx);
        if !step_enabled {
            self.gate_is_open = false;
            return;
        }

        let mode = params.mode.value();
        let octaves = params.octaves.value().clamp(1, 4) as usize;
        let vel_mode = params.vel_mode.value();
        let pattern_vel = (step_vel_u8 as f32 / 127.0).clamp(0.01, 1.0);

        let seq_len = self.build_sequence(mode, octaves);
        if seq_len == 0 {
            self.gate_is_open = false;
            return;
        }

        match mode {
            ArpMode::Chord => {
                // Trigger all notes in the current octave slice as a synchronized chord
                let base_count = self.pool_len.min(MAX_HELD_NOTES);
                let oct_idx = self.seq_cursor % octaves;
                let oct_shift = (oct_idx as i32) * 12;
                for i in 0..base_count {
                    let src = self.pool[i];
                    let shifted = (src.note as i32 + oct_shift + step_offset as i32).clamp(0, 127) as u8;
                    let vel = Self::resolve_velocity(vel_mode, src.velocity, pattern_vel);
                    self.push_sounding(shifted, src.channel, vel, events);
                }
            }
            ArpMode::Random => {
                let r = next_random_bipolar(&mut self.rng_state) * 0.5 + 0.5;
                let pick = ((r * seq_len as f32) as usize).min(seq_len - 1);
                let src = self.seq_buf[pick];
                let shifted = (src.note as i32 + step_offset as i32).clamp(0, 127) as u8;
                let vel = Self::resolve_velocity(vel_mode, src.velocity, pattern_vel);
                self.push_sounding(shifted, src.channel, vel, events);
            }
            _ => {
                let pick = self.seq_cursor % seq_len;
                let src = self.seq_buf[pick];
                let shifted = (src.note as i32 + step_offset as i32).clamp(0, 127) as u8;
                let vel = Self::resolve_velocity(vel_mode, src.velocity, pattern_vel);
                self.push_sounding(shifted, src.channel, vel, events);
            }
        }

        self.gate_is_open = self.sounding_len > 0;
    }

    /// Build the octave-expanded note sequence in `self.seq_buf` (zero heap allocations).
    fn build_sequence(&mut self, mode: ArpMode, octaves: usize) -> usize {
        let n = self.pool_len.min(MAX_HELD_NOTES);
        if n == 0 {
            return 0;
        }

        // Prepare base list (either press-order for AsPlayed/Chord or sorted ascending)
        let mut base = [HeldNote::default(); MAX_HELD_NOTES];
        base[..n].copy_from_slice(&self.pool[..n]);

        if !matches!(mode, ArpMode::AsPlayed | ArpMode::Chord) {
            // Simple insertion sort (n <= 32, zero alloc, ultra-fast)
            for i in 1..n {
                let key = base[i];
                let mut j = i;
                while j > 0 && base[j - 1].note > key.note {
                    base[j] = base[j - 1];
                    j -= 1;
                }
                base[j] = key;
            }
        }

        // Expand across octaves ascending
        let mut expanded = [HeldNote::default(); MAX_SEQ_NOTES];
        let mut exp_len = 0usize;
        for oct in 0..octaves {
            let shift = (oct as i32) * 12;
            for item in base.iter().take(n) {
                if exp_len >= MAX_SEQ_NOTES {
                    break;
                }
                let shifted_note = (item.note as i32 + shift).clamp(0, 127) as u8;
                expanded[exp_len] = HeldNote {
                    note: shifted_note,
                    velocity: item.velocity,
                    channel: item.channel,
                };
                exp_len += 1;
            }
        }

        if exp_len == 0 {
            return 0;
        }

        let mut out_len = 0usize;
        match mode {
            ArpMode::Up | ArpMode::AsPlayed | ArpMode::Random | ArpMode::Chord => {
                self.seq_buf[..exp_len].copy_from_slice(&expanded[..exp_len]);
                out_len = exp_len;
            }
            ArpMode::Down => {
                for i in 0..exp_len {
                    self.seq_buf[i] = expanded[exp_len - 1 - i];
                }
                out_len = exp_len;
            }
            ArpMode::UpDown => {
                for i in 0..exp_len {
                    self.seq_buf[out_len] = expanded[i];
                    out_len += 1;
                }
                if exp_len > 2 {
                    for i in (1..(exp_len - 1)).rev() {
                        if out_len >= MAX_SEQ_NOTES {
                            break;
                        }
                        self.seq_buf[out_len] = expanded[i];
                        out_len += 1;
                    }
                }
            }
            ArpMode::DownUp => {
                for i in (0..exp_len).rev() {
                    self.seq_buf[out_len] = expanded[i];
                    out_len += 1;
                }
                if exp_len > 2 {
                    for i in 1..(exp_len - 1) {
                        if out_len >= MAX_SEQ_NOTES {
                            break;
                        }
                        self.seq_buf[out_len] = expanded[i];
                        out_len += 1;
                    }
                }
            }
        }

        out_len
    }

    #[inline]
    fn resolve_velocity(mode: ArpVelMode, note_vel: f32, pattern_vel: f32) -> f32 {
        match mode {
            ArpVelMode::NoteVel => note_vel.clamp(0.01, 1.0),
            ArpVelMode::PatternVel => pattern_vel.clamp(0.01, 1.0),
            ArpVelMode::Mixed => (note_vel * pattern_vel).clamp(0.01, 1.0),
        }
    }

    #[inline]
    fn push_sounding(&mut self, note: u8, channel: u8, velocity: f32, events: &mut ArpEvents) {
        if self.sounding_len < MAX_HELD_NOTES {
            self.sounding[self.sounding_len] = SoundingNote { note, channel };
            self.sounding_len += 1;
        }
        events.push(ArpEvent::NoteOn {
            note,
            velocity,
            channel,
        });
    }

    #[inline]
    fn release_all_sounding(&mut self, events: &mut ArpEvents) {
        for i in 0..self.sounding_len {
            let s = self.sounding[i];
            events.push(ArpEvent::NoteOff {
                note: s.note,
                channel: s.channel,
            });
        }
        self.sounding_len = 0;
    }

    #[inline]
    fn upsert_note(buf: &mut [HeldNote; MAX_HELD_NOTES], len: &mut usize, entry: HeldNote) {
        for slot in buf.iter_mut().take(*len) {
            if slot.note == entry.note {
                *slot = entry;
                return;
            }
        }
        if *len < MAX_HELD_NOTES {
            buf[*len] = entry;
            *len += 1;
        }
    }

    #[inline]
    fn remove_note(buf: &mut [HeldNote; MAX_HELD_NOTES], len: &mut usize, note: u8) {
        if let Some(pos) = buf[..*len].iter().position(|h| h.note == note) {
            for i in pos..(*len - 1) {
                buf[i] = buf[i + 1];
            }
            *len -= 1;
        }
    }

    #[inline]
    fn copy_notes(
        src: &[HeldNote; MAX_HELD_NOTES],
        src_len: usize,
        dst: &mut [HeldNote; MAX_HELD_NOTES],
        dst_len: &mut usize,
    ) {
        let count = src_len.min(MAX_HELD_NOTES);
        dst[..count].copy_from_slice(&src[..count]);
        *dst_len = count;
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{
        set_bool_param, set_enum_param, set_float_param, set_int_param, ExtraSynthState, SyncDiv,
    };

    fn collect_triggered_notes(
        arp: &mut Arpeggiator,
        params: &ArpParams,
        shared: &SharedSynthState,
        transport: &TransportInfo,
        samples: usize,
    ) -> Vec<(usize, ArpEvent)> {
        let mut out = Vec::new();
        for s in 0..samples {
            let evs = arp.process_sample(params, shared, transport);
            for &ev in evs.iter() {
                out.push((s, ev));
            }
        }
        out
    }

    #[test]
    fn arp_modes_up_down_updown_downup_asplayed_chord_and_random_work() {
        let sr = 48000.0;
        let shared = SharedSynthState::default();
        let transport = TransportInfo {
            tempo_bpm: 120.0,
            playing: false,
            sample_rate: sr,
            ppq_pos: None,
            pos_samples: None,
        };

        // 1/16 at 120 BPM = 0.125s = 6000 samples per step.
        // 5 steps -> ~26000 samples.
        let run_mode = |mode: ArpMode, octaves: i32| -> Vec<u8> {
            let mut params = ArpParams::default();
            set_bool_param(&mut params.enabled, true);
            set_enum_param(&mut params.mode, mode);
            set_int_param(&mut params.octaves, octaves);
            set_bool_param(&mut params.sync, true);
            set_enum_param(&mut params.sync_div, SyncDiv::D1_16);
            set_float_param(&mut params.internal_bpm, 120.0);

            let mut arp = Arpeggiator::new(sr);
            // Press out of sorted order: E4 (64), C4 (60), G4 (67)
            arp.note_on(64, 0.9, 0, &params);
            arp.note_on(60, 0.8, 0, &params);
            arp.note_on(67, 0.7, 0, &params);

            let evs = collect_triggered_notes(&mut arp, &params, &shared, &transport, 31000);
            evs.into_iter()
                .filter_map(|(_, ev)| match ev {
                    ArpEvent::NoteOn { note, .. } => Some(note),
                    ArpEvent::NoteOff { .. } => None,
                })
                .collect()
        };

        assert_eq!(run_mode(ArpMode::Up, 1)[..4], [60, 64, 67, 60]);
        assert_eq!(run_mode(ArpMode::Down, 1)[..4], [67, 64, 60, 67]);
        assert_eq!(run_mode(ArpMode::UpDown, 1)[..5], [60, 64, 67, 64, 60]);
        assert_eq!(run_mode(ArpMode::DownUp, 1)[..5], [67, 64, 60, 64, 67]);
        assert_eq!(run_mode(ArpMode::AsPlayed, 1)[..4], [64, 60, 67, 64]);
        // Octaves = 2 in Up mode: 60, 64, 67, 72, 76, 79
        assert_eq!(run_mode(ArpMode::Up, 2)[..6], [60, 64, 67, 72, 76, 79]);

        // Chord triggers all 3 notes simultaneously on step 0
        let chord_notes = run_mode(ArpMode::Chord, 1);
        assert!(chord_notes.len() >= 6);
        assert_eq!(&chord_notes[..3], &[64, 60, 67]);

        // Random picks only valid notes from the chord pool
        let rnd_notes = run_mode(ArpMode::Random, 1);
        assert!(rnd_notes.len() >= 4);
        assert!(rnd_notes.iter().all(|n| [60, 64, 67].contains(n)));
    }

    #[test]
    fn arp_swing_gate_pattern_offsets_and_clean_disable_note_off() {
        let sr = 48000.0;
        let mut params = ArpParams::default();
        set_bool_param(&mut params.enabled, true);
        set_enum_param(&mut params.mode, ArpMode::Up);
        set_bool_param(&mut params.sync, true);
        set_enum_param(&mut params.sync_div, SyncDiv::D1_16);
        set_float_param(&mut params.internal_bpm, 120.0);
        set_float_param(&mut params.gate, 0.5);
        set_float_param(&mut params.swing, 0.4); // even step = 1.2 * 6000 = 7200, odd = 0.8 * 6000 = 4800
        set_enum_param(&mut params.vel_mode, ArpVelMode::PatternVel);

        let mut extra = ExtraSynthState::default();
        extra.arp_steps_enabled = 0b1111_1111_1111_1101; // step 1 disabled (rest!)
        extra.arp_step_offsets[0] = 12;
        extra.arp_step_velocities[0] = 64;
        let shared = SharedSynthState::default();
        shared.sync_from_extra(&extra);

        let transport = TransportInfo {
            tempo_bpm: 120.0,
            playing: false,
            sample_rate: sr,
            ppq_pos: None,
            pos_samples: None,
        };

        let mut arp = Arpeggiator::new(sr);
        arp.note_on(60, 1.0, 0, &params);

        let evs = collect_triggered_notes(&mut arp, &params, &shared, &transport, 13000);
        // Step 0 triggers immediately at sample 0 with +12 offset (72) and velocity ~64/127
        match evs[0] {
            (0, ArpEvent::NoteOn { note, velocity, .. }) => {
                assert_eq!(note, 72);
                assert!((velocity - 64.0 / 127.0).abs() < 0.02);
            }
            other => panic!("Expected NoteOn at sample 0, got {other:?}"),
        }
        // Gate closes at 0.5 * 7200 = 3600
        assert!(evs.iter().any(|&(t, ev)| t == 3600 && matches!(ev, ArpEvent::NoteOff { note: 72, .. })));
        // Step 1 (at 7200) is disabled, so NO NoteOn around 7200; step 2 triggers at 7200 + 4800 = 12000!
        assert!(evs.iter().any(|&(t, ev)| t == 12000 && matches!(ev, ArpEvent::NoteOn { note: 60, .. })));

        // Disabling Arp mid-note emits clean NoteOff with zero stuck notes
        set_bool_param(&mut params.enabled, false);
        let disable_evs = arp.process_sample(&params, &shared, &transport);
        assert!(disable_evs
            .iter()
            .any(|ev| matches!(ev, ArpEvent::NoteOff { note: 60, .. })));
        assert_eq!(arp.sounding_note_mask(), 0);
    }

    #[test]
    fn arp_latch_keeps_playing_after_key_release() {
        let sr = 48000.0;
        let mut params = ArpParams::default();
        set_bool_param(&mut params.enabled, true);
        set_bool_param(&mut params.latch, true);
        set_bool_param(&mut params.sync, false);
        set_float_param(&mut params.rate, 10.0); // 4800 samples/step

        let shared = SharedSynthState::default();
        let transport = TransportInfo {
            tempo_bpm: 120.0,
            playing: false,
            sample_rate: sr,
            ppq_pos: None,
            pos_samples: None,
        };

        let mut arp = Arpeggiator::new(sr);
        arp.note_on(60, 0.9, 0, &params);
        arp.note_on(67, 0.9, 0, &params);
        // Release both keys immediately
        arp.note_off(60, 0, &params);
        arp.note_off(67, 0, &params);

        let evs = collect_triggered_notes(&mut arp, &params, &shared, &transport, 15000);
        let note_ons: Vec<u8> = evs
            .iter()
            .filter_map(|(_, ev)| match *ev {
                ArpEvent::NoteOn { note, .. } => Some(note),
                _ => None,
            })
            .collect();
        assert!(
            note_ons.len() >= 3 && note_ons[0] == 60 && note_ons[1] == 67,
            "Latched arp should continue stepping: {note_ons:?}"
        );
    }

    #[test]
    fn arp_follows_transport_bpm_changes_and_scales_step_rate() {
        let sr = 48000.0;
        let mut params = ArpParams::default();
        set_bool_param(&mut params.enabled, true);
        set_enum_param(&mut params.mode, ArpMode::Up);
        set_bool_param(&mut params.sync, true);
        set_enum_param(&mut params.sync_div, SyncDiv::D1_16);
        set_float_param(&mut params.internal_bpm, 120.0);

        let shared = SharedSynthState::default();

        let count_note_ons = |bpm: f32, playing: bool| -> usize {
            let transport = TransportInfo {
                tempo_bpm: bpm,
                playing,
                sample_rate: sr,
                ppq_pos: None,
                pos_samples: None,
            };
            let mut arp = Arpeggiator::new(sr);
            arp.note_on(60, 0.8, 0, &params);
            arp.note_on(64, 0.8, 0, &params);
            let evs = collect_triggered_notes(&mut arp, &params, &shared, &transport, 48000);
            evs.iter()
                .filter(|(_, ev)| matches!(ev, ArpEvent::NoteOn { .. }))
                .count()
        };

        // 1/16 note at 90 BPM = 6 steps/sec -> 6 or 7 triggers in 1.0 s (incl. t=0)
        // 1/16 note at 160 BPM = 10.67 steps/sec -> 11 triggers in 1.0 s (incl. t=0)
        // Verify this works even when DAW transport is stopped (playing=false)
        let n_90 = count_note_ons(90.0, false);
        let n_160 = count_note_ons(160.0, false);
        assert!(
            n_90 >= 6 && n_90 <= 7,
            "Expected 6..7 arp triggers in 1s at 90 BPM, got {n_90}"
        );
        assert!(
            n_160 >= 10 && n_160 <= 12,
            "Expected 10..12 arp triggers in 1s at 160 BPM, got {n_160}"
        );

        // Rapid mid-stream BPM swings (90 -> 160 -> 45 -> 240 -> NaN) remain valid & bounded
        let mut arp = Arpeggiator::new(sr);
        arp.note_on(60, 0.9, 0, &params);
        arp.note_on(67, 0.9, 0, &params);
        let bpms = [90.0_f32, 160.0, 45.0, 240.0, f32::NAN, 128.0];
        let mut triggers = 0usize;
        for (i, &b) in bpms.iter().enumerate() {
            let t = TransportInfo {
                tempo_bpm: b,
                playing: i % 2 == 0,
                sample_rate: sr,
                ppq_pos: None,
                pos_samples: None,
            };
            for _ in 0..8000 {
                let evs = arp.process_sample(&params, &shared, &t);
                for ev in evs.iter() {
                    if let ArpEvent::NoteOn { velocity, .. } = ev {
                        assert!(velocity.is_finite() && *velocity > 0.0);
                        triggers += 1;
                    }
                }
            }
        }
        assert!(triggers >= 6);
        let off = arp.all_notes_off();
        assert!(!off.is_empty() || arp.sounding_note_mask() == 0);
        assert_eq!(arp.sounding_note_mask(), 0);
    }
}
