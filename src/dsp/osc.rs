//! Multi-mode polyphonic oscillators with band-limited wavetables, PolyBLEP
//! anti-aliasing, 7 Spire synthesis modes, 1..9-voice stereo unison, and
//! dual-filter routing balance.

use crate::params::{OscMode, OscParams, OscWaveform, UnisonMode};
use std::f32::consts::{FRAC_PI_4, PI, SQRT_2, TAU};
use std::sync::OnceLock;

pub const NUM_OSCS: usize = 4;
pub const MAX_UNISON: usize = 9;

const TABLE_SIZE: usize = 512;
const NUM_MIPS: usize = 10;
const NUM_WAVEFORMS: usize = 16;

/// Flush subnormals/denormals and non-finite values to zero.
#[inline]
pub fn flush_denormal(x: f32) -> f32 {
    if !x.is_finite() || x.abs() < 1e-24 {
        0.0
    } else {
        x
    }
}

/// Fast smooth saturator bounded strictly in (-1, 1).
#[inline]
pub fn fast_tanh(x: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let clamped = x.clamp(-9.0, 9.0);
    let x2 = clamped * clamped;
    clamped * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// PolyBLEP anti-aliasing residual for step discontinuities at phase `t == 0`.
#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 1e-7 {
        return 0.0;
    }
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Fast deterministic xorshift32 PRNG returning bipolar noise in [-1.0, 1.0].
#[inline]
fn next_noise_bipolar(state: &mut u32) -> f32 {
    let mut x = *state;
    if x == 0 {
        x = 0x6D2B_79F5;
    }
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    // Map u32 -> [-1.0, 1.0]
    (x as f32) * (2.0 / u32::MAX as f32) - 1.0
}

#[inline]
fn next_rand_unit(state: &mut u32) -> f32 {
    0.5 * (next_noise_bipolar(state) + 1.0)
}

#[inline]
pub fn waveform_index(w: OscWaveform) -> usize {
    match w {
        OscWaveform::Sin => 0,
        OscWaveform::Tri => 1,
        OscWaveform::Saw => 2,
        OscWaveform::Square => 3,
        OscWaveform::Pulse => 4,
        OscWaveform::SuperSaw => 5,
        OscWaveform::Organ => 6,
        OscWaveform::Bell => 7,
        OscWaveform::Formant => 8,
        OscWaveform::Metallic => 9,
        OscWaveform::Acid => 10,
        OscWaveform::Bass => 11,
        OscWaveform::Choir => 12,
        OscWaveform::Strings => 13,
        OscWaveform::Hollow => 14,
        OscWaveform::Sub => 15,
    }
}

/// Band-limited mipmapped wavetable bank shared read-only across all voices.
/// Initialized once via `WavetableBank::global()` (call during engine init to
/// guarantee zero audio-thread allocation).
pub struct WavetableBank {
    tables: [[[f32; TABLE_SIZE + 1]; NUM_MIPS]; NUM_WAVEFORMS],
}

static WAVETABLE_BANK: OnceLock<Box<WavetableBank>> = OnceLock::new();

impl WavetableBank {
    /// Get or initialize the global read-only wavetable bank.
    pub fn global() -> &'static Self {
        WAVETABLE_BANK.get_or_init(Self::build_boxed)
    }

    fn build_boxed() -> Box<Self> {
        let mut bank = Box::new(Self {
            tables: [[[0.0; TABLE_SIZE + 1]; NUM_MIPS]; NUM_WAVEFORMS],
        });

        let mip_max_harmonics: [usize; NUM_MIPS] = [255, 128, 64, 32, 16, 8, 4, 2, 1, 1];

        for wf in 0..NUM_WAVEFORMS {
            for (mip, &max_h) in mip_max_harmonics.iter().enumerate() {
                let table = &mut bank.tables[wf][mip];
                Self::generate_table(wf, max_h, table);
            }
        }
        bank
    }

    fn harmonic_spec(wf: usize, k: usize) -> (f32, f32) {
        let kf = k as f32;
        match wf {
            // 0: Sin
            0 => {
                if k == 1 {
                    (1.0, 0.0)
                } else {
                    (0.0, 0.0)
                }
            }
            // 1: Tri
            1 => {
                if k % 2 == 1 {
                    let sign = if ((k - 1) / 2) % 2 == 0 { 1.0 } else { -1.0 };
                    (sign / (kf * kf), 0.0)
                } else {
                    (0.0, 0.0)
                }
            }
            // 2: Saw
            2 => (1.0 / kf, 0.0),
            // 3: Square
            3 => {
                if k % 2 == 1 {
                    (1.0 / kf, 0.0)
                } else {
                    (0.0, 0.0)
                }
            }
            // 4: Pulse (25% duty cycle)
            4 => ((kf * PI * 0.25).sin() / kf, 0.0),
            // 5: SuperSaw (bright phase-dispersed harmonic stack)
            5 => {
                let amp = (1.0 / kf.powf(0.82)) * (1.0 + 0.25 * (kf * 0.7).sin());
                let phase = (kf * kf * 0.37) % TAU;
                (amp, phase)
            }
            // 6: Organ (drawbar partials)
            6 => match k {
                1 => (1.0, 0.0),
                2 => (0.85, 0.0),
                3 => (0.65, 0.0),
                4 => (0.55, 0.0),
                6 => (0.35, 0.0),
                8 => (0.30, 0.0),
                12 => (0.15, 0.0),
                _ => (0.0, 0.0),
            },
            // 7: Bell (metallic bright partial ratios)
            7 => match k {
                1 => (0.90, 0.0),
                2 => (0.35, 0.4),
                3 => (0.55, 1.1),
                5 => (0.48, 0.2),
                7 => (0.42, 2.0),
                11 => (0.32, 0.9),
                14 => (0.24, 1.7),
                19 => (0.16, 0.5),
                23 => (0.11, 2.5),
                _ => (0.0, 0.0),
            },
            // 8: Formant (vocal spectral peaks at harmonics 3, 7, 13)
            8 => {
                let g1 = (-0.5 * ((kf - 3.0) / 1.4).powi(2)).exp();
                let g2 = 0.85 * (-0.5 * ((kf - 7.5) / 2.0).powi(2)).exp();
                let g3 = 0.55 * (-0.5 * ((kf - 13.0) / 2.8).powi(2)).exp();
                let base = if k == 1 { 0.45 } else { 0.0 };
                (base + g1 + g2 + g3, 0.0)
            }
            // 9: Metallic (sparse high prime/odd cluster)
            9 => {
                if k == 1 {
                    (0.55, 0.0)
                } else if matches!(k, 3 | 5 | 9 | 13 | 17 | 23 | 29 | 37 | 47 | 59) {
                    (1.2 / kf.powf(0.45), (kf * 1.13) % TAU)
                } else {
                    (0.0, 0.0)
                }
            }
            // 10: Acid (resonant 303-like peaky saw)
            10 => {
                let res_peak = 1.0 + 1.8 * (-0.5 * ((kf - 5.0) / 2.0).powi(2)).exp();
                (res_peak / kf.powf(0.95), 0.15 * kf)
            }
            // 11: Bass (punchy fundamental + warm low harmonics)
            11 => match k {
                1 => (1.0, 0.0),
                2 => (0.72, 0.2),
                3 => (0.42, 0.0),
                4 => (0.24, 0.3),
                5 => (0.14, 0.0),
                6 => (0.08, 0.1),
                _ => (0.05 / kf, 0.0),
            },
            // 12: Choir (smooth vowel-like harmonic envelope)
            12 => {
                let p1 = (-0.5 * ((kf - 2.0) / 1.5).powi(2)).exp();
                let p2 = 0.7 * (-0.5 * ((kf - 5.5) / 1.8).powi(2)).exp();
                let p3 = 0.45 * (-0.5 * ((kf - 10.0) / 2.5).powi(2)).exp();
                let amp = (if k == 1 { 0.7 } else { 0.0 }) + 0.6 * p1 + p2 + p3;
                (amp / kf.powf(0.25), (kf * 0.25) % TAU)
            }
            // 13: Strings (bowed saw with rich mid-upper body)
            13 => {
                let bow = 1.0 + 0.55 * (-0.5 * ((kf - 6.0) / 3.5).powi(2)).exp();
                let alt = if k % 2 == 0 { 0.85 } else { 1.0 };
                (bow * alt / kf.powf(0.9), (kf * 0.41) % TAU)
            }
            // 14: Hollow (strong odd harmonics 1, 3, 5, 7, suppressed even)
            14 => {
                if k % 2 == 1 {
                    let boost = if k == 3 || k == 5 { 1.6 } else { 1.0 };
                    (boost / kf.powf(0.85), 0.0)
                } else {
                    (0.06 / kf, 0.0)
                }
            }
            // 15: Sub (deep clean sub with gentle analog 2nd/3rd harmonics)
            _ => match k {
                1 => (1.0, 0.0),
                2 => (0.22, 0.0),
                3 => (0.08, 0.0),
                _ => (0.0, 0.0),
            },
        }
    }

    fn generate_table(wf: usize, max_h: usize, out: &mut [f32; TABLE_SIZE + 1]) {
        out.fill(0.0);
        let limit = max_h.min(TABLE_SIZE / 2 - 1).max(1);

        for k in 1..=limit {
            let (amp, phase_off) = Self::harmonic_spec(wf, k);
            if amp.abs() < 1e-7 {
                continue;
            }
            // Lanczos sigma factor to suppress Gibbs ringing on band-limited truncations
            let sigma = if limit > 2 {
                let x = PI * (k as f32) / ((limit + 1) as f32);
                if x.abs() < 1e-6 {
                    1.0
                } else {
                    x.sin() / x
                }
            } else {
                1.0
            };
            let weighted = amp * sigma;
            let freq_step = TAU * (k as f32) / (TABLE_SIZE as f32);
            for (i, sample) in out.iter_mut().take(TABLE_SIZE).enumerate() {
                *sample += weighted * ((i as f32) * freq_step + phase_off).sin();
            }
        }

        // Remove any residual numerical DC and normalize peak to 1.0
        let mean: f32 = out.iter().take(TABLE_SIZE).sum::<f32>() / (TABLE_SIZE as f32);
        let mut peak = 0.0_f32;
        for sample in out.iter_mut().take(TABLE_SIZE) {
            *sample -= mean;
            peak = peak.max(sample.abs());
        }
        if peak > 1e-6 {
            let inv_peak = 1.0 / peak;
            for sample in out.iter_mut().take(TABLE_SIZE) {
                *sample *= inv_peak;
            }
        }
        out[TABLE_SIZE] = out[0];
    }

    #[inline]
    fn sample_mip(&self, wf: usize, mip: usize, phase: f32) -> f32 {
        let p = phase.rem_euclid(1.0) * (TABLE_SIZE as f32);
        let i0 = (p as usize).min(TABLE_SIZE - 1);
        let frac = p - (i0 as f32);
        let tbl = &self.tables[wf][mip];
        tbl[i0] + frac * (tbl[i0 + 1] - tbl[i0])
    }

    /// Band-limited, mip-interpolated wavetable lookup.
    #[inline]
    pub fn lookup(&self, waveform: OscWaveform, phase: f32, phase_inc: f32) -> f32 {
        let wf = waveform_index(waveform);
        let dt = phase_inc.abs().max(1e-6);
        let max_h = (0.46 / dt).clamp(1.0, 255.0);
        let mip_f = (255.0 / max_h).log2().clamp(0.0, (NUM_MIPS - 1) as f32);
        let mip0 = (mip_f as usize).min(NUM_MIPS - 1);
        let mip1 = (mip0 + 1).min(NUM_MIPS - 1);
        let frac = mip_f - (mip0 as f32);

        let s0 = self.sample_mip(wf, mip0, phase);
        if mip0 == mip1 || frac < 1e-5 {
            s0
        } else {
            let s1 = self.sample_mip(wf, mip1, phase);
            s0 + frac * (s1 - s0)
        }
    }
}

/// Snapshot of one oscillator's static parameters (extracted from `OscParams`
/// or constructed directly in tests).
#[derive(Debug, Clone, Copy)]
pub struct OscSettings {
    pub mode: OscMode,
    pub waveform: OscWaveform,
    pub wt_mix: f32,
    pub note: i32,
    pub fine: f32,
    pub octave: i32,
    pub ctrl_a: f32,
    pub ctrl_b: f32,
    pub phase: f32,
    pub unison_count: usize,
    pub unison_detune: f32,
    pub unison_density: f32,
    pub unison_mode: UnisonMode,
    pub wide: f32,
    pub pan: f32,
    pub mix_level: f32,
    pub filter_balance: f32,
    pub ana: bool,
    pub inv: bool,
    pub key: bool,
}

impl Default for OscSettings {
    fn default() -> Self {
        Self {
            mode: OscMode::Classic,
            waveform: OscWaveform::Saw,
            wt_mix: 0.0,
            note: 0,
            fine: 0.0,
            octave: 0,
            ctrl_a: 0.5,
            ctrl_b: 0.0,
            phase: 0.0,
            unison_count: 1,
            unison_detune: 0.2,
            unison_density: 0.5,
            unison_mode: UnisonMode::Octave1,
            wide: 0.5,
            pan: 0.0,
            mix_level: 0.8,
            filter_balance: 0.0,
            ana: false,
            inv: false,
            key: true,
        }
    }
}

impl OscSettings {
    /// Extract a plain `Copy` snapshot from `nih_plug` `OscParams`.
    #[inline]
    pub fn from_params(p: &OscParams) -> Self {
        Self {
            mode: p.mode.value(),
            waveform: p.waveform.value(),
            wt_mix: p.wt_mix.value(),
            note: p.note.value(),
            fine: p.fine.value(),
            octave: p.octave.value(),
            ctrl_a: p.ctrl_a.value(),
            ctrl_b: p.ctrl_b.value(),
            phase: p.phase.value(),
            unison_count: (p.unison_count.value().clamp(1, MAX_UNISON as i32)) as usize,
            unison_detune: p.unison_detune.value(),
            unison_density: p.unison_density.value(),
            unison_mode: p.unison_mode.value(),
            wide: p.wide.value(),
            pan: p.pan.value(),
            mix_level: p.mix_level.value(),
            filter_balance: p.filter_balance.value(),
            ana: p.ana.value(),
            inv: p.inv.value(),
            key: p.key.value(),
        }
    }
}

/// Per-sample modulation offsets applied to one oscillator.
#[derive(Debug, Clone, Copy, Default)]
pub struct OscModInput {
    pub pitch_st: f32,
    pub ctrl_a: f32,
    pub ctrl_b: f32,
    pub wt_mix: f32,
    pub level: f32,
    pub pan: f32,
    pub detune: f32,
}

/// Stereo oscillator output pre-split into Filter 1 and Filter 2 busses.
#[derive(Debug, Clone, Copy, Default)]
pub struct OscOutput {
    pub f1_l: f32,
    pub f1_r: f32,
    pub f2_l: f32,
    pub f2_r: f32,
}

/// Unison chord/interval offsets in semitones for sub-voice `v` of `count`.
#[inline]
fn unison_interval_semitones(mode: UnisonMode, voice_idx: usize) -> f32 {
    if voice_idx == 0 {
        return 0.0;
    }
    match mode {
        // Spire's "1 Octave" keeps all unison voices in the base octave (classic supersaw)
        UnisonMode::Octave1 => 0.0,
        UnisonMode::Octave2 => {
            if voice_idx % 2 == 1 {
                12.0
            } else {
                0.0
            }
        }
        UnisonMode::Octave3 => match voice_idx % 3 {
            1 => 12.0,
            2 => 24.0,
            _ => 0.0,
        },
        UnisonMode::Fifth => match voice_idx % 4 {
            1 => 7.0,
            2 => 12.0,
            3 => 19.0,
            _ => 0.0,
        },
        UnisonMode::Minor3rd => match voice_idx % 4 {
            1 => 3.0,
            2 => 7.0,
            3 => 12.0,
            _ => 0.0,
        },
        UnisonMode::Major3rd => match voice_idx % 4 {
            1 => 4.0,
            2 => 7.0,
            3 => 12.0,
            _ => 0.0,
        },
        UnisonMode::Sus4 => match voice_idx % 4 {
            1 => 5.0,
            2 => 7.0,
            3 => 12.0,
            _ => 0.0,
        },
    }
}

/// Normalized symmetric detune offset in [-1.0, 1.0] shaped by `density` (0..1).
/// `density = 0.0` pushes sub-voices wider toward outer edges;
/// `density = 0.5` is uniform; `density = 1.0` clusters inner voices tightly.
#[inline]
fn unison_detune_position(voice_idx: usize, count: usize, density: f32) -> f32 {
    if count <= 1 {
        return 0.0;
    }
    let raw = if count % 2 == 1 {
        if voice_idx == 0 {
            0.0
        } else {
            let pair = ((voice_idx + 1) / 2) as f32;
            let max_pair = (count / 2) as f32;
            let sign = if voice_idx % 2 == 1 { -1.0 } else { 1.0 };
            sign * (pair / max_pair)
        }
    } else {
        let pair = (voice_idx / 2) as f32 + 0.5;
        let max_pair = (count as f32) * 0.5;
        let sign = if voice_idx % 2 == 0 { -1.0 } else { 1.0 };
        sign * (pair / max_pair)
    };

    // Power curve exponent: 0.45 at density=0 -> 1.0 at density=0.5 -> 2.4 at density=1.0
    let d = density.clamp(0.0, 1.0);
    let exp = if d < 0.5 {
        0.45 + (d * 2.0) * 0.55
    } else {
        1.0 + ((d - 0.5) * 2.0) * 1.4
    };
    raw.signum() * raw.abs().powf(exp)
}

/// Stereo pan position in [-1.0, 1.0] for unison voice `voice_idx` of `count`.
#[inline]
fn unison_pan_position(voice_idx: usize, count: usize) -> f32 {
    if count <= 1 {
        return 0.0;
    }
    if count % 2 == 1 {
        if voice_idx == 0 {
            0.0
        } else {
            let pair = ((voice_idx + 1) / 2) as f32;
            let max_pair = (count / 2) as f32;
            let sign = if voice_idx % 2 == 1 { -1.0 } else { 1.0 };
            sign * (pair / max_pair)
        }
    } else {
        let pair = (voice_idx / 2) as f32 + 0.5;
        let max_pair = (count as f32) * 0.5;
        let sign = if voice_idx % 2 == 0 { -1.0 } else { 1.0 };
        sign * (pair / max_pair)
    }
}

/// Formant table (F1, F2, F3 in Hz) for Spire Vowel mode: A, E, I, O, U.
const VOWEL_FORMANTS: [[f32; 3]; 5] = [
    [730.0, 1090.0, 2440.0], // A
    [530.0, 1840.0, 2480.0], // E
    [270.0, 2290.0, 3010.0], // I
    [570.0, 840.0, 2410.0],  // O
    [300.0, 870.0, 2240.0],  // U
];

const VOWEL_WEIGHTS: [f32; 3] = [1.0, 0.65, 0.35];

/// Single multi-mode unison oscillator (one of 4 per voice).
pub struct Oscillator {
    sample_rate: f32,
    inv_sr: f32,
    phases: [f32; MAX_UNISON],
    slave_phases: [f32; MAX_UNISON],
    fb_state: [f32; MAX_UNISON],
    rng_state: [u32; MAX_UNISON],
    noise_lp: [f32; MAX_UNISON],
    noise_bp: [f32; MAX_UNISON],
}

impl Oscillator {
    pub fn new(sample_rate: f32, seed: u32) -> Self {
        // Ensure global band-limited wavetables are built off the audio thread.
        let _ = WavetableBank::global();

        let sr = if sample_rate.is_finite() && sample_rate >= 8000.0 {
            sample_rate
        } else {
            44100.0
        };

        let mut rng_state = [0u32; MAX_UNISON];
        let mut phases = [0.0f32; MAX_UNISON];
        let mut slave_phases = [0.0f32; MAX_UNISON];
        let mut s = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453) | 1;
        for i in 0..MAX_UNISON {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            rng_state[i] = s | 1;
            phases[i] = next_rand_unit(&mut rng_state[i]);
            slave_phases[i] = next_rand_unit(&mut rng_state[i]);
        }

        Self {
            sample_rate: sr,
            inv_sr: 1.0 / sr,
            phases,
            slave_phases,
            fb_state: [0.0; MAX_UNISON],
            rng_state,
            noise_lp: [0.0; MAX_UNISON],
            noise_bp: [0.0; MAX_UNISON],
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate >= 8000.0 {
            self.sample_rate = sample_rate;
            self.inv_sr = 1.0 / sample_rate;
        }
    }

    pub fn reset(&mut self) {
        self.phases = [0.0; MAX_UNISON];
        self.slave_phases = [0.0; MAX_UNISON];
        self.fb_state = [0.0; MAX_UNISON];
        self.noise_lp = [0.0; MAX_UNISON];
        self.noise_bp = [0.0; MAX_UNISON];
    }

    /// Trigger or retrigger the oscillator on note-on.
    /// When `settings.ana` is true, phases run freely (analog free-run);
    /// otherwise phases reset deterministically to `settings.phase`.
    pub fn note_on(&mut self, settings: &OscSettings) {
        self.fb_state = [0.0; MAX_UNISON];
        self.noise_lp = [0.0; MAX_UNISON];
        self.noise_bp = [0.0; MAX_UNISON];

        if !settings.ana {
            let base_p = settings.phase.rem_euclid(1.0);
            let count = settings.unison_count.clamp(1, MAX_UNISON);
            for i in 0..MAX_UNISON {
                // Voice 0 starts at exact `settings.phase`; extra unison voices get golden-ratio dispersion
                // so unison stacks sound full immediately without transient phase-lock click.
                let spread = if i == 0 || count <= 1 {
                    0.0
                } else {
                    ((i as f32) * 0.618_033_98).fract()
                };
                self.phases[i] = (base_p + spread).fract();
                self.slave_phases[i] = self.phases[i];
            }
        }
    }

    /// Compute one stereo sample pair split into Filter 1 and Filter 2 inputs.
    #[inline]
    pub fn process_sample(
        &mut self,
        base_midi_note: f32,
        settings: &OscSettings,
        mod_in: &OscModInput,
    ) -> OscOutput {
        let level = (settings.mix_level + mod_in.level).clamp(0.0, 1.5);
        let count = settings.unison_count.clamp(1, MAX_UNISON);

        // Compute effective oscillator pitch in MIDI semitones.
        let note_base = if settings.key { base_midi_note } else { 60.0 };
        let pitch_st = note_base
            + (settings.octave as f32) * 12.0
            + (settings.note as f32)
            + settings.fine * 0.01
            + mod_in.pitch_st;

        // If muted, keep free-running ANA phases advancing cheaply and return silence.
        if level <= 1e-6 {
            if settings.ana {
                let freq = midi_to_hz(pitch_st).clamp(1.0, self.sample_rate * 0.45);
                let dt = freq * self.inv_sr;
                for i in 0..count {
                    self.phases[i] = (self.phases[i] + dt).fract();
                }
            }
            return OscOutput::default();
        }

        let ctrl_a = (settings.ctrl_a + mod_in.ctrl_a).clamp(0.0, 1.0);
        let ctrl_b = (settings.ctrl_b + mod_in.ctrl_b).clamp(0.0, 1.0);
        let wt_mix = (settings.wt_mix + mod_in.wt_mix).clamp(0.0, 1.0);
        let detune_amt = (settings.unison_detune + mod_in.detune).clamp(0.0, 1.0);
        let wide = settings.wide.clamp(0.0, 1.0);
        let pan = (settings.pan + mod_in.pan).clamp(-1.0, 1.0);
        let bank = WavetableBank::global();

        // Maximum unison detune +/- 50 cents (~0.5 semitones) at detune = 1.0
        let max_detune_st = detune_amt * detune_amt * 0.65 + detune_amt * 0.15;

        let mut sum_l = 0.0_f32;
        let mut sum_r = 0.0_f32;
        let max_freq = self.sample_rate * 0.45;

        for v in 0..count {
            let interval_st = unison_interval_semitones(settings.unison_mode, v);
            let detune_st =
                unison_detune_position(v, count, settings.unison_density) * max_detune_st;
            let voice_freq = midi_to_hz(pitch_st + interval_st + detune_st).clamp(1.0, max_freq);
            let dt = (voice_freq * self.inv_sr).clamp(1e-6, 0.45);

            let sample = self.render_subvoice(
                v,
                dt,
                voice_freq,
                settings.mode,
                settings.waveform,
                ctrl_a,
                ctrl_b,
                wt_mix,
                bank,
            );

            // Stereo panning for this unison sub-voice
            let v_pan = unison_pan_position(v, count) * wide;
            let angle = (v_pan + 1.0) * FRAC_PI_4;
            let g_l = angle.cos() * SQRT_2;
            let g_r = angle.sin() * SQRT_2;
            sum_l += sample * g_l;
            sum_r += sample * g_r;
        }

        // Normalize across unison voice count (RMS constant energy)
        let norm = 1.0 / (count as f32).sqrt();
        let polarity = if settings.inv { -1.0 } else { 1.0 };
        let scale = level * norm * polarity;

        // Apply oscillator-level stereo pan (constant power, center = 1.0 gain)
        let pan_angle = (pan + 1.0) * FRAC_PI_4;
        let pan_l = pan_angle.cos() * SQRT_2;
        let pan_r = pan_angle.sin() * SQRT_2;

        let out_l = flush_denormal(sum_l * scale * pan_l);
        let out_r = flush_denormal(sum_r * scale * pan_r);

        let to_f2 = settings.filter_balance.clamp(0.0, 1.0);
        let to_f1 = 1.0 - to_f2;

        OscOutput {
            f1_l: out_l * to_f1,
            f1_r: out_r * to_f1,
            f2_l: out_l * to_f2,
            f2_r: out_r * to_f2,
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn render_subvoice(
        &mut self,
        v: usize,
        dt: f32,
        freq: f32,
        mode: OscMode,
        waveform: OscWaveform,
        ctrl_a: f32,
        ctrl_b: f32,
        wt_mix: f32,
        bank: &WavetableBank,
    ) -> f32 {
        let ph = self.phases[v];
        let next_ph = ph + dt;
        let wrapped = next_ph >= 1.0;
        self.phases[v] = if wrapped { next_ph - 1.0 } else { next_ph };

        let primary = match mode {
            OscMode::Classic => self.render_classic(ph, dt, waveform, ctrl_a, ctrl_b, bank),
            OscMode::Noise => self.render_noise(v, freq, ctrl_a, ctrl_b),
            OscMode::FM => self.render_fm(v, ph, dt, ctrl_a, ctrl_b),
            OscMode::AMSync => self.render_amsync(v, ph, dt, wrapped, next_ph, ctrl_a, ctrl_b),
            OscMode::SawPWM => self.render_saw_pwm(ph, dt, ctrl_a, ctrl_b),
            OscMode::HardFM => self.render_hard_fm(v, ph, dt, ctrl_a, ctrl_b),
            OscMode::Vowel => self.render_vowel(ph, freq, ctrl_a, ctrl_b),
        };

        if wt_mix <= 1e-5 {
            primary
        } else {
            // Wavetable layer with subtle phase distortion from ctrl_b for animated morphing
            let warped_ph = if ctrl_b > 1e-4 {
                let bend = 0.15 + 0.7 * (1.0 - ctrl_b * 0.75);
                if ph < bend {
                    0.5 * ph / bend
                } else {
                    0.5 + 0.5 * (ph - bend) / (1.0 - bend)
                }
            } else {
                ph
            };
            let wt = bank.lookup(waveform, warped_ph, dt);
            primary + wt_mix * (wt - primary)
        }
    }

    /// Classic mode: PolyBLEP Saw <-> Pulse/Square morph controlled by `ctrl_a`
    /// and pulse width / shape `ctrl_b`, adapting smoothly to the selected `waveform`.
    #[inline]
    fn render_classic(
        &self,
        ph: f32,
        dt: f32,
        waveform: OscWaveform,
        ctrl_a: f32,
        ctrl_b: f32,
        bank: &WavetableBank,
    ) -> f32 {
        let pw = 0.08 + 0.84 * (0.5 - 0.42 * ctrl_b);
        let saw = (2.0 * ph - 1.0) - poly_blep(ph, dt);

        let mut pulse = if ph < pw { 1.0 } else { -1.0 };
        pulse += poly_blep(ph, dt);
        let ph_pw = (ph - pw).rem_euclid(1.0);
        pulse -= poly_blep(ph_pw, dt);
        pulse -= 2.0 * pw - 1.0;

        match waveform {
            OscWaveform::Saw => pulse + ctrl_a * (saw - pulse),
            OscWaveform::Square => {
                let sq_pw = 0.5 - 0.35 * ctrl_b;
                let mut sq = if ph < sq_pw { 1.0 } else { -1.0 };
                sq += poly_blep(ph, dt);
                sq -= poly_blep((ph - sq_pw).rem_euclid(1.0), dt);
                sq -= 2.0 * sq_pw - 1.0;
                sq + (ctrl_a - 0.5).abs() * 0.35 * (saw - sq)
            }
            OscWaveform::Pulse => {
                let narrow_pw = 0.12 + 0.36 * ctrl_a;
                let mut pl = if ph < narrow_pw { 1.0 } else { -1.0 };
                pl += poly_blep(ph, dt);
                pl -= poly_blep((ph - narrow_pw).rem_euclid(1.0), dt);
                pl -= 2.0 * narrow_pw - 1.0;
                pl
            }
            OscWaveform::Sin => {
                let s = (ph * TAU).sin();
                let shaped = fast_tanh(s * (1.0 + ctrl_b * 3.0));
                s + ctrl_a * 0.25 * (shaped - s)
            }
            OscWaveform::Tri => {
                let tri = if ph < 0.25 {
                    4.0 * ph
                } else if ph < 0.75 {
                    2.0 - 4.0 * ph
                } else {
                    4.0 * ph - 4.0
                };
                tri + ctrl_b * 0.45 * (saw - tri)
            }
            // Rich named wavetables (SuperSaw, Acid, Sub, Bell, Strings, etc.) blend cleanly
            // with the Classic PolyBLEP core so selecting a waveform immediately shapes tone
            // while wt_mix still crossfades into 100% pure wavetable + warp.
            _ => {
                let classic = pulse + ctrl_a * (saw - pulse);
                let wt = bank.lookup(waveform, ph, dt);
                0.35 * classic + 0.65 * wt
            }
        }
    }

    /// Noise mode: 2-pole SVF filtered noise (`ctrl_a` = filter cutoff/pitch, `ctrl_b` = resonance).
    #[inline]
    fn render_noise(&mut self, v: usize, freq: f32, ctrl_a: f32, ctrl_b: f32) -> f32 {
        let white = next_noise_bipolar(&mut self.rng_state[v]);
        // Cutoff tracks base frequency shifted by ctrl_a (-2..+5 octaves)
        let fc = (freq * 2.0_f32.powf((ctrl_a - 0.25) * 7.0))
            .clamp(40.0, (self.sample_rate * 0.42).min(18000.0));
        let f = (2.0 * (PI * fc * self.inv_sr).sin()).clamp(0.001, 1.4);
        let q = (1.0 - ctrl_b * 0.92).clamp(0.08, 1.0);

        let lp = self.noise_lp[v];
        let bp = self.noise_bp[v];
        let hp = white - lp - q * bp;
        let new_bp = flush_denormal(fast_tanh(bp + f * hp));
        let new_lp = flush_denormal(lp + f * new_bp);
        self.noise_bp[v] = new_bp;
        self.noise_lp[v] = new_lp;

        // Blend LP and resonant BP as ctrl_b rises
        let out = new_lp * (1.0 - ctrl_b * 0.6) + new_bp * (0.4 + ctrl_b * 0.9);
        fast_tanh(out * 1.15)
    }

    /// FM mode: 2-op smooth phase modulation (`ctrl_a` = modulator ratio, `ctrl_b` = FM depth).
    #[inline]
    fn render_fm(&mut self, v: usize, ph: f32, dt: f32, ctrl_a: f32, ctrl_b: f32) -> f32 {
        // Musical ratio sweep 0.5x .. 8.0x
        let ratio = 0.5 + ctrl_a * 7.5;
        let mod_dt = (dt * ratio).min(0.45);
        let mod_ph = self.slave_phases[v];
        self.slave_phases[v] = (mod_ph + mod_dt).fract();

        // Anti-alias index rolloff as carrier/mod approaches Nyquist
        let nyq_guard = (1.0 - (dt * (1.0 + ratio) * 2.0)).clamp(0.0, 1.0);
        let index = (0.15 + ctrl_b * 6.5) * nyq_guard;
        let modulator = (mod_ph * TAU).sin();
        (ph * TAU + index * modulator).sin()
    }

    /// AMSync mode: Anti-aliased hard sync + AM hybrid (`ctrl_a` = slave ratio, `ctrl_b` = AM depth).
    #[inline]
    fn render_amsync(
        &mut self,
        v: usize,
        ph: f32,
        dt: f32,
        wrapped: bool,
        next_ph: f32,
        ctrl_a: f32,
        ctrl_b: f32,
    ) -> f32 {
        let sync_ratio = 1.0 + ctrl_a * 7.0;
        let slave_dt = (dt * sync_ratio).min(0.45);

        let slave_ph = if wrapped {
            // Sub-sample accurate slave phase reset upon master wrap
            let frac_after_wrap = (next_ph - 1.0) / dt.max(1e-6);
            (frac_after_wrap * slave_dt).rem_euclid(1.0)
        } else {
            (self.slave_phases[v] + slave_dt).rem_euclid(1.0)
        };
        self.slave_phases[v] = slave_ph;

        // Band-limited slave saw + windowing at master edge to eliminate hard-sync aliasing
        let slave_saw = (2.0 * slave_ph - 1.0) - poly_blep(slave_ph, slave_dt);
        // Smooth short taper near master wrap boundary (0..dt and 1-dt..1)
        let edge_win = (ph / (dt * 2.0).max(0.015)).min(1.0)
            * ((1.0 - ph) / (dt * 2.0).max(0.015)).min(1.0);
        let synced = slave_saw * (0.25 + 0.75 * edge_win);

        // AM/Ring modulation with master carrier sine controlled by ctrl_b
        let carrier = (ph * TAU).sin();
        let am = synced * (1.0 - ctrl_b + ctrl_b * (0.5 + 0.5 * carrier) * 1.45);
        am.clamp(-1.2, 1.2)
    }

    /// SawPWM mode: Dual phase-shifted PolyBLEP saws (`ctrl_a` = phase offset, `ctrl_b` = skew/asymmetry).
    #[inline]
    fn render_saw_pwm(&self, ph: f32, dt: f32, ctrl_a: f32, ctrl_b: f32) -> f32 {
        let saw1 = (2.0 * ph - 1.0) - poly_blep(ph, dt);
        let offset = 0.05 + 0.90 * ctrl_a;
        let ph2 = (ph + offset).rem_euclid(1.0);
        let saw2 = (2.0 * ph2 - 1.0) - poly_blep(ph2, dt);

        // Blend from pulse-like difference saw (ctrl_b = 0) to thick dual/asymmetric saw (ctrl_b = 1)
        let diff_saw = 0.65 * (saw1 - saw2);
        let sum_saw = 0.55 * (saw1 + saw2);
        let out = diff_saw + ctrl_b * (sum_saw - diff_saw);
        fast_tanh(out * 1.15)
    }

    /// HardFM mode: Aggressive harmonic-rich FM with modulator self-feedback (`ctrl_a` = ratio, `ctrl_b` = depth/fb).
    #[inline]
    fn render_hard_fm(&mut self, v: usize, ph: f32, dt: f32, ctrl_a: f32, ctrl_b: f32) -> f32 {
        // Stepped/continuous harmonic ratios biased toward aggressive metallic timbres
        let ratio = 1.0 + ctrl_a * 11.0;
        let mod_dt = (dt * ratio).min(0.45);
        let mod_ph = self.slave_phases[v];
        self.slave_phases[v] = (mod_ph + mod_dt).fract();

        let nyq_guard = (1.0 - (dt * (1.0 + ratio * 0.5) * 1.8)).clamp(0.0, 1.0);
        let fb = self.fb_state[v];
        let fb_amt = (0.35 + ctrl_b * 1.35) * nyq_guard;
        let mod_sig = (mod_ph * TAU + fb_amt * fb).sin();
        self.fb_state[v] = flush_denormal(0.5 * (fb + mod_sig));

        let index = (0.6 + ctrl_b * 9.5) * nyq_guard;
        let carrier = (ph * TAU + index * mod_sig).sin();
        fast_tanh(carrier * (1.0 + ctrl_b * 0.6))
    }

    /// Vowel mode: Pitch-synchronous 3-formant vocal synthesizer (`ctrl_a` = A/E/I/O/U morph, `ctrl_b` = formant shift).
    #[inline]
    fn render_vowel(&self, ph: f32, freq: f32, ctrl_a: f32, ctrl_b: f32) -> f32 {
        let pos = ctrl_a.clamp(0.0, 1.0) * 4.0;
        let idx0 = (pos as usize).min(3);
        let idx1 = idx0 + 1;
        let frac = pos - (idx0 as f32);

        // Throat/formant frequency shift from 0.6x to 1.75x
        let shift = 0.6 + ctrl_b * 1.15;
        let f0 = freq.max(20.0);
        let nyq_limit = self.sample_rate * 0.42;

        // Smooth Hann excitation window synced to fundamental period (zero DC after formant sum)
        let window = 0.5 * (1.0 - (ph * TAU).cos());
        let mut sum = 0.0_f32;

        for k in 0..3 {
            let f_hz = (VOWEL_FORMANTS[idx0][k]
                + frac * (VOWEL_FORMANTS[idx1][k] - VOWEL_FORMANTS[idx0][k]))
                * shift;
            if f_hz >= nyq_limit {
                continue;
            }
            // Roll off smoothly near Nyquist
            let atten = ((nyq_limit - f_hz) / (nyq_limit * 0.18)).clamp(0.0, 1.0);
            let harm = (f_hz / f0).clamp(1.0, 64.0);
            let h_lo = harm.floor();
            let h_frac = harm - h_lo;
            // Crossfade adjacent integer harmonics with sine carriers:
            // because Hann window is even around 0.5 and sin(TAU*k*ph) is odd,
            // every harmonic k >= 1 has mathematically zero DC offset.
            let s_lo = (TAU * h_lo * ph).sin();
            let s_hi = (TAU * (h_lo + 1.0) * ph).sin();
            let formant_wave = s_lo + h_frac * (s_hi - s_lo);
            sum += VOWEL_WEIGHTS[k] * atten * formant_wave;
        }

        // Blend subtle fundamental warmth + windowed formant wave
        let fund = 0.22 * (ph * TAU).sin();
        fast_tanh((window * sum * 0.62) + fund)
    }
}

#[inline]
pub fn midi_to_hz(midi_note: f32) -> f32 {
    440.0 * 2.0_f32.powf((midi_note - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_block(settings: &OscSettings, note: f32, sr: f32, n: usize) -> Vec<(f32, f32)> {
        let mut osc = Oscillator::new(sr, 42);
        osc.note_on(settings);
        let mod_in = OscModInput::default();
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let o = osc.process_sample(note, settings, &mod_in);
            out.push((o.f1_l + o.f2_l, o.f1_r + o.f2_r));
        }
        out
    }

    fn rms_diff(a: &[(f32, f32)], b: &[(f32, f32)]) -> f32 {
        let sum: f32 = a
            .iter()
            .zip(b.iter())
            .map(|((al, ar), (bl, br))| (al - bl).powi(2) + (ar - br).powi(2))
            .sum();
        (sum / (2.0 * a.len() as f32)).sqrt()
    }

    #[test]
    fn all_osc_modes_produce_distinct_finite_low_dc_audio() {
        let modes = [
            OscMode::Classic,
            OscMode::Noise,
            OscMode::FM,
            OscMode::AMSync,
            OscMode::SawPWM,
            OscMode::HardFM,
            OscMode::Vowel,
        ];
        let mut rendered = Vec::new();
        for &mode in &modes {
            let mut s = OscSettings::default();
            s.mode = mode;
            s.ctrl_a = 0.6;
            s.ctrl_b = 0.45;
            let buf = render_block(&s, 60.0, 48000.0, 4096);

            let mut peak = 0.0_f32;
            let mut dc_l = 0.0_f32;
            let mut rms = 0.0_f32;
            for &(l, r) in &buf {
                assert!(l.is_finite() && r.is_finite(), "Non-finite in mode {mode:?}");
                peak = peak.max(l.abs()).max(r.abs());
                dc_l += l;
                rms += l * l;
            }
            dc_l /= buf.len() as f32;
            rms = (rms / buf.len() as f32).sqrt();
            assert!(rms > 0.05, "Mode {mode:?} too quiet: rms={rms}");
            assert!(peak <= 1.5, "Mode {mode:?} peak too high: {peak}");
            assert!(dc_l.abs() < 0.04, "Mode {mode:?} DC offset too high: {dc_l}");
            rendered.push((mode, buf));
        }

        // Every pair of modes must be measurably different
        for i in 0..rendered.len() {
            for j in (i + 1)..rendered.len() {
                let d = rms_diff(&rendered[i].1, &rendered[j].1);
                assert!(
                    d > 0.05,
                    "Modes {:?} and {:?} too similar: rms_diff={d}",
                    rendered[i].0,
                    rendered[j].0
                );
            }
        }
    }

    #[test]
    fn wavetable_mix_and_waveforms_produce_measurable_differences() {
        let mut s_classic = OscSettings::default();
        s_classic.waveform = OscWaveform::Bell;
        s_classic.wt_mix = 0.0;
        let a = render_block(&s_classic, 60.0, 44100.0, 2048);

        let mut s_wt = s_classic;
        s_wt.wt_mix = 1.0;
        let b = render_block(&s_wt, 60.0, 44100.0, 2048);
        assert!(rms_diff(&a, &b) > 0.05, "wt_mix must audibly morph signal");

        let mut s_sin = s_wt;
        s_sin.waveform = OscWaveform::Sin;
        let c = render_block(&s_sin, 60.0, 44100.0, 2048);
        assert!(rms_diff(&b, &c) > 0.1, "Different wavetables must differ");
    }

    #[test]
    fn unison_detune_density_mode_and_stereo_width_work() {
        let mut s_mono = OscSettings::default();
        s_mono.unison_count = 7;
        s_mono.unison_detune = 0.45;
        s_mono.wide = 0.0;
        let mono_buf = render_block(&s_mono, 60.0, 48000.0, 4096);
        let mono_side: f32 = mono_buf.iter().map(|(l, r)| (l - r).abs()).sum::<f32>() / 4096.0;
        assert!(mono_side < 1e-5, "wide=0 must be mono, got side={mono_side}");

        let mut s_wide = s_mono;
        s_wide.wide = 1.0;
        let wide_buf = render_block(&s_wide, 60.0, 48000.0, 4096);
        let wide_side: f32 = wide_buf.iter().map(|(l, r)| (l - r).abs()).sum::<f32>() / 4096.0;
        assert!(wide_side > 0.08, "wide=1 with 7-voice detune must be stereo, got {wide_side}");

        let mut s_chord = s_wide;
        s_chord.unison_mode = UnisonMode::Minor3rd;
        let chord_buf = render_block(&s_chord, 60.0, 48000.0, 4096);
        assert!(
            rms_diff(&wide_buf, &chord_buf) > 0.1,
            "UnisonMode::Minor3rd must differ from Octave1"
        );
    }

    #[test]
    fn phase_invert_filter_balance_and_keytrack_work() {
        let mut s = OscSettings::default();
        let normal = render_block(&s, 60.0, 44100.0, 512);
        s.inv = true;
        let inverted = render_block(&s, 60.0, 44100.0, 512);
        for (&(nl, nr), &(il, ir)) in normal.iter().zip(inverted.iter()) {
            assert!((nl + il).abs() < 1e-5);
            assert!((nr + ir).abs() < 1e-5);
        }

        // Filter balance routing check
        s.inv = false;
        s.filter_balance = 0.0;
        let mut osc = Oscillator::new(44100.0, 7);
        osc.note_on(&s);
        let o1 = osc.process_sample(60.0, &s, &OscModInput::default());
        assert_eq!(o1.f2_l, 0.0);
        assert_eq!(o1.f2_r, 0.0);

        s.filter_balance = 1.0;
        let o2 = osc.process_sample(60.0, &s, &OscModInput::default());
        assert_eq!(o2.f1_l, 0.0);
        assert_eq!(o2.f1_r, 0.0);

        // KEY off fixes pitch regardless of MIDI note
        s.filter_balance = 0.0;
        s.key = false;
        let n48 = render_block(&s, 48.0, 44100.0, 512);
        let n72 = render_block(&s, 72.0, 44100.0, 512);
        assert!(rms_diff(&n48, &n72) < 1e-6, "KEY=false must ignore MIDI note pitch");

        // Phase offset & ANA free-running vs KEY phase retrigger
        let mut s_p0 = OscSettings::default();
        s_p0.ana = false;
        s_p0.phase = 0.0;
        let mut s_p5 = s_p0;
        s_p5.phase = 0.5;
        let b_p0 = render_block(&s_p0, 60.0, 48000.0, 256);
        let b_p5 = render_block(&s_p5, 60.0, 48000.0, 256);
        assert!(rms_diff(&b_p0, &b_p5) > 0.2, "Phase offset must shift waveform");

        let mut osc_ana = Oscillator::new(48000.0, 99);
        let mut s_ana = OscSettings::default();
        s_ana.ana = true;
        osc_ana.note_on(&s_ana);
        for _ in 0..73 {
            osc_ana.process_sample(60.0, &s_ana, &OscModInput::default());
        }
        let ph_before = osc_ana.phases[0];
        osc_ana.note_on(&s_ana);
        assert!((osc_ana.phases[0] - ph_before).abs() < 1e-6, "ANA=true must not reset phase");
    }

    #[test]
    fn high_pitch_anti_aliasing_stays_bounded_across_sample_rates() {
        for &sr in &[44100.0_f32, 48000.0, 96000.0] {
            for &mode in &[
                OscMode::Classic,
                OscMode::FM,
                OscMode::AMSync,
                OscMode::SawPWM,
                OscMode::HardFM,
                OscMode::Vowel,
            ] {
                let mut s = OscSettings::default();
                s.mode = mode;
                s.ctrl_a = 0.9;
                s.ctrl_b = 0.9;
                s.wt_mix = 0.5;
                let buf = render_block(&s, 108.0, sr, 1024);
                assert!(buf.iter().all(|(l, r)| l.is_finite() && r.is_finite() && l.abs() <= 2.5));
            }
        }
    }
}
