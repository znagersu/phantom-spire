use nih_plug::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{
    AtomicBool, AtomicI8, AtomicU16, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering,
};
use std::sync::{Arc, Mutex};

pub mod dsp;
pub mod params;
pub mod presets;
mod presets_factory;
pub mod ui;
pub mod updater;

use dsp::{SynthEngine, TransportInfo};
use params::{ExtraSynthState, PhantomSpireParams};
use presets::PresetManager;
use updater::UpdaterHandle;

/// Returns the configured screenshot output path when `PHANTOM_SPIRE_SCREENSHOT_OUT` is set.
pub fn env_screenshot_out() -> Option<PathBuf> {
    std::env::var_os("PHANTOM_SPIRE_SCREENSHOT_OUT")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Returns the requested initial UI tab (`synth`, `fx`, `mod`) when `PHANTOM_SPIRE_UI_TAB` is set.
pub fn env_ui_tab() -> Option<String> {
    std::env::var("PHANTOM_SPIRE_UI_TAB")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

// ============================================================================
// Lock-Free UI Audition SPSC Ring Queue
// ============================================================================

const AUDITION_QUEUE_CAP: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditionEvent {
    NoteOn { note: u8, velocity_u8: u8 },
    NoteOff { note: u8 },
    AllOff,
}

impl AuditionEvent {
    #[inline]
    fn encode(self) -> u32 {
        match self {
            AuditionEvent::NoteOn { note, velocity_u8 } => {
                1u32 | ((note as u32) << 8) | ((velocity_u8 as u32) << 16)
            }
            AuditionEvent::NoteOff { note } => 2u32 | ((note as u32) << 8),
            AuditionEvent::AllOff => 3u32,
        }
    }

    #[inline]
    fn decode(raw: u32) -> Option<Self> {
        let kind = (raw & 0xFF) as u8;
        let note = ((raw >> 8) & 0x7F) as u8;
        let velocity_u8 = ((raw >> 16) & 0xFF) as u8;
        match kind {
            1 => Some(AuditionEvent::NoteOn { note, velocity_u8 }),
            2 => Some(AuditionEvent::NoteOff { note }),
            3 => Some(AuditionEvent::AllOff),
            _ => None,
        }
    }
}

pub struct AuditionEventQueue {
    slots: [AtomicU32; AUDITION_QUEUE_CAP],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl Default for AuditionEventQueue {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| AtomicU32::new(0)),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }
}

impl AuditionEventQueue {
    pub fn push(&self, ev: AuditionEvent) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) & (AUDITION_QUEUE_CAP - 1);
        let tail = self.tail.load(Ordering::Acquire);
        if next == tail {
            return false;
        }
        self.slots[head].store(ev.encode(), Ordering::Relaxed);
        self.head.store(next, Ordering::Release);
        true
    }

    pub fn pop(&self) -> Option<AuditionEvent> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let raw = self.slots[tail].load(Ordering::Relaxed);
        let next = (tail + 1) & (AUDITION_QUEUE_CAP - 1);
        self.tail.store(next, Ordering::Release);
        AuditionEvent::decode(raw)
    }
}

// ============================================================================
// Lock-Free Audio <-> UI Bridge
// ============================================================================

pub struct SharedUiState {
    pub active_input_notes_low: AtomicU64,
    pub active_input_notes_high: AtomicU64,
    pub active_output_notes_low: AtomicU64,
    pub active_output_notes_high: AtomicU64,
    pub active_voice_count: AtomicU32,
    pub peak_l_bits: AtomicU32,
    pub peak_r_bits: AtomicU32,
    pub last_note: AtomicI8,
    pub last_param_seq: AtomicU32,
    pub last_param_bytes: [AtomicU64; 4],
    pub last_param_len: AtomicU8,
    pub last_param_val_bits: AtomicU32,
    pub ui_audition_note_queue: AuditionEventQueue,
    pub env_levels: [AtomicU32; 4],
    pub lfo_phases: [AtomicU32; 4],
    pub arp_step: AtomicU8,
    pub stp_steps: [AtomicU8; 2],
    pub active_bpm_bits: AtomicU32,
    pub host_tempo_connected: AtomicBool,
}

impl Default for SharedUiState {
    fn default() -> Self {
        Self {
            active_input_notes_low: AtomicU64::new(0),
            active_input_notes_high: AtomicU64::new(0),
            active_output_notes_low: AtomicU64::new(0),
            active_output_notes_high: AtomicU64::new(0),
            active_voice_count: AtomicU32::new(0),
            peak_l_bits: AtomicU32::new(0.0_f32.to_bits()),
            peak_r_bits: AtomicU32::new(0.0_f32.to_bits()),
            last_note: AtomicI8::new(-1),
            last_param_seq: AtomicU32::new(0),
            last_param_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            last_param_len: AtomicU8::new(0),
            last_param_val_bits: AtomicU32::new(0.0_f32.to_bits()),
            ui_audition_note_queue: AuditionEventQueue::default(),
            env_levels: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
            lfo_phases: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
            arp_step: AtomicU8::new(255),
            stp_steps: [AtomicU8::new(255), AtomicU8::new(255)],
            active_bpm_bits: AtomicU32::new(120.0_f32.to_bits()),
            host_tempo_connected: AtomicBool::new(false),
        }
    }
}

impl SharedUiState {
    #[inline]
    pub fn set_input_mask(&self, mask: u128) {
        self.active_input_notes_low
            .store(mask as u64, Ordering::Relaxed);
        self.active_input_notes_high
            .store((mask >> 64) as u64, Ordering::Relaxed);
    }

    #[inline]
    pub fn get_input_mask(&self) -> u128 {
        let lo = self.active_input_notes_low.load(Ordering::Relaxed) as u128;
        let hi = self.active_input_notes_high.load(Ordering::Relaxed) as u128;
        lo | (hi << 64)
    }

    #[inline]
    pub fn set_output_mask(&self, mask: u128) {
        self.active_output_notes_low
            .store(mask as u64, Ordering::Relaxed);
        self.active_output_notes_high
            .store((mask >> 64) as u64, Ordering::Relaxed);
    }

    #[inline]
    pub fn get_output_mask(&self) -> u128 {
        let lo = self.active_output_notes_low.load(Ordering::Relaxed) as u128;
        let hi = self.active_output_notes_high.load(Ordering::Relaxed) as u128;
        lo | (hi << 64)
    }

    #[inline]
    pub fn set_peaks(&self, l: f32, r: f32) {
        let safe_l = if l.is_finite() {
            l.clamp(0.0, 4.0)
        } else {
            0.0
        };
        let safe_r = if r.is_finite() {
            r.clamp(0.0, 4.0)
        } else {
            0.0
        };
        self.peak_l_bits.store(safe_l.to_bits(), Ordering::Relaxed);
        self.peak_r_bits.store(safe_r.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn get_peaks(&self) -> (f32, f32) {
        let l = f32::from_bits(self.peak_l_bits.load(Ordering::Relaxed));
        let r = f32::from_bits(self.peak_r_bits.load(Ordering::Relaxed));
        (
            if l.is_finite() { l.clamp(0.0, 4.0) } else { 0.0 },
            if r.is_finite() { r.clamp(0.0, 4.0) } else { 0.0 },
        )
    }

    pub fn set_last_param_label(&self, label: &str) {
        self.set_last_param_label_with_val(label, 0.0);
    }

    pub fn set_last_param_label_with_val(&self, label: &str, val: f32) {
        let mut bytes = [0u8; 32];
        let mut len = label.len().min(32);
        while len > 0 && !label.is_char_boundary(len) {
            len -= 1;
        }
        bytes[..len].copy_from_slice(&label.as_bytes()[..len]);

        // Odd sequence number signals writer in progress
        let seq = self.last_param_seq.load(Ordering::Relaxed);
        let odd_seq = (seq & !1).wrapping_add(1);
        self.last_param_seq.store(odd_seq, Ordering::Release);
        std::sync::atomic::fence(Ordering::Release);

        for (i, slot) in self.last_param_bytes.iter().enumerate() {
            let chunk = u64::from_le_bytes(bytes[i * 8..(i + 1) * 8].try_into().unwrap());
            slot.store(chunk, Ordering::Relaxed);
        }
        self.last_param_len.store(len as u8, Ordering::Relaxed);
        let safe_val = if val.is_finite() { val } else { 0.0 };
        self.last_param_val_bits
            .store(safe_val.to_bits(), Ordering::Relaxed);

        std::sync::atomic::fence(Ordering::Release);
        self.last_param_seq
            .store(odd_seq.wrapping_add(1), Ordering::Release);
    }

    pub fn get_last_param_label(&self) -> String {
        for _ in 0..64 {
            let s1 = self.last_param_seq.load(Ordering::Acquire);
            if s1 & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let len = (self.last_param_len.load(Ordering::Relaxed) as usize).min(32);
            let mut bytes = [0u8; 32];
            for (i, slot) in self.last_param_bytes.iter().enumerate() {
                let chunk = slot.load(Ordering::Relaxed).to_le_bytes();
                bytes[i * 8..(i + 1) * 8].copy_from_slice(&chunk);
            }
            std::sync::atomic::fence(Ordering::Acquire);
            let s2 = self.last_param_seq.load(Ordering::Acquire);
            if s1 == s2 {
                return String::from_utf8_lossy(&bytes[..len]).into_owned();
            }
        }
        String::new()
    }

    pub fn push_audition_note_on(&self, note: u8, velocity_u8: u8) -> bool {
        self.ui_audition_note_queue.push(AuditionEvent::NoteOn {
            note: note & 0x7F,
            velocity_u8,
        })
    }

    pub fn push_audition_note_off(&self, note: u8) -> bool {
        self.ui_audition_note_queue
            .push(AuditionEvent::NoteOff { note: note & 0x7F })
    }

    pub fn push_audition_all_off(&self) -> bool {
        self.ui_audition_note_queue.push(AuditionEvent::AllOff)
    }

    pub fn pop_audition_event(&self) -> Option<AuditionEvent> {
        self.ui_audition_note_queue.pop()
    }

    #[inline]
    pub fn set_env_level(&self, idx: usize, level: f32) {
        if idx < 4 {
            let safe = if level.is_finite() {
                level.clamp(0.0, 1.0)
            } else {
                0.0
            };
            self.env_levels[idx].store(safe.to_bits(), Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn get_env_level(&self, idx: usize) -> f32 {
        if idx >= 4 {
            return 0.0;
        }
        let v = f32::from_bits(self.env_levels[idx].load(Ordering::Relaxed));
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    #[inline]
    pub fn set_lfo_phase(&self, idx: usize, phase: f32) {
        if idx < 4 {
            let safe = if phase.is_finite() {
                phase.rem_euclid(1.0)
            } else {
                0.0
            };
            self.lfo_phases[idx].store(safe.to_bits(), Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn get_lfo_phase(&self, idx: usize) -> f32 {
        if idx >= 4 {
            return 0.0;
        }
        let v = f32::from_bits(self.lfo_phases[idx].load(Ordering::Relaxed));
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    #[inline]
    pub fn set_bpm_state(&self, bpm: f32, host_connected: bool) {
        let safe = if bpm.is_finite() {
            bpm.clamp(40.0, 260.0)
        } else {
            120.0
        };
        self.active_bpm_bits.store(safe.to_bits(), Ordering::Relaxed);
        self.host_tempo_connected
            .store(host_connected, Ordering::Relaxed);
    }

    #[inline]
    pub fn get_active_bpm(&self) -> f32 {
        let v = f32::from_bits(self.active_bpm_bits.load(Ordering::Relaxed));
        if v.is_finite() {
            v.clamp(40.0, 260.0)
        } else {
            120.0
        }
    }

    #[inline]
    pub fn is_host_tempo_connected(&self) -> bool {
        self.host_tempo_connected.load(Ordering::Relaxed)
    }
}

// ============================================================================
// Lock-Free Mirror of ExtraSynthState for Audio Thread
// ============================================================================

pub struct SharedSynthState {
    pub stepper_steps: [[AtomicU32; 16]; 2],
    pub stepper_ties: [AtomicU16; 2],
    pub arp_steps_enabled: AtomicU16,
    pub arp_step_offsets: [AtomicI8; 16],
    pub arp_step_velocities: [AtomicU8; 16],
    pub mod_sources: [AtomicU8; 15],
    pub mod_targets: [AtomicU8; 15],
    pub microtuning_preset: AtomicU8,
    pub microtuning_cents: [AtomicU32; 12],
    pub revision: AtomicU64,
}

impl Default for SharedSynthState {
    fn default() -> Self {
        let state = Self {
            stepper_steps: [
                std::array::from_fn(|_| AtomicU32::new(0.5_f32.to_bits())),
                std::array::from_fn(|_| AtomicU32::new(0.5_f32.to_bits())),
            ],
            stepper_ties: [AtomicU16::new(0), AtomicU16::new(0)],
            arp_steps_enabled: AtomicU16::new(0xFFFF),
            arp_step_offsets: std::array::from_fn(|_| AtomicI8::new(0)),
            arp_step_velocities: std::array::from_fn(|_| AtomicU8::new(100)),
            mod_sources: std::array::from_fn(|_| AtomicU8::new(0)),
            mod_targets: std::array::from_fn(|_| AtomicU8::new(0)),
            microtuning_preset: AtomicU8::new(0),
            microtuning_cents: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
            revision: AtomicU64::new(0),
        };
        state.sync_from_extra(&ExtraSynthState::default());
        state
    }
}

impl SharedSynthState {
    pub fn sync_from_extra(&self, extra: &ExtraSynthState) {
        let mut changed = false;
        for s in 0..2 {
            for step in 0..16 {
                let v = extra.stepper_steps[s][step];
                let safe = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.5 };
                let bits = safe.to_bits();
                if self.stepper_steps[s][step].swap(bits, Ordering::Relaxed) != bits {
                    changed = true;
                }
            }
            let tie = extra.stepper_ties[s];
            if self.stepper_ties[s].swap(tie, Ordering::Relaxed) != tie {
                changed = true;
            }
        }
        if self
            .arp_steps_enabled
            .swap(extra.arp_steps_enabled, Ordering::Relaxed)
            != extra.arp_steps_enabled
        {
            changed = true;
        }
        for step in 0..16 {
            let off = extra.arp_step_offsets[step];
            if self.arp_step_offsets[step].swap(off, Ordering::Relaxed) != off {
                changed = true;
            }
            let vel = extra.arp_step_velocities[step];
            if self.arp_step_velocities[step].swap(vel, Ordering::Relaxed) != vel {
                changed = true;
            }
        }
        for slot in 0..15 {
            let src = extra.mod_sources[slot];
            if self.mod_sources[slot].swap(src, Ordering::Relaxed) != src {
                changed = true;
            }
            let tgt = extra.mod_targets[slot];
            if self.mod_targets[slot].swap(tgt, Ordering::Relaxed) != tgt {
                changed = true;
            }
        }
        if self
            .microtuning_preset
            .swap(extra.microtuning_preset, Ordering::Relaxed)
            != extra.microtuning_preset
        {
            changed = true;
        }
        for pc in 0..12 {
            let c = extra.microtuning_cents[pc];
            let safe = if c.is_finite() { c } else { 0.0 };
            let bits = safe.to_bits();
            if self.microtuning_cents[pc].swap(bits, Ordering::Relaxed) != bits {
                changed = true;
            }
        }
        if changed {
            self.revision.fetch_add(1, Ordering::Release);
        }
    }

    #[inline]
    pub fn read_stepper_step(&self, idx: usize, step: usize) -> f32 {
        if idx >= 2 || step >= 16 {
            return 0.5;
        }
        let v = f32::from_bits(self.stepper_steps[idx][step].load(Ordering::Relaxed));
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.5
        }
    }

    #[inline]
    pub fn read_arp_step(&self, step: usize) -> (bool, i8, u8) {
        if step >= 16 {
            return (false, 0, 100);
        }
        let enabled = (self.arp_steps_enabled.load(Ordering::Relaxed) & (1u16 << step)) != 0;
        let offset = self.arp_step_offsets[step].load(Ordering::Relaxed);
        let vel = self.arp_step_velocities[step].load(Ordering::Relaxed);
        (enabled, offset, vel)
    }

    #[inline]
    pub fn read_mod_route(&self, slot: usize) -> (u8, u8) {
        if slot >= 15 {
            return (0, 0);
        }
        (
            self.mod_sources[slot].load(Ordering::Relaxed),
            self.mod_targets[slot].load(Ordering::Relaxed),
        )
    }

    #[inline]
    pub fn read_microtuning_cents(&self, pc: usize) -> f32 {
        if pc >= 12 {
            return 0.0;
        }
        let c = f32::from_bits(self.microtuning_cents[pc].load(Ordering::Relaxed));
        if c.is_finite() {
            c
        } else {
            0.0
        }
    }
}

// ============================================================================
// PhantomSpire Plugin
// ============================================================================

pub struct PhantomSpire {
    params: Arc<PhantomSpireParams>,
    ui_state: Arc<SharedUiState>,
    shared_synth: Arc<SharedSynthState>,
    synth: SynthEngine,
    presets: Arc<Mutex<PresetManager>>,
    updater: UpdaterHandle,
}

impl Default for PhantomSpire {
    fn default() -> Self {
        let params = Arc::new(PhantomSpireParams::default());
        let shared_synth = Arc::new(SharedSynthState::default());
        if let Ok(extra) = params.extra_state.lock() {
            shared_synth.sync_from_extra(&extra);
        }
        Self {
            params,
            ui_state: Arc::new(SharedUiState::default()),
            shared_synth,
            synth: SynthEngine::new(44100.0),
            presets: Arc::new(Mutex::new(PresetManager::new())),
            updater: UpdaterHandle::default(),
        }
    }
}

impl Plugin for PhantomSpire {
    const NAME: &'static str = "Phantom Spire";
    const VENDOR: &'static str = "Endio Audio";
    const URL: &'static str = "https://github.com/znagersu/phantom-spire";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        self.updater.trigger_startup_check_if_enabled();
        ui::create(
            self.params.clone(),
            self.ui_state.clone(),
            self.shared_synth.clone(),
            self.presets.clone(),
            self.updater.clone(),
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.synth.set_sample_rate(buffer_config.sample_rate);
        if let Ok(extra) = self.params.extra_state.lock() {
            self.shared_synth.sync_from_extra(&extra);
        }
        true
    }

    fn reset(&mut self) {
        self.synth.reset();
        self.synth.sync_ui_state(&self.ui_state, 0.0, 0.0);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let num_samples = buffer.samples();
        if num_samples == 0 {
            return ProcessStatus::Normal;
        }

        // Keep lock-free synth mirror fresh when host restores state
        if let Ok(extra) = self.params.extra_state.try_lock() {
            self.shared_synth.sync_from_extra(&extra);
        }

        // Drain UI audition events
        let mut voice_settings = self
            .synth
            .make_voice_settings(&self.params, &self.shared_synth);
        while let Some(ev) = self.ui_state.pop_audition_event() {
            match ev {
                AuditionEvent::NoteOn { note, velocity_u8 } => {
                    self.synth.note_on_with_params(
                        note,
                        velocity_u8 as f32 / 127.0,
                        0,
                        &self.params,
                        &voice_settings,
                    );
                }
                AuditionEvent::NoteOff { note } => {
                    self.synth
                        .note_off_with_params(note, 0, &self.params, &voice_settings);
                }
                AuditionEvent::AllOff => {
                    self.synth.all_notes_off();
                }
            }
        }

        let host_tempo = context.transport().tempo;
        let host_connected = host_tempo.is_some();
        let effective_bpm = if self.params.global.host_bpm_sync.value() {
            host_tempo.unwrap_or(self.params.arp.internal_bpm.value() as f64)
        } else {
            self.params.arp.internal_bpm.value() as f64
        }
        .clamp(40.0, 260.0) as f32;
        self.ui_state.set_bpm_state(effective_bpm, host_connected);

        let transport = TransportInfo {
            tempo_bpm: effective_bpm,
            playing: context.transport().playing,
            sample_rate: context.transport().sample_rate,
            ppq_pos: context.transport().pos_beats(),
            pos_samples: context.transport().pos_samples(),
        };

        let mut next_ev = context.next_event();
        let mut peak_l = 0.0_f32;
        let mut peak_r = 0.0_f32;

        for (sample_idx, channel_samples) in buffer.iter_samples().enumerate() {
            if sample_idx > 0 && (sample_idx & 15) == 0 {
                voice_settings = self
                    .synth
                    .make_voice_settings(&self.params, &self.shared_synth);
            }
            let s_u32 = sample_idx as u32;
            while let Some(ev) = next_ev.as_ref() {
                if ev.timing() > s_u32 {
                    break;
                }
                let ev = next_ev.take().unwrap();
                self.handle_note_event(ev, &mut voice_settings);
                next_ev = context.next_event();
            }

            let gain = self.params.master.volume.smoothed.next();
            let (out_l, out_r) = self.synth.process_sample_full(
                &self.params,
                &self.shared_synth,
                &transport,
                &voice_settings,
                gain,
            );
            peak_l = peak_l.max(out_l.abs());
            peak_r = peak_r.max(out_r.abs());

            let mut ch_iter = channel_samples.into_iter();
            if let Some(l) = ch_iter.next() {
                *l = out_l;
            }
            if let Some(r) = ch_iter.next() {
                *r = out_r;
            }
            for extra_ch in ch_iter {
                *extra_ch = 0.0;
            }
        }

        while let Some(ev) = next_ev {
            self.handle_note_event(ev, &mut voice_settings);
            next_ev = context.next_event();
        }

        self.synth.sync_ui_state(&self.ui_state, peak_l, peak_r);

        if self.synth.active_voice_count() > 0
            || self.synth.is_arp_active()
            || peak_l > 1e-5
            || peak_r > 1e-5
        {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Normal
        }
    }
}

impl PhantomSpire {
    fn handle_note_event(
        &mut self,
        ev: PluginNoteEvent<Self>,
        voice_settings: &mut dsp::voice::VoiceRenderSettings,
    ) {
        match ev {
            NoteEvent::NoteOn {
                channel,
                note,
                velocity,
                ..
            } => {
                if velocity <= 0.0 {
                    self.synth
                        .note_off_with_params(note, channel, &self.params, voice_settings);
                } else {
                    self.synth.note_on_with_params(
                        note,
                        velocity,
                        channel,
                        &self.params,
                        voice_settings,
                    );
                }
            }
            NoteEvent::NoteOff { channel, note, .. } | NoteEvent::Choke { channel, note, .. } => {
                self.synth
                    .note_off_with_params(note, channel, &self.params, voice_settings);
            }
            NoteEvent::MidiCC { cc, value, .. } => {
                if cc == 1 {
                    self.synth.set_mod_wheel(value);
                } else if cc == 64 {
                    self.synth.set_sustain_pedal_with_params(value >= 0.5);
                } else if cc == 120 || cc == 123 {
                    self.synth.all_notes_off();
                }
            }
            NoteEvent::MidiPitchBend { value, .. } => {
                self.synth.set_pitch_bend(value * 2.0 - 1.0);
                *voice_settings = self
                    .synth
                    .make_voice_settings(&self.params, &self.shared_synth);
            }
            NoteEvent::MidiChannelPressure { pressure, .. } => {
                self.synth.set_channel_pressure(pressure);
            }
            _ => {}
        }
    }
}

impl ClapPlugin for PhantomSpire {
    const CLAP_ID: &'static str = "com.endio-audio.phantom-spire";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Persona 5 inspired Spire-style polyphonic synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for PhantomSpire {
    const VST3_CLASS_ID: [u8; 16] = *b"Ph4nt0mSp1r3P5!!";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(PhantomSpire);
nih_export_vst3!(PhantomSpire);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_ui_state_audition_queue_push_pop_and_wrap() {
        let ui = SharedUiState::default();
        assert!(ui.pop_audition_event().is_none());

        // Fill capacity - 1 = 63 items
        for i in 0..63 {
            assert!(ui.push_audition_note_on(i as u8, 100));
        }
        // 64th push fails cleanly when full
        assert!(!ui.push_audition_note_on(99, 100));

        // Drain half and push again to exercise wrap-around
        for i in 0..32 {
            assert_eq!(
                ui.pop_audition_event(),
                Some(AuditionEvent::NoteOn {
                    note: i as u8,
                    velocity_u8: 100
                })
            );
        }
        for i in 0..20 {
            assert!(ui.push_audition_note_off(i as u8));
        }

        let mut popped = 0;
        while ui.pop_audition_event().is_some() {
            popped += 1;
        }
        assert_eq!(popped, 31 + 20);
    }

    #[test]
    fn shared_ui_state_masks_peaks_and_seqlock_label() {
        let ui = SharedUiState::default();
        let mask = (1u128 << 3) | (1u128 << 60) | (1u128 << 127);
        ui.set_input_mask(mask);
        ui.set_output_mask(mask);
        assert_eq!(ui.get_input_mask(), mask);
        assert_eq!(ui.get_output_mask(), mask);

        ui.set_peaks(f32::NAN, 99.0);
        assert_eq!(ui.get_peaks(), (0.0, 4.0));

        ui.set_last_param_label("OSC1: SawPWM (50%)");
        assert_eq!(ui.get_last_param_label(), "OSC1: SawPWM (50%)");

        // Long UTF-8 truncation safety
        ui.set_last_param_label("VeryLongParameterLabelThatExceedsThirtyTwoBytes12345");
        let read = ui.get_last_param_label();
        assert!(read.len() <= 32);
        assert!(read.starts_with("VeryLongParameterLabel"));
    }

    #[test]
    fn shared_synth_state_syncs_from_extra() {
        let shared = SharedSynthState::default();
        let initial_rev = shared.revision.load(Ordering::Acquire);

        let mut extra = ExtraSynthState::default();
        extra.stepper_steps[0][5] = 0.88;
        extra.arp_steps_enabled = 0b1010_0101;
        extra.arp_step_offsets[2] = 12;
        extra.arp_step_velocities[2] = 111;
        extra.mod_sources[3] = 7;
        extra.mod_targets[3] = 19;
        extra.microtuning_cents[1] = -24.5;

        shared.sync_from_extra(&extra);
        assert!(shared.revision.load(Ordering::Acquire) > initial_rev);
        assert!((shared.read_stepper_step(0, 5) - 0.88).abs() < 1e-6);
        assert_eq!(shared.read_arp_step(2), (true, 12, 111));
        assert_eq!(shared.read_arp_step(1), (false, 0, 100));
        assert_eq!(shared.read_mod_route(3), (7, 19));
        assert!((shared.read_microtuning_cents(1) - (-24.5)).abs() < 1e-6);
    }

    #[test]
    fn plugin_default_does_not_hang_or_trigger_update_check() {
        let plugin = PhantomSpire::default();
        assert_eq!(plugin.updater.status(), updater::UpdateStatus::Idle);
        assert_eq!(plugin.params.editor_state.size(), (1220, 760));
    }
}
