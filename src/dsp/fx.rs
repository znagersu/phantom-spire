//! 5-Module Stereo FX Rack in series for Phantom Spire:
//! `SHP` (Shaper) -> `PHS` (Phaser) -> `CHR` (Chorus) -> `DEL` (Delay) -> `REV` (Reverb)
//!
//! Each module supports `MUTE` bypass, dry/wet mix, and real-time global modulation offsets
//! (`GlobalModOffsets`). All delay buffers are pre-allocated on construction or sample-rate
//! changes — zero heap allocations on the audio thread, guaranteed finite & denormal-free
//! at 44.1 kHz, 48 kHz, and 96 kHz.

use crate::dsp::mod_engine::{
    flush_denormal, sanitize_bpm, sanitize_sample_rate, sync_div_to_seconds, GlobalModOffsets,
};
use crate::params::{
    ChorusMode, ChorusParams, DelayParams, FxParams, PhaserParams, ReverbMode, ReverbParams,
    ShaperMode, ShaperParams,
};
use std::f32::consts::{PI, TAU};

// ============================================================================
// Shared Lightweight DSP Building Blocks (Zero-Alloc)
// ============================================================================

/// Dry-preserving constant-power-style blend: keeps the dry signal strong across
/// typical 0..60% FX mix levels while still reaching 100% wet at `mix = 1.0`.
#[inline]
fn dry_preserving_mix(dry: f32, wet: f32, mix: f32) -> f32 {
    let m = mix.clamp(0.0, 1.0);
    let dry_g = (1.0 - m * m).max(0.0).sqrt();
    flush_denormal(dry * dry_g + wet * m)
}

#[derive(Debug, Clone, Copy, Default)]
struct OnePole {
    z1: f32,
}

impl OnePole {
    #[inline]
    fn reset(&mut self) {
        self.z1 = 0.0;
    }

    #[inline]
    fn lp_coeff(cutoff_hz: f32, sr: f32) -> f32 {
        let fc = cutoff_hz.clamp(10.0, sr * 0.45);
        1.0 - (-TAU * fc / sr).exp()
    }

    #[inline]
    fn process_lp(&mut self, x: f32, coeff: f32) -> f32 {
        self.z1 = flush_denormal(self.z1 + (x - self.z1) * coeff);
        self.z1
    }

    #[inline]
    fn process_hp(&mut self, x: f32, coeff: f32) -> f32 {
        let lp = self.process_lp(x, coeff);
        flush_denormal(x - lp)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct DcBlocker {
    x1: f32,
    y1: f32,
}

impl DcBlocker {
    #[inline]
    fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }

    #[inline]
    fn process(&mut self, x: f32, r: f32) -> f32 {
        let y = flush_denormal(x - self.x1 + r * self.y1);
        self.x1 = x;
        self.y1 = y;
        y
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    #[inline]
    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    fn process(&mut self, x: f32, b0: f32, b1: f32, b2: f32, a1: f32, a2: f32) -> f32 {
        let y = flush_denormal(b0 * x + self.z1);
        self.z1 = flush_denormal(b1 * x - a1 * y + self.z2);
        self.z2 = flush_denormal(b2 * x - a2 * y);
        y
    }
}

/// Circular delay buffer with cubic Hermite and linear fractional-sample interpolation.
#[derive(Debug, Clone)]
struct DelayLine {
    buf: Vec<f32>,
    mask: usize,
    write_pos: usize,
}

impl DelayLine {
    fn new(min_samples: usize) -> Self {
        let len = min_samples.max(64).next_power_of_two();
        Self {
            buf: vec![0.0; len],
            mask: len - 1,
            write_pos: 0,
        }
    }

    fn resize(&mut self, min_samples: usize) {
        let len = min_samples.max(64).next_power_of_two();
        if self.buf.len() != len {
            self.buf.resize(len, 0.0);
            self.mask = len - 1;
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write_pos = 0;
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    fn write(&mut self, sample: f32) {
        self.buf[self.write_pos] = flush_denormal(sample);
        self.write_pos = (self.write_pos + 1) & self.mask;
    }

    /// Read `delay_samples` behind the most recently written sample using linear interpolation.
    #[inline]
    fn read_linear(&self, delay_samples: f32) -> f32 {
        let max_d = (self.buf.len() - 4) as f32;
        let d = delay_samples.clamp(1.0, max_d);
        let d_int = d as usize;
        let frac = d - d_int as f32;
        let idx0 = self.write_pos.wrapping_sub(d_int) & self.mask;
        let idx1 = idx0.wrapping_sub(1) & self.mask;
        let s0 = self.buf[idx0];
        let s1 = self.buf[idx1];
        s0 + (s1 - s0) * frac
    }

    /// Read `delay_samples` behind the most recently written sample using 4-point Hermite cubic.
    #[inline]
    fn read_cubic(&self, delay_samples: f32) -> f32 {
        let max_d = (self.buf.len() - 4) as f32;
        let d = delay_samples.clamp(2.0, max_d);
        let d_int = d as usize;
        let t = d - d_int as f32;

        let i1 = self.write_pos.wrapping_sub(d_int) & self.mask;
        let i0 = i1.wrapping_add(1) & self.mask;
        let i2 = i1.wrapping_sub(1) & self.mask;
        let i3 = i2.wrapping_sub(1) & self.mask;

        let y0 = self.buf[i0];
        let y1 = self.buf[i1];
        let y2 = self.buf[i2];
        let y3 = self.buf[i3];

        let c0 = y1;
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        flush_denormal(((c3 * t + c2) * t + c1) * t + c0)
    }
}

// ============================================================================
// 1. SHP — Shaper (Distortion / Waveshaper / Bitcrusher / Decimator + BAND + HQ)
// ============================================================================

pub struct ShaperFx {
    sample_rate: f32,
    dc_r: f32,
    dc_l: DcBlocker,
    dc_r_blk: DcBlocker,
    pre_hp_l: OnePole,
    pre_hp_r: OnePole,
    pre_lp_l: OnePole,
    pre_lp_r: OnePole,
    post_hp_l: OnePole,
    post_hp_r: OnePole,
    post_lp_l: OnePole,
    post_lp_r: OnePole,
    // 2x oversampling state (previous input + 2-pole Butterworth AA filter at Nyquist/2)
    os_prev_l: f32,
    os_prev_r: f32,
    os_aa_l: BiquadState,
    os_aa_r: BiquadState,
    // Sample-rate decimator state
    decim_phase: f32,
    decim_hold_l: f32,
    decim_hold_r: f32,
}

impl ShaperFx {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        Self {
            sample_rate: sr,
            dc_r: 1.0 - (TAU * 12.0 / sr).min(0.05),
            dc_l: DcBlocker::default(),
            dc_r_blk: DcBlocker::default(),
            pre_hp_l: OnePole::default(),
            pre_hp_r: OnePole::default(),
            pre_lp_l: OnePole::default(),
            pre_lp_r: OnePole::default(),
            post_hp_l: OnePole::default(),
            post_hp_r: OnePole::default(),
            post_lp_l: OnePole::default(),
            post_lp_r: OnePole::default(),
            os_prev_l: 0.0,
            os_prev_r: 0.0,
            os_aa_l: BiquadState::default(),
            os_aa_r: BiquadState::default(),
            decim_phase: 0.0,
            decim_hold_l: 0.0,
            decim_hold_r: 0.0,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sanitize_sample_rate(sample_rate);
        self.sample_rate = sr;
        self.dc_r = 1.0 - (TAU * 12.0 / sr).min(0.05);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.dc_l.reset();
        self.dc_r_blk.reset();
        self.pre_hp_l.reset();
        self.pre_hp_r.reset();
        self.pre_lp_l.reset();
        self.pre_lp_r.reset();
        self.post_hp_l.reset();
        self.post_hp_r.reset();
        self.post_lp_l.reset();
        self.post_lp_r.reset();
        self.os_prev_l = 0.0;
        self.os_prev_r = 0.0;
        self.os_aa_l.reset();
        self.os_aa_r.reset();
        self.decim_phase = 0.0;
        self.decim_hold_l = 0.0;
        self.decim_hold_r = 0.0;
    }

    #[inline]
    fn shape_sample(x: f32, mode: ShaperMode, drive: f32) -> f32 {
        let d = drive.clamp(0.0, 1.0);
        let gain = 1.0 + d * d * 18.0;
        let g = x * gain;
        let shaped = match mode {
            ShaperMode::Warm => {
                // Asymmetric warm saturation with even+odd harmonics
                let asym = g + 0.14 * g * g.abs().min(2.0);
                (asym * 0.85).tanh() * (1.0 / (1.0 + d * 0.35))
            }
            ShaperMode::Soft => {
                // Smooth soft-knee rational saturator
                let s = g / (1.0 + g.abs());
                s * 1.15
            }
            ShaperMode::Hard => {
                // Tight aggressive transistor saturation
                (g * 2.2).tanh()
            }
            ShaperMode::Clip => {
                // Crisp diode hard-clip
                (g * 1.4).clamp(-1.0, 1.0)
            }
            ShaperMode::Tube => {
                // Asymmetric vacuum-tube curve
                if g >= 0.0 {
                    1.0 - (-g * 1.2).exp()
                } else {
                    -(1.0 - (g * 0.85).exp()) * 0.92
                }
            }
            ShaperMode::Fold => {
                // West-coast wavefolder
                let folded = (g * PI * 0.5).sin();
                folded * 0.95
            }
            ShaperMode::Sine => {
                // Sinusoidal FM-like shaper
                let s = (g * (1.0 + d * 2.5)).sin();
                s * 0.9
            }
            ShaperMode::Bitcrush => {
                // Inherent step quantization + crunch
                let levels = (16.0 - d * 12.0).max(3.0);
                let q = (g.clamp(-1.0, 1.0) * levels).round() / levels;
                q.tanh()
            }
        };
        flush_denormal(shaped).clamp(-1.5, 1.5)
    }

    /// 2nd-order Butterworth LPF at fc = 0.22 * (2*sr) for 2x oversampling anti-aliasing.
    #[inline]
    fn os_down_aa(state: &mut BiquadState, s0: f32, s1: f32) -> f32 {
        // Normalized cutoff = 0.22 (below 0.25 = Nyquist of 1x rate)
        // Precomputed Butterworth 2nd-order coeffs for fc/fs = 0.22:
        const B0: f32 = 0.248_325_7;
        const B1: f32 = 0.496_651_4;
        const B2: f32 = 0.248_325_7;
        const A1: f32 = -0.184_213_8;
        const A2: f32 = 0.177_516_6;
        let _ = state.process(s0, B0, B1, B2, A1, A2);
        state.process(s1, B0, B1, B2, A1, A2)
    }

    #[inline]
    pub fn process_sample(
        &mut self,
        left: f32,
        right: f32,
        params: &ShaperParams,
        mods: &GlobalModOffsets,
    ) -> (f32, f32) {
        if params.mute.value() {
            return (left, right);
        }
        let mix = (params.dry_wet.value() + mods.fx_shaper_mix).clamp(0.0, 1.0);
        if mix <= 1e-4 {
            return (left, right);
        }

        let mode = params.mode.value();
        let band = params.band.value();
        let hq = params.hq.value();
        let drive = (params.drive.value() + mods.fx_shaper_drive).clamp(0.0, 1.0);
        let bit = params.bit.value().clamp(0.0, 1.0);
        let s_rate = params.s_rate.value().clamp(0.0, 1.0);
        let low_cut = params.low_cut.value().clamp(20.0, 2000.0);
        let hi_cut = params.hi_cut.value().clamp(1000.0, 20000.0);

        let hp_c = OnePole::lp_coeff(low_cut, self.sample_rate);
        let lp_c = OnePole::lp_coeff(hi_cut, self.sample_rate);

        // In BAND mode, isolate the low_cut..hi_cut band before waveshaping and preserve outside energy
        let (in_l, in_r, bypass_l, bypass_r) = if band {
            let bl = self.pre_lp_l.process_lp(self.pre_hp_l.process_hp(left, hp_c), lp_c);
            let br = self.pre_lp_r.process_lp(self.pre_hp_r.process_hp(right, hp_c), lp_c);
            (bl, br, left - bl, right - br)
        } else {
            (left, right, 0.0, 0.0)
        };

        // Waveshaping stage (with optional 2x oversampling in HQ mode)
        let (mut wl, mut wr) = if hq {
            let mid_l = 0.5 * (self.os_prev_l + in_l);
            let mid_r = 0.5 * (self.os_prev_r + in_r);
            self.os_prev_l = in_l;
            self.os_prev_r = in_r;

            let sl0 = Self::shape_sample(mid_l, mode, drive);
            let sl1 = Self::shape_sample(in_l, mode, drive);
            let sr0 = Self::shape_sample(mid_r, mode, drive);
            let sr1 = Self::shape_sample(in_r, mode, drive);

            (
                Self::os_down_aa(&mut self.os_aa_l, sl0, sl1),
                Self::os_down_aa(&mut self.os_aa_r, sr0, sr1),
            )
        } else {
            self.os_prev_l = in_l;
            self.os_prev_r = in_r;
            (
                Self::shape_sample(in_l, mode, drive),
                Self::shape_sample(in_r, mode, drive),
            )
        };

        // Bit-depth reduction
        if bit > 0.005 {
            let eff_bits = 16.0 - bit * 14.0; // 16 bits -> 2 bits
            let levels = 2.0_f32.powf(eff_bits - 1.0).max(2.0);
            wl = (wl * levels).round() / levels;
            wr = (wr * levels).round() / levels;
        }

        // Sample-rate decimator (sample-and-hold)
        if s_rate > 0.005 {
            // Factor from 1.0 (full rate) to 32.0 (crushed)
            let factor = 2.0_f32.powf(s_rate * 5.0);
            self.decim_phase += 1.0 / factor;
            if self.decim_phase >= 1.0 {
                self.decim_phase -= 1.0;
                self.decim_hold_l = wl;
                self.decim_hold_r = wr;
            }
            wl = self.decim_hold_l;
            wr = self.decim_hold_r;
        } else {
            self.decim_hold_l = wl;
            self.decim_hold_r = wr;
        }

        // Post DC blocker + tone filters
        wl = self.dc_l.process(wl, self.dc_r);
        wr = self.dc_r_blk.process(wr, self.dc_r);

        if band {
            wl = self.post_lp_l.process_lp(self.post_hp_l.process_hp(wl, hp_c), lp_c) + bypass_l;
            wr = self.post_lp_r.process_lp(self.post_hp_r.process_hp(wr, hp_c), lp_c) + bypass_r;
        } else {
            if low_cut > 21.0 {
                wl = self.post_hp_l.process_hp(wl, hp_c);
                wr = self.post_hp_r.process_hp(wr, hp_c);
            }
            if hi_cut < 19900.0 {
                wl = self.post_lp_l.process_lp(wl, lp_c);
                wr = self.post_lp_r.process_lp(wr, lp_c);
            }
        }

        let out_l = flush_denormal(left + (wl - left) * mix);
        let out_r = flush_denormal(right + (wr - right) * mix);
        (out_l, out_r)
    }
}

// ============================================================================
// 2. PHS — Stereo Multi-Stage Allpass Phaser
// ============================================================================

const MAX_PHASER_STAGES: usize = 12;

#[derive(Debug, Clone, Copy, Default)]
struct Allpass1 {
    x1: f32,
    y1: f32,
}

impl Allpass1 {
    #[inline]
    fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }

    #[inline]
    fn process(&mut self, x: f32, a: f32) -> f32 {
        let y = flush_denormal(a * (x - self.y1) + self.x1);
        self.x1 = x;
        self.y1 = y;
        y
    }
}

pub struct PhaserFx {
    sample_rate: f32,
    ap_l: [Allpass1; MAX_PHASER_STAGES],
    ap_r: [Allpass1; MAX_PHASER_STAGES],
    fb_l: f32,
    fb_r: f32,
    lfo_phase: f32,
}

impl PhaserFx {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sanitize_sample_rate(sample_rate),
            ap_l: [Allpass1::default(); MAX_PHASER_STAGES],
            ap_r: [Allpass1::default(); MAX_PHASER_STAGES],
            fb_l: 0.0,
            fb_r: 0.0,
            lfo_phase: 0.0,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitize_sample_rate(sample_rate);
        self.reset();
    }

    pub fn reset(&mut self) {
        for ap in &mut self.ap_l {
            ap.reset();
        }
        for ap in &mut self.ap_r {
            ap.reset();
        }
        self.fb_l = 0.0;
        self.fb_r = 0.0;
        self.lfo_phase = 0.0;
    }

    #[inline]
    fn allpass_coeff(freq_hz: f32, sr: f32) -> f32 {
        let fc = freq_hz.clamp(40.0, sr * 0.42);
        let t = (PI * fc / sr).tan();
        ((1.0 - t) / (1.0 + t)).clamp(-0.999, 0.999)
    }

    #[inline]
    pub fn process_sample(
        &mut self,
        left: f32,
        right: f32,
        params: &PhaserParams,
        mods: &GlobalModOffsets,
    ) -> (f32, f32) {
        if params.mute.value() {
            return (left, right);
        }
        let mix = (params.dry_wet.value() + mods.fx_phaser_mix).clamp(0.0, 1.0);
        if mix <= 1e-4 {
            return (left, right);
        }

        let stages = params.stages.value().clamp(2, MAX_PHASER_STAGES as i32) as usize;
        let rate = params.rate.value().clamp(0.02, 20.0);
        let fb = params.feedback.value().clamp(-0.95, 0.95);
        let spread = params.spread.value().clamp(0.0, 1.0);
        let base_freq = (params.freq.value() * 2.0_f32.powf(mods.fx_phaser_freq * 3.0))
            .clamp(60.0, self.sample_rate * 0.35);

        // Stereo LFO with spread-controlled L/R phase offset (up to 180 deg)
        let lfo_l = (self.lfo_phase * TAU).sin();
        let lfo_r = ((self.lfo_phase + spread * 0.5) * TAU).sin();
        self.lfo_phase = (self.lfo_phase + rate / self.sample_rate).fract();

        let sweep_oct = 1.4 + spread * 1.2;
        let center_l = (base_freq * 2.0_f32.powf(lfo_l * sweep_oct)).clamp(60.0, self.sample_rate * 0.4);
        let center_r = (base_freq * 2.0_f32.powf(lfo_r * sweep_oct)).clamp(60.0, self.sample_rate * 0.4);

        let mut xl = left + (self.fb_l * fb).tanh();
        let mut xr = right + (self.fb_r * fb).tanh();

        for s in 0..stages {
            // Subtle per-stage frequency staggering controlled by spread
            let stagger = 1.0 + spread * 0.12 * ((s as f32) - (stages as f32) * 0.5);
            let fl = (center_l * stagger.max(0.4)).clamp(40.0, self.sample_rate * 0.42);
            let fr = (center_r * stagger.max(0.4)).clamp(40.0, self.sample_rate * 0.42);
            let al = Self::allpass_coeff(fl, self.sample_rate);
            let ar = Self::allpass_coeff(fr, self.sample_rate);
            xl = self.ap_l[s].process(xl, al);
            xr = self.ap_r[s].process(xr, ar);
        }

        self.fb_l = flush_denormal(xl).clamp(-2.0, 2.0);
        self.fb_r = flush_denormal(xr).clamp(-2.0, 2.0);

        // Classic phaser interference: linear crossfade yields 1 - 0.5*mix dry + 0.5*mix allpass
        let wet_l = 0.5 * (left + xl);
        let wet_r = 0.5 * (right + xr);
        let out_l = flush_denormal(left + (wet_l - left) * mix);
        let out_r = flush_denormal(right + (wet_r - right) * mix);
        (out_l, out_r)
    }
}

// ============================================================================
// 3. CHR — Multi-Tap Stereo Chorus / Ensemble / Dimension
// ============================================================================

pub struct ChorusFx {
    sample_rate: f32,
    delay_l: DelayLine,
    delay_r: DelayLine,
    lfo_phase: f32,
    fb_l: f32,
    fb_r: f32,
    hp_l: OnePole,
    hp_r: OnePole,
    lp_l: OnePole,
    lp_r: OnePole,
}

impl ChorusFx {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        let cap = (0.09 * sr) as usize + 64;
        Self {
            sample_rate: sr,
            delay_l: DelayLine::new(cap),
            delay_r: DelayLine::new(cap),
            lfo_phase: 0.0,
            fb_l: 0.0,
            fb_r: 0.0,
            hp_l: OnePole::default(),
            hp_r: OnePole::default(),
            lp_l: OnePole::default(),
            lp_r: OnePole::default(),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sanitize_sample_rate(sample_rate);
        self.sample_rate = sr;
        let cap = (0.09 * sr) as usize + 64;
        self.delay_l.resize(cap);
        self.delay_r.resize(cap);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.delay_l.reset();
        self.delay_r.reset();
        self.lfo_phase = 0.0;
        self.fb_l = 0.0;
        self.fb_r = 0.0;
        self.hp_l.reset();
        self.hp_r.reset();
        self.lp_l.reset();
        self.lp_r.reset();
    }

    #[inline]
    pub fn process_sample(
        &mut self,
        left: f32,
        right: f32,
        params: &ChorusParams,
        mods: &GlobalModOffsets,
    ) -> (f32, f32) {
        if params.mute.value() {
            return (left, right);
        }
        let mix = (params.dry_wet.value() + mods.fx_chorus_mix).clamp(0.0, 1.0);
        if mix <= 1e-4 {
            return (left, right);
        }

        let mode = params.mode.value();
        let rate = params.rate.value().clamp(0.02, 12.0);
        let base_delay_ms = params.delay.value().clamp(1.0, 40.0);
        let depth = (params.depth.value() + mods.fx_chorus_depth).clamp(0.0, 1.0);
        let fb = params.feedback.value().clamp(-0.9, 0.9);
        let low_cut = params.low_cut.value().clamp(20.0, 2000.0);
        let hi_cut = params.hi_cut.value().clamp(1000.0, 20000.0);

        let sr = self.sample_rate;
        let base_samp = base_delay_ms * 0.001 * sr;
        let mod_amp_samp = depth * (base_delay_ms * 0.45).min(12.0) * 0.001 * sr;

        // Write inputs + soft-clipped feedback into stereo delay lines
        self.delay_l.write((left + self.fb_l * fb).tanh());
        self.delay_r.write((right + self.fb_r * fb).tanh());

        let p = self.lfo_phase;
        self.lfo_phase = (self.lfo_phase + rate / sr).fract();

        let (mut wl, mut wr) = match mode {
            ChorusMode::Chorus => {
                // Classic 2-tap quadrature stereo chorus with offset L/R center delays
                // so summing to mono doesn't produce deep symmetric comb cancellations.
                let d_l = (base_samp * 0.93 + (p * TAU).sin() * mod_amp_samp).max(2.0);
                let d_r = (base_samp * 1.07 + ((p + 0.25) * TAU).sin() * mod_amp_samp).max(2.0);
                (self.delay_l.read_cubic(d_l), self.delay_r.read_cubic(d_r))
            }
            ChorusMode::Ensemble => {
                // 3-phase (0 / 120 / 240 deg) string-ensemble chorus, level-matched to dry
                let l0 = (p * TAU).sin();
                let l1 = ((p + 1.0 / 3.0) * TAU).sin();
                let l2 = ((p + 2.0 / 3.0) * TAU).sin();

                let tl0 = self.delay_l.read_cubic((base_samp + l0 * mod_amp_samp).max(2.0));
                let tl1 = self
                    .delay_l
                    .read_cubic((base_samp * 1.17 + l1 * mod_amp_samp * 0.85).max(2.0));
                let tl2 = self
                    .delay_r
                    .read_cubic((base_samp * 0.86 + l2 * mod_amp_samp * 0.9).max(2.0));

                let tr0 = self.delay_r.read_cubic((base_samp + l1 * mod_amp_samp).max(2.0));
                let tr1 = self
                    .delay_r
                    .read_cubic((base_samp * 1.19 + l2 * mod_amp_samp * 0.85).max(2.0));
                let tr2 = self
                    .delay_l
                    .read_cubic((base_samp * 0.84 + l0 * mod_amp_samp * 0.9).max(2.0));

                ((tl0 + tl1 + tl2 * 0.7) * 0.34, (tr0 + tr1 + tr2 * 0.7) * 0.34)
            }
            ChorusMode::Dimension => {
                // 4-tap Dimension-D style M/S-safe stereo widener: mono sum (wl + wr)/2 stays intact.
                let s0 = (p * TAU).sin();
                let s1 = ((p + 0.25) * TAU).sin();
                let amp = mod_amp_samp * 0.55;
                let a = self.delay_l.read_cubic((base_samp * 0.77 + s0 * amp).max(2.0));
                let b = self.delay_r.read_cubic((base_samp * 1.23 + s1 * amp).max(2.0));
                let c = self.delay_l.read_cubic((base_samp * 1.13 - s0 * amp).max(2.0));
                let d = self.delay_r.read_cubic((base_samp * 0.87 - s1 * amp).max(2.0));
                let core_l = 0.5 * (a + c);
                let core_r = 0.5 * (b + d);
                let mid = 0.5 * (core_l + core_r);
                let side = 0.5 * (core_l - core_r) + 0.22 * ((b - d) - (a - c));
                (mid + side, mid - side)
            }
        };

        // Filter wet output
        let hp_c = OnePole::lp_coeff(low_cut, sr);
        let lp_c = OnePole::lp_coeff(hi_cut, sr);
        if low_cut > 21.0 {
            wl = self.hp_l.process_hp(wl, hp_c);
            wr = self.hp_r.process_hp(wr, hp_c);
        }
        if hi_cut < 19900.0 {
            wl = self.lp_l.process_lp(wl, lp_c);
            wr = self.lp_r.process_lp(wr, lp_c);
        }

        self.fb_l = flush_denormal(wl);
        self.fb_r = flush_denormal(wr);

        let wet_trim = 0.74 + 0.26 * mix;
        let out_l = dry_preserving_mix(left, wl * wet_trim, mix);
        let out_r = dry_preserving_mix(right, wr * wet_trim, mix);
        (out_l, out_r)
    }
}

// ============================================================================
// 4. DEL — Stereo / Ping-Pong Tempo-Synced Modulated Delay
// ============================================================================

pub struct DelayFx {
    sample_rate: f32,
    delay_l: DelayLine,
    delay_r: DelayLine,
    smooth_samples_l: f32,
    smooth_samples_r: f32,
    lfo_phase: f32,
    fb_hp_l: OnePole,
    fb_hp_r: OnePole,
    fb_lp_l: OnePole,
    fb_lp_r: OnePole,
}

impl DelayFx {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        // Up to 2.2 seconds delay capacity (covers 2000ms + modulation headroom)
        let cap = (2.25 * sr) as usize + 64;
        Self {
            sample_rate: sr,
            delay_l: DelayLine::new(cap),
            delay_r: DelayLine::new(cap),
            smooth_samples_l: 0.375 * sr,
            smooth_samples_r: 0.500 * sr,
            lfo_phase: 0.0,
            fb_hp_l: OnePole::default(),
            fb_hp_r: OnePole::default(),
            fb_lp_l: OnePole::default(),
            fb_lp_r: OnePole::default(),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sanitize_sample_rate(sample_rate);
        self.sample_rate = sr;
        let cap = (2.25 * sr) as usize + 64;
        self.delay_l.resize(cap);
        self.delay_r.resize(cap);
        self.smooth_samples_l = 0.375 * sr;
        self.smooth_samples_r = 0.500 * sr;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.delay_l.reset();
        self.delay_r.reset();
        self.lfo_phase = 0.0;
        self.fb_hp_l.reset();
        self.fb_hp_r.reset();
        self.fb_lp_l.reset();
        self.fb_lp_r.reset();
    }

    #[inline]
    pub fn process_sample(
        &mut self,
        left: f32,
        right: f32,
        params: &DelayParams,
        mods: &GlobalModOffsets,
        bpm: f32,
    ) -> (f32, f32) {
        if params.mute.value() {
            return (left, right);
        }
        let mix = (params.dry_wet.value() + mods.fx_delay_mix).clamp(0.0, 1.0);
        if mix <= 1e-4 {
            return (left, right);
        }

        let sr = self.sample_rate;
        let max_delay_s = ((self.delay_l.capacity() - 16) as f32 / sr).min(2.15);

        let (target_s_l, target_s_r) = if params.sync.value() {
            let b = sanitize_bpm(bpm);
            (
                sync_div_to_seconds(params.sync_div_l.value(), b).clamp(0.001, max_delay_s),
                sync_div_to_seconds(params.sync_div_r.value(), b).clamp(0.001, max_delay_s),
            )
        } else {
            (
                (params.time_l.value() * 0.001).clamp(0.001, max_delay_s),
                (params.time_r.value() * 0.001).clamp(0.001, max_delay_s),
            )
        };

        // Smooth tape-style slew on delay times (~25 ms) so changes never click
        let slew = 1.0 - (-1.0 / (0.025 * sr).max(1.0)).exp();
        self.smooth_samples_l += (target_s_l * sr - self.smooth_samples_l) * slew;
        self.smooth_samples_r += (target_s_r * sr - self.smooth_samples_r) * slew;

        // Subtle LFO tape warble modulation
        let mod_rate = params.rate.value().clamp(0.02, 10.0);
        let mod_depth = params.depth.value().clamp(0.0, 1.0);
        let mod_amp_samp = mod_depth * 0.003 * sr; // up to ±3ms warble
        let lfo_l = (self.lfo_phase * TAU).sin() * mod_amp_samp;
        let lfo_r = ((self.lfo_phase + 0.25) * TAU).sin() * mod_amp_samp;
        self.lfo_phase = (self.lfo_phase + mod_rate / sr).fract();

        let read_l = self.delay_l.read_cubic((self.smooth_samples_l + lfo_l).max(2.0));
        let read_r = self.delay_r.read_cubic((self.smooth_samples_r + lfo_r).max(2.0));

        // Feedback path filtering (HPF + LPF)
        let fb = (params.feedback.value() + mods.fx_delay_feedback).clamp(0.0, 0.95);
        let hp_c = OnePole::lp_coeff(params.low_cut.value().clamp(20.0, 2000.0), sr);
        let lp_c = OnePole::lp_coeff(params.hi_cut.value().clamp(1000.0, 20000.0), sr);

        let filt_l = self.fb_lp_l.process_lp(self.fb_hp_l.process_hp(read_l, hp_c), lp_c);
        let filt_r = self.fb_lp_r.process_lp(self.fb_hp_r.process_hp(read_r, hp_c), lp_c);

        if params.ping_pong.value() {
            // Ping-pong: feed stereo-summed input into L, bounce L -> R -> L
            let mono_in = 0.5 * (left + right);
            let write_l = (mono_in + filt_r * fb).tanh();
            let write_r = (filt_l * fb).tanh();
            self.delay_l.write(write_l);
            self.delay_r.write(write_r);
        } else {
            let write_l = (left + filt_l * fb).tanh();
            let write_r = (right + filt_r * fb).tanh();
            self.delay_l.write(write_l);
            self.delay_r.write(write_r);
        }

        let out_l = dry_preserving_mix(left, read_l, mix);
        let out_r = dry_preserving_mix(right, read_r, mix);
        (out_l, out_r)
    }
}

// ============================================================================
// 5. REV — Smooth 8-Channel Hadamard FDN Algorithmic Stereo Reverb
// ============================================================================

const FDN_CHANNELS: usize = 8;
const DIFF_STAGES: usize = 4;

/// Base prime delay lengths (in ms) for the 8 FDN lines (mutually incommensurate).
const FDN_BASE_MS: [f32; FDN_CHANNELS] = [29.7, 37.1, 41.3, 47.9, 53.3, 61.7, 67.9, 73.3];
/// Input allpass diffusion lengths (in ms).
const DIFF_BASE_MS: [f32; DIFF_STAGES] = [4.77, 7.13, 11.29, 15.83];

struct AllpassDelay {
    line: DelayLine,
    delay_samples: f32,
}

impl AllpassDelay {
    fn new(delay_ms: f32, sr: f32) -> Self {
        let samp = (delay_ms * 0.001 * sr).max(8.0);
        Self {
            line: DelayLine::new(samp as usize + 32),
            delay_samples: samp,
        }
    }

    fn resize(&mut self, delay_ms: f32, sr: f32) {
        let samp = (delay_ms * 0.001 * sr).max(8.0);
        self.delay_samples = samp;
        self.line.resize(samp as usize + 32);
    }

    fn reset(&mut self) {
        self.line.reset();
    }

    #[inline]
    fn process(&mut self, x: f32, g: f32) -> f32 {
        let delayed = self.line.read_linear(self.delay_samples);
        let v = flush_denormal(x - g * delayed);
        self.line.write(v);
        flush_denormal(delayed + g * v)
    }
}

pub struct ReverbFx {
    sample_rate: f32,
    predelay_l: DelayLine,
    predelay_r: DelayLine,
    diff_l: [AllpassDelay; DIFF_STAGES],
    diff_r: [AllpassDelay; DIFF_STAGES],
    fdn_lines: [DelayLine; FDN_CHANNELS],
    fdn_damp: [OnePole; FDN_CHANNELS],
    mud_hp_l: OnePole,
    mud_hp_r: OnePole,
    color_lp_l: OnePole,
    color_lp_r: OnePole,
    color_hp_l: OnePole,
    color_hp_r: OnePole,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
    dc_coeff: f32,
    mod_phase: [f32; 4],
}

impl ReverbFx {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        let pre_cap = (0.28 * sr) as usize + 64;
        // Ambient mode scales FDN delays up to 1.65x -> allocate for 2.0x headroom
        let fdn_lines = std::array::from_fn(|i| {
            let cap = (FDN_BASE_MS[i] * 2.0 * 0.001 * sr) as usize + 128;
            DelayLine::new(cap)
        });
        let diff_l = std::array::from_fn(|i| AllpassDelay::new(DIFF_BASE_MS[i], sr));
        let diff_r = std::array::from_fn(|i| AllpassDelay::new(DIFF_BASE_MS[i] * 1.07, sr));

        Self {
            sample_rate: sr,
            predelay_l: DelayLine::new(pre_cap),
            predelay_r: DelayLine::new(pre_cap),
            diff_l,
            diff_r,
            fdn_lines,
            fdn_damp: [OnePole::default(); FDN_CHANNELS],
            mud_hp_l: OnePole::default(),
            mud_hp_r: OnePole::default(),
            color_lp_l: OnePole::default(),
            color_lp_r: OnePole::default(),
            color_hp_l: OnePole::default(),
            color_hp_r: OnePole::default(),
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
            dc_coeff: 1.0 - (TAU * 10.0 / sr).min(0.05),
            mod_phase: [0.0, 0.25, 0.5, 0.75],
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sanitize_sample_rate(sample_rate);
        self.sample_rate = sr;
        let pre_cap = (0.28 * sr) as usize + 64;
        self.predelay_l.resize(pre_cap);
        self.predelay_r.resize(pre_cap);
        for i in 0..DIFF_STAGES {
            self.diff_l[i].resize(DIFF_BASE_MS[i], sr);
            self.diff_r[i].resize(DIFF_BASE_MS[i] * 1.07, sr);
        }
        for i in 0..FDN_CHANNELS {
            let cap = (FDN_BASE_MS[i] * 2.0 * 0.001 * sr) as usize + 128;
            self.fdn_lines[i].resize(cap);
        }
        self.dc_coeff = 1.0 - (TAU * 10.0 / sr).min(0.05);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.predelay_l.reset();
        self.predelay_r.reset();
        for d in &mut self.diff_l {
            d.reset();
        }
        for d in &mut self.diff_r {
            d.reset();
        }
        for line in &mut self.fdn_lines {
            line.reset();
        }
        for d in &mut self.fdn_damp {
            d.reset();
        }
        self.mud_hp_l.reset();
        self.mud_hp_r.reset();
        self.color_lp_l.reset();
        self.color_lp_r.reset();
        self.color_hp_l.reset();
        self.color_hp_r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
    }

    /// Orthonormal 8x8 Walsh-Hadamard transform (normalized by 1 / sqrt(8)).
    #[inline]
    fn hadamard8(x: &mut [f32; 8]) {
        // Stage 1
        for i in (0..8).step_by(2) {
            let a = x[i];
            let b = x[i + 1];
            x[i] = a + b;
            x[i + 1] = a - b;
        }
        // Stage 2
        for i in (0..8).step_by(4) {
            for j in 0..2 {
                let a = x[i + j];
                let b = x[i + j + 2];
                x[i + j] = a + b;
                x[i + j + 2] = a - b;
            }
        }
        // Stage 3
        for j in 0..4 {
            let a = x[j];
            let b = x[j + 4];
            x[j] = a + b;
            x[j + 4] = a - b;
        }
        // Normalize by 1 / sqrt(8) ≈ 0.35355339
        const INV_SQRT_8: f32 = 0.353_553_38;
        for v in x.iter_mut() {
            *v = flush_denormal(*v * INV_SQRT_8);
        }
    }

    /// Mode character tuning: `(size_scale, diffusion_gain, mod_depth_ms, damp_bias)`.
    #[inline]
    fn mode_profile(mode: ReverbMode) -> (f32, f32, f32, f32) {
        match mode {
            ReverbMode::Plate1 => (0.85, 0.62, 0.35, 0.0),
            ReverbMode::Plate2 => (1.00, 0.68, 0.55, 0.15),
            ReverbMode::Hall => (1.25, 0.58, 0.65, 0.10),
            ReverbMode::Room => (0.58, 0.50, 0.20, 0.25),
            ReverbMode::Ambient => (1.60, 0.72, 1.10, -0.10),
        }
    }

    #[inline]
    pub fn process_sample(
        &mut self,
        left: f32,
        right: f32,
        params: &ReverbParams,
        mods: &GlobalModOffsets,
    ) -> (f32, f32) {
        if params.mute.value() {
            return (left, right);
        }
        let mix = (params.dry_wet.value() + mods.fx_reverb_mix).clamp(0.0, 1.0);
        if mix <= 1e-4 {
            return (left, right);
        }

        let sr = self.sample_rate;
        let (size_scale, diff_g, mod_ms, damp_bias) = Self::mode_profile(params.mode.value());
        let predelay_samp = (params.predelay.value().clamp(0.0, 250.0) * 0.001 * sr).max(1.0);
        let rt60 = (params.decay.value() * 2.0_f32.powf(mods.fx_reverb_decay * 2.0)).clamp(0.1, 20.0);
        let damp = (params.damp.value() + damp_bias).clamp(0.0, 0.98);
        let color = params.color.value().clamp(-1.0, 1.0);
        let wide = params.wide.value().clamp(0.0, 1.0);

        // 1. Pre-delay & subtle sub-bass mud filter so long tails never fog dry lows
        self.predelay_l.write(left);
        self.predelay_r.write(right);
        let mud_cut = if color < -0.2 { 75.0 } else { 110.0 };
        let mud_c = OnePole::lp_coeff(mud_cut, sr);
        let mut dl = self.mud_hp_l.process_hp(self.predelay_l.read_linear(predelay_samp), mud_c);
        let mut dr = self.mud_hp_r.process_hp(self.predelay_r.read_linear(predelay_samp), mud_c);

        // 2. Input diffusion allpass cascade
        for i in 0..DIFF_STAGES {
            dl = self.diff_l[i].process(dl, diff_g);
            dr = self.diff_r[i].process(dr, diff_g);
        }

        // 3. Advance subtle internal modulation LFOs (mutually prime slow rates)
        const MOD_RATES: [f32; 4] = [0.31, 0.47, 0.67, 0.89];
        let mut mod_offsets = [0.0_f32; 4];
        let mod_samp = mod_ms * 0.001 * sr;
        for i in 0..4 {
            mod_offsets[i] = (self.mod_phase[i] * TAU).sin() * mod_samp;
            self.mod_phase[i] = (self.mod_phase[i] + MOD_RATES[i] / sr).fract();
        }

        // 4. Read 8 FDN lines + apply damping LPF + RT60 gain
        let damp_cutoff = 18000.0 * 2.0_f32.powf(-damp * 4.5); // 18 kHz down to ~800 Hz
        let damp_coeff = OnePole::lp_coeff(damp_cutoff, sr);

        let mut taps = [0.0_f32; FDN_CHANNELS];
        for i in 0..FDN_CHANNELS {
            let base_delay_s = FDN_BASE_MS[i] * size_scale * 0.001;
            let mut delay_samp = base_delay_s * sr;
            if i < 4 {
                delay_samp += mod_offsets[i];
            }
            let raw = self.fdn_lines[i].read_linear(delay_samp.max(8.0));
            let damped = self.fdn_damp[i].process_lp(raw, damp_coeff);
            // Exact RT60 decay coefficient: g = 10^(-3 * T_delay / RT60)
            let g = (10.0_f32.powf(-3.0 * base_delay_s / rt60)).clamp(0.0, 0.992);
            taps[i] = flush_denormal(damped * g);
        }

        // 5. Orthogonal Hadamard feedback mixing
        let mut mixed = taps;
        Self::hadamard8(&mut mixed);

        // Inject diffused stereo input across the 8 channels with RT60 energy normalization
        // so long Ambient/Hall decays never swell louder than dry or drown out chords.
        let avg_delay_s = 0.050 * size_scale;
        let g_avg = (10.0_f32.powf(-3.0 * avg_delay_s / rt60)).clamp(0.1, 0.992);
        let rt_norm = (1.0 - g_avg * g_avg).max(0.03).sqrt();
        let in_scale = (0.64 * rt_norm).clamp(0.12, 0.44);
        self.fdn_lines[0].write(mixed[0] + dl * in_scale);
        self.fdn_lines[1].write(mixed[1] + dr * in_scale);
        self.fdn_lines[2].write(mixed[2] - dl * in_scale);
        self.fdn_lines[3].write(mixed[3] - dr * in_scale);
        self.fdn_lines[4].write(mixed[4] + (dl + dr) * 0.5 * in_scale);
        self.fdn_lines[5].write(mixed[5] + (dl - dr) * 0.5 * in_scale);
        self.fdn_lines[6].write(mixed[6] - (dl + dr) * 0.5 * in_scale);
        self.fdn_lines[7].write(mixed[7] - (dl - dr) * 0.5 * in_scale);

        // 6. Decorrelated stereo output taps
        let mut wet_l = (taps[0] + taps[2] - taps[4] + taps[6]) * 0.45;
        let mut wet_r = (taps[1] - taps[3] + taps[5] + taps[7]) * 0.45;

        // 7. Color tilt EQ (-1 = dark/warm, 0 = neutral, +1 = airy/bright)
        if color < -0.01 {
            let cut = 18000.0 * 2.0_f32.powf(color * 3.8);
            let c = OnePole::lp_coeff(cut, sr);
            wet_l = self.color_lp_l.process_lp(wet_l, c);
            wet_r = self.color_lp_r.process_lp(wet_r, c);
        } else if color > 0.01 {
            let cut = 40.0 * 2.0_f32.powf(color * 4.5);
            let c = OnePole::lp_coeff(cut, sr);
            let hi_l = self.color_hp_l.process_hp(wet_l, c);
            let hi_r = self.color_hp_r.process_hp(wet_r, c);
            wet_l = wet_l * (1.0 - color * 0.45) + hi_l * (0.45 * color + 0.35 * color);
            wet_r = wet_r * (1.0 - color * 0.45) + hi_r * (0.45 * color + 0.35 * color);
        }

        // DC block wet signal
        wet_l = self.dc_l.process(wet_l, self.dc_coeff);
        wet_r = self.dc_r.process(wet_r, self.dc_coeff);

        // 8. Stereo width (M/S)
        let mid = 0.5 * (wet_l + wet_r);
        let side = 0.5 * (wet_l - wet_r) * wide;
        wet_l = mid + side;
        wet_r = mid - side;

        let wet_trim = 0.72 + 0.28 * mix;
        let out_l = dry_preserving_mix(left, wet_l * wet_trim, mix);
        let out_r = dry_preserving_mix(right, wet_r * wet_trim, mix);
        (out_l, out_r)
    }
}

// ============================================================================
// Complete 5-Module Series FX Rack
// ============================================================================

/// Series FX Rack: `SHP -> PHS -> CHR -> DEL -> REV`.
pub struct FxRack {
    pub shaper: ShaperFx,
    pub phaser: PhaserFx,
    pub chorus: ChorusFx,
    pub delay: DelayFx,
    pub reverb: ReverbFx,
}

impl Default for FxRack {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl FxRack {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        Self {
            shaper: ShaperFx::new(sr),
            phaser: PhaserFx::new(sr),
            chorus: ChorusFx::new(sr),
            delay: DelayFx::new(sr),
            reverb: ReverbFx::new(sr),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sanitize_sample_rate(sample_rate);
        self.shaper.set_sample_rate(sr);
        self.phaser.set_sample_rate(sr);
        self.chorus.set_sample_rate(sr);
        self.delay.set_sample_rate(sr);
        self.reverb.set_sample_rate(sr);
    }

    pub fn reset(&mut self) {
        self.shaper.reset();
        self.phaser.reset();
        self.chorus.reset();
        self.delay.reset();
        self.reverb.reset();
    }

    #[inline]
    pub fn process_sample(
        &mut self,
        left: f32,
        right: f32,
        params: &FxParams,
        mods: &GlobalModOffsets,
        bpm: f32,
    ) -> (f32, f32) {
        let (l, r) = self.shaper.process_sample(left, right, &params.shaper, mods);
        let (l, r) = self.phaser.process_sample(l, r, &params.phaser, mods);
        let (l, r) = self.chorus.process_sample(l, r, &params.chorus, mods);
        let (l, r) = self.delay.process_sample(l, r, &params.delay, mods, bpm);
        self.reverb.process_sample(l, r, &params.reverb, mods)
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{set_bool_param, set_enum_param, set_float_param};

    fn rms_diff(a: &[(f32, f32)], b: &[(f32, f32)]) -> f32 {
        let sum: f32 = a
            .iter()
            .zip(b.iter())
            .map(|((al, ar), (bl, br))| (al - bl).powi(2) + (ar - br).powi(2))
            .sum();
        (sum / (a.len() as f32 * 2.0)).sqrt()
    }

    fn gen_test_signal(n: usize, sr: f32) -> Vec<(f32, f32)> {
        (0..n)
            .map(|i| {
                let t = i as f32 / sr;
                let s = 0.4 * (TAU * 220.0 * t).sin() + 0.2 * (TAU * 880.0 * t).sin();
                (s, s)
            })
            .collect()
    }

    #[test]
    fn shaper_modes_band_hq_bit_srate_and_mute_produce_measurable_differences() {
        let sr = 48000.0;
        let dry = gen_test_signal(4096, sr);
        let mods = GlobalModOffsets::default();

        // Mute bypass must be bit-exact dry
        let mut p_mute = ShaperParams::default();
        set_float_param(&mut p_mute.dry_wet, 1.0);
        set_bool_param(&mut p_mute.mute, true);
        let mut shp = ShaperFx::new(sr);
        let out_mute: Vec<_> = dry
            .iter()
            .map(|&(l, r)| shp.process_sample(l, r, &p_mute, &mods))
            .collect();
        assert_eq!(rms_diff(&dry, &out_mute), 0.0);

        // Verify all 8 modes alter audio cleanly with zero NaN/Inf
        let modes = [
            ShaperMode::Warm,
            ShaperMode::Soft,
            ShaperMode::Hard,
            ShaperMode::Clip,
            ShaperMode::Tube,
            ShaperMode::Fold,
            ShaperMode::Sine,
            ShaperMode::Bitcrush,
        ];
        let mut mode_outs = Vec::new();
        for &mode in &modes {
            let mut p = ShaperParams::default();
            set_enum_param(&mut p.mode, mode);
            set_float_param(&mut p.drive, 0.7);
            set_float_param(&mut p.dry_wet, 1.0);
            let mut fx = ShaperFx::new(sr);
            let out: Vec<_> = dry
                .iter()
                .map(|&(l, r)| fx.process_sample(l, r, &p, &mods))
                .collect();
            assert!(out.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
            assert!(
                rms_diff(&dry, &out) > 0.05,
                "Shaper mode {mode:?} did not measurably shape audio"
            );
            mode_outs.push(out);
        }

        // Verify BAND toggle and HQ toggle produce measurable differences
        let mut p_base = ShaperParams::default();
        set_enum_param(&mut p_base.mode, ShaperMode::Fold);
        set_float_param(&mut p_base.drive, 0.8);
        set_float_param(&mut p_base.low_cut, 600.0);
        set_float_param(&mut p_base.dry_wet, 1.0);

        let mut fx_a = ShaperFx::new(sr);
        let out_a: Vec<_> = dry
            .iter()
            .map(|&(l, r)| fx_a.process_sample(l, r, &p_base, &mods))
            .collect();

        set_bool_param(&mut p_base.band, true);
        let mut fx_band = ShaperFx::new(sr);
        let out_band: Vec<_> = dry
            .iter()
            .map(|&(l, r)| fx_band.process_sample(l, r, &p_base, &mods))
            .collect();
        assert!(rms_diff(&out_a, &out_band) > 0.02, "BAND mode should differ from broadband");

        set_bool_param(&mut p_base.band, false);
        set_bool_param(&mut p_base.hq, true);
        let mut fx_hq = ShaperFx::new(sr);
        let out_hq: Vec<_> = dry
            .iter()
            .map(|&(l, r)| fx_hq.process_sample(l, r, &p_base, &mods))
            .collect();
        assert!(rms_diff(&out_a, &out_hq) > 0.01, "HQ 2x oversampling should differ from 1x");

        // Verify bit & s_rate crush
        let mut p_crush = ShaperParams::default();
        set_float_param(&mut p_crush.drive, 0.0);
        set_float_param(&mut p_crush.bit, 0.8);
        set_float_param(&mut p_crush.s_rate, 0.6);
        set_float_param(&mut p_crush.dry_wet, 1.0);
        let mut fx_c = ShaperFx::new(sr);
        let out_c: Vec<_> = dry
            .iter()
            .map(|&(l, r)| fx_c.process_sample(l, r, &p_crush, &mods))
            .collect();
        assert!(rms_diff(&dry, &out_c) > 0.05);
    }

    #[test]
    fn phaser_chorus_delay_and_reverb_all_work_across_sample_rates() {
        for sr in [44100.0_f32, 48000.0, 96000.0] {
            let n = (0.35 * sr) as usize;
            let mut input = gen_test_signal(n, sr);
            // Make second half silent to test Delay & Reverb tails
            for s in input.iter_mut().skip(n / 2) {
                *s = (0.0, 0.0);
            }
            let mods = GlobalModOffsets::default();

            // 1. Phaser
            let mut phs_p = PhaserParams::default();
            set_float_param(&mut phs_p.dry_wet, 0.8);
            set_float_param(&mut phs_p.feedback, 0.65);
            let mut phs = PhaserFx::new(sr);
            let phs_out: Vec<_> = input
                .iter()
                .map(|&(l, r)| phs.process_sample(l, r, &phs_p, &mods))
                .collect();
            assert!(phs_out.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
            assert!(rms_diff(&input, &phs_out) > 0.03, "Phaser must affect signal at sr={sr}");

            // 2. Chorus (all 3 modes)
            for mode in [ChorusMode::Chorus, ChorusMode::Ensemble, ChorusMode::Dimension] {
                let mut chr_p = ChorusParams::default();
                set_enum_param(&mut chr_p.mode, mode);
                set_float_param(&mut chr_p.dry_wet, 0.7);
                let mut chr = ChorusFx::new(sr);
                let chr_out: Vec<_> = input
                    .iter()
                    .map(|&(l, r)| chr.process_sample(l, r, &chr_p, &mods))
                    .collect();
                assert!(chr_out.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
                assert!(rms_diff(&input, &chr_out) > 0.03, "Chorus {mode:?} failed at sr={sr}");
            }

            // 3. Delay (ping-pong creates measurable stereo tail in silent second half)
            let mut del_p = DelayParams::default();
            set_bool_param(&mut del_p.sync, false);
            set_float_param(&mut del_p.time_l, 40.0);
            set_float_param(&mut del_p.time_r, 60.0);
            set_float_param(&mut del_p.feedback, 0.5);
            set_bool_param(&mut del_p.ping_pong, true);
            set_float_param(&mut del_p.dry_wet, 0.6);
            let mut del = DelayFx::new(sr);
            let del_out: Vec<_> = input
                .iter()
                .map(|&(l, r)| del.process_sample(l, r, &del_p, &mods, 120.0))
                .collect();
            assert!(del_out.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
            let tail_energy: f32 = del_out[(n * 3 / 5)..]
                .iter()
                .map(|(l, r)| l * l + r * r)
                .sum();
            assert!(tail_energy > 0.5, "Delay tail missing at sr={sr} (energy={tail_energy})");

            // 4. Reverb (produces smooth lush stereo tail in silent second half)
            let mut rev_p = ReverbParams::default();
            set_enum_param(&mut rev_p.mode, ReverbMode::Hall);
            set_float_param(&mut rev_p.predelay, 10.0);
            set_float_param(&mut rev_p.decay, 3.0);
            set_float_param(&mut rev_p.wide, 1.0);
            set_float_param(&mut rev_p.dry_wet, 0.6);
            let mut rev = ReverbFx::new(sr);
            let rev_out: Vec<_> = input
                .iter()
                .map(|&(l, r)| rev.process_sample(l, r, &rev_p, &mods))
                .collect();
            assert!(rev_out.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
            let rev_tail: f32 = rev_out[(n * 3 / 5)..]
                .iter()
                .map(|(l, r)| l * l + r * r)
                .sum();
            let stereo_diff: f32 = rev_out[(n * 3 / 5)..]
                .iter()
                .map(|(l, r)| (l - r).abs())
                .sum();
            assert!(rev_tail > 0.5, "Reverb tail missing at sr={sr} (energy={rev_tail})");
            assert!(stereo_diff > 1.0, "Reverb wide=1.0 should be stereo decorrelated at sr={sr}");
        }
    }

    #[test]
    fn delay_tempo_sync_scales_with_bpm_90_to_160_and_handles_rapid_bpm_changes() {
        let sr = 48000.0;
        let mods = GlobalModOffsets::default();

        let mut del_p = DelayParams::default();
        set_bool_param(&mut del_p.sync, true);
        set_enum_param(&mut del_p.sync_div_l, crate::params::SyncDiv::D1_8);
        set_enum_param(&mut del_p.sync_div_r, crate::params::SyncDiv::D1_8);
        set_bool_param(&mut del_p.ping_pong, false);
        set_float_param(&mut del_p.feedback, 0.0);
        set_float_param(&mut del_p.depth, 0.0);
        set_float_param(&mut del_p.dry_wet, 1.0);
        set_float_param(&mut del_p.hi_cut, 20000.0);
        set_float_param(&mut del_p.low_cut, 20.0);

        // Find first echo peak sample index for a single impulse after settling delay slew
        let find_echo_sample = |bpm: f32| -> usize {
            let mut del = DelayFx::new(sr);
            // Settle slew at target BPM with silence
            for _ in 0..((0.35 * sr) as usize) {
                let _ = del.process_sample(0.0, 0.0, &del_p, &mods, bpm);
            }
            let mut peak_idx = 0usize;
            let mut peak_val = 0.0_f32;
            for i in 0..((0.5 * sr) as usize) {
                let x = if i == 0 { 1.0 } else { 0.0 };
                let (l, r) = del.process_sample(x, x, &del_p, &mods, bpm);
                assert!(l.is_finite() && r.is_finite());
                if i > 100 && l.abs() > peak_val {
                    peak_val = l.abs();
                    peak_idx = i;
                }
            }
            assert!(peak_val > 0.3, "Echo peak too weak at bpm={bpm}: {peak_val}");
            peak_idx
        };

        // 1/8 note = 0.5 beat:
        // At 90 BPM: 0.5 * 60 / 90 = 0.3333 s -> ~16000 samples
        // At 160 BPM: 0.5 * 60 / 160 = 0.1875 s -> ~9000 samples
        let echo_90 = find_echo_sample(90.0);
        let echo_160 = find_echo_sample(160.0);
        assert!(
            (echo_90 as i32 - 16000).abs() < 150,
            "90 BPM 1/8 delay expected ~16000 samples, got {echo_90}"
        );
        assert!(
            (echo_160 as i32 - 9000).abs() < 150,
            "160 BPM 1/8 delay expected ~9000 samples, got {echo_160}"
        );

        // Rapid BPM sweeps (including NaN / extremes) produce finite, smooth, bounded audio
        let mut del = DelayFx::new(sr);
        set_float_param(&mut del_p.feedback, 0.55);
        for (idx, &b) in [90.0_f32, 160.0, 40.0, 240.0, f32::NAN, 128.0].iter().enumerate() {
            for s in 0..4000 {
                let x = if s < 500 {
                    0.3 * (TAU * 330.0 * s as f32 / sr).sin()
                } else {
                    0.0
                };
                let bpm = if b.is_finite() { b + idx as f32 } else { b };
                let (l, r) = del.process_sample(x, x, &del_p, &mods, bpm);
                assert!(l.is_finite() && r.is_finite() && l.abs() <= 2.0 && r.abs() <= 2.0);
            }
        }
    }

    #[test]
    fn chorus_and_reverb_preserve_dry_signal_and_mono_phase_coherence() {
        let sr = 48000.0;
        let mods = GlobalModOffsets::default();
        let dry = gen_test_signal(16384, sr);
        let mono_dry: Vec<f32> = dry[4096..].iter().map(|&(l, r)| 0.5 * (l + r)).collect();
        let rms_mono = |buf: &[f32]| -> f32 {
            (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt()
        };
        let dry_rms = rms_mono(&mono_dry);

        for mode in [ChorusMode::Chorus, ChorusMode::Ensemble, ChorusMode::Dimension] {
            let mut p = ChorusParams::default();
            set_enum_param(&mut p.mode, mode);
            set_float_param(&mut p.dry_wet, 0.45);
            let mut chr = ChorusFx::new(sr);
            let out: Vec<(f32, f32)> = dry
                .iter()
                .map(|&(l, r)| chr.process_sample(l, r, &p, &mods))
                .collect();
            let mono_out: Vec<f32> = out[4096..].iter().map(|&(l, r)| 0.5 * (l + r)).collect();
            let out_rms = rms_mono(&mono_out);
            assert!(
                out_rms >= dry_rms * 0.65 && out_rms <= dry_rms * 1.45,
                "Chorus {mode:?} mono collapse out of healthy range: out={out_rms} dry={dry_rms}"
            );
        }

        // Long Ambient reverb @ 40% mix must preserve dry clarity & mono energy
        let mut rp = ReverbParams::default();
        set_enum_param(&mut rp.mode, ReverbMode::Ambient);
        set_float_param(&mut rp.decay, 8.0);
        set_float_param(&mut rp.wide, 1.0);
        set_float_param(&mut rp.dry_wet, 0.40);
        let mut rev = ReverbFx::new(sr);
        let rev_out: Vec<(f32, f32)> = dry
            .iter()
            .map(|&(l, r)| rev.process_sample(l, r, &rp, &mods))
            .collect();
        let rev_mono: Vec<f32> = rev_out[4096..].iter().map(|&(l, r)| 0.5 * (l + r)).collect();
        let rev_rms = rms_mono(&rev_mono);
        assert!(
            rev_rms >= dry_rms * 0.72 && rev_rms <= dry_rms * 1.55,
            "Ambient reverb mono collapse or drowning failed: rev={rev_rms} dry={dry_rms}"
        );
    }
}
