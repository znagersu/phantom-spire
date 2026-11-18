//! Master section DSP for Phantom Spire:
//! - `XComp`: Spire-style 3-band upward/downward OTT-like exciter compressor
//! - `warm` / `soft` / `boost` analog character processors
//! - 3-band parametric `EQ` (`Low` shelf, `Mid` peak, `High` shelf with `frq`, `Q`, `level` + bypass)
//! - `apply_velocity_curve`: Master velocity sensitivity mapping
//! - Master volume + global pan modulation
//! - 10 Hz stereo DC blocker (`|DC| <= 0.005`)
//! - Brickwall safety limiter (`peak <= 0.99`)
//!
//! Real-time safe: zero heap allocations, no panics, finite & denormal-free at
//! 44.1 kHz, 48 kHz, and 96 kHz.

use crate::dsp::mod_engine::{flush_denormal, sanitize_sample_rate, GlobalModOffsets};
use crate::params::MasterParams;
use std::f32::consts::{FRAC_PI_2, TAU};

// ============================================================================
// Master Velocity Curve Helper
// ============================================================================

/// Map incoming MIDI velocity (`0.0..=1.0`) through the Master `velocity` sensitivity knob.
/// - `sensitivity = 0.0`: fixed full velocity (`1.0`).
/// - `sensitivity = 1.0`: full dynamic range (`raw_velocity`).
#[inline]
pub fn apply_velocity_curve(raw_velocity: f32, sensitivity: f32) -> f32 {
    let v = if raw_velocity.is_finite() {
        raw_velocity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let s = if sensitivity.is_finite() {
        sensitivity.clamp(0.0, 1.0)
    } else {
        0.75
    };
    (1.0 - s + s * v).clamp(0.01, 1.0)
}

// ============================================================================
// Stereo Biquad & RBJ Audio EQ Cookbook Filters
// ============================================================================

#[derive(Debug, Clone, Copy)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Default for BiquadCoeffs {
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }
}

impl BiquadCoeffs {
    fn low_shelf(freq_hz: f32, q: f32, gain_db: f32, sr: f32) -> Self {
        if gain_db.abs() < 0.01 {
            return Self::default();
        }
        let fc = freq_hz.clamp(20.0, sr * 0.45);
        let q = q.clamp(0.2, 8.0);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = TAU * fc / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        Self::normalize(b0, b1, b2, a0, a1, a2)
    }

    fn peaking(freq_hz: f32, q: f32, gain_db: f32, sr: f32) -> Self {
        if gain_db.abs() < 0.01 {
            return Self::default();
        }
        let fc = freq_hz.clamp(20.0, sr * 0.45);
        let q = q.clamp(0.2, 12.0);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = TAU * fc / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;
        Self::normalize(b0, b1, b2, a0, a1, a2)
    }

    fn high_shelf(freq_hz: f32, q: f32, gain_db: f32, sr: f32) -> Self {
        if gain_db.abs() < 0.01 {
            return Self::default();
        }
        let fc = freq_hz.clamp(20.0, sr * 0.45);
        let q = q.clamp(0.2, 8.0);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = TAU * fc / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        Self::normalize(b0, b1, b2, a0, a1, a2)
    }

    fn lowpass_lr2(freq_hz: f32, sr: f32) -> Self {
        // 2nd-order Linkwitz-Riley / critically damped Q = 0.50 -> LR2 crossover
        let fc = freq_hz.clamp(30.0, sr * 0.42);
        let w0 = TAU * fc / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * 0.707_106_77);
        let b0 = (1.0 - cos_w0) * 0.5;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::normalize(b0, b1, b2, a0, a1, a2)
    }

    #[inline]
    fn normalize(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        if !a0.is_finite() || a0.abs() < 1e-9 {
            return Self::default();
        }
        let inv = 1.0 / a0;
        Self {
            b0: b0 * inv,
            b1: b1 * inv,
            b2: b2 * inv,
            a1: a1 * inv,
            a2: a2 * inv,
        }
    }

    /// Evaluate exact magnitude response (linear) at `freq_hz`.
    fn magnitude(&self, freq_hz: f32, sr: f32) -> f32 {
        let w = TAU * freq_hz.clamp(1.0, sr * 0.49) / sr;
        let cos1 = w.cos();
        let sin1 = w.sin();
        let cos2 = (2.0 * w).cos();
        let sin2 = (2.0 * w).sin();

        let num_re = self.b0 + self.b1 * cos1 + self.b2 * cos2;
        let num_im = -(self.b1 * sin1 + self.b2 * sin2);
        let den_re = 1.0 + self.a1 * cos1 + self.a2 * cos2;
        let den_im = -(self.a1 * sin1 + self.a2 * sin2);

        let num_sq = num_re * num_re + num_im * num_im;
        let den_sq = (den_re * den_re + den_im * den_im).max(1e-12);
        (num_sq / den_sq).sqrt()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct StereoBiquad {
    coeffs: BiquadCoeffs,
    z1_l: f32,
    z2_l: f32,
    z1_r: f32,
    z2_r: f32,
}

impl StereoBiquad {
    fn reset(&mut self) {
        self.z1_l = 0.0;
        self.z2_l = 0.0;
        self.z1_r = 0.0;
        self.z2_r = 0.0;
    }

    #[inline]
    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let c = &self.coeffs;
        let yl = flush_denormal(c.b0 * l + self.z1_l);
        self.z1_l = flush_denormal(c.b1 * l - c.a1 * yl + self.z2_l);
        self.z2_l = flush_denormal(c.b2 * l - c.a2 * yl);

        let yr = flush_denormal(c.b0 * r + self.z1_r);
        self.z1_r = flush_denormal(c.b1 * r - c.a1 * yr + self.z2_r);
        self.z2_r = flush_denormal(c.b2 * r - c.a2 * yr);
        (yl, yr)
    }
}

// ============================================================================
// 3-Band Parametric Master EQ (with Coefficient Caching)
// ============================================================================

pub struct MasterEq {
    sample_rate: f32,
    low: StereoBiquad,
    mid: StereoBiquad,
    high: StereoBiquad,
    cached_low: (f32, f32, f32),
    cached_mid: (f32, f32, f32),
    cached_high: (f32, f32, f32),
}

impl MasterEq {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        Self {
            sample_rate: sr,
            low: StereoBiquad::default(),
            mid: StereoBiquad::default(),
            high: StereoBiquad::default(),
            cached_low: (-1.0, -1.0, 999.0),
            cached_mid: (-1.0, -1.0, 999.0),
            cached_high: (-1.0, -1.0, 999.0),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitize_sample_rate(sample_rate);
        self.cached_low = (-1.0, -1.0, 999.0);
        self.cached_mid = (-1.0, -1.0, 999.0);
        self.cached_high = (-1.0, -1.0, 999.0);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.low.reset();
        self.mid.reset();
        self.high.reset();
    }

    #[inline]
    fn update_coeffs_if_needed(&mut self, params: &MasterParams) {
        let lf = params.eq_low_freq.value();
        let lq = params.eq_low_q.value();
        let lg = params.eq_low_gain.value();
        if (lf - self.cached_low.0).abs() > 1e-3
            || (lq - self.cached_low.1).abs() > 1e-4
            || (lg - self.cached_low.2).abs() > 1e-4
        {
            self.low.coeffs = BiquadCoeffs::low_shelf(lf, lq, lg, self.sample_rate);
            self.cached_low = (lf, lq, lg);
        }

        let mf = params.eq_mid_freq.value();
        let mq = params.eq_mid_q.value();
        let mg = params.eq_mid_gain.value();
        if (mf - self.cached_mid.0).abs() > 1e-3
            || (mq - self.cached_mid.1).abs() > 1e-4
            || (mg - self.cached_mid.2).abs() > 1e-4
        {
            self.mid.coeffs = BiquadCoeffs::peaking(mf, mq, mg, self.sample_rate);
            self.cached_mid = (mf, mq, mg);
        }

        let hf = params.eq_high_freq.value();
        let hq = params.eq_high_q.value();
        let hg = params.eq_high_gain.value();
        if (hf - self.cached_high.0).abs() > 1e-3
            || (hq - self.cached_high.1).abs() > 1e-4
            || (hg - self.cached_high.2).abs() > 1e-4
        {
            self.high.coeffs = BiquadCoeffs::high_shelf(hf, hq, hg, self.sample_rate);
            self.cached_high = (hf, hq, hg);
        }
    }

    #[inline]
    pub fn process_sample(&mut self, left: f32, right: f32, params: &MasterParams) -> (f32, f32) {
        if !params.eq_on.value() {
            return (left, right);
        }
        self.update_coeffs_if_needed(params);
        let (l, r) = self.low.process(left, right);
        let (l, r) = self.mid.process(l, r);
        self.high.process(l, r)
    }

    /// Evaluate combined 3-band EQ magnitude response in dB at `freq_hz` (for UI curve plotting).
    pub fn magnitude_db(freq_hz: f32, sample_rate: f32, params: &MasterParams) -> f32 {
        if !params.eq_on.value() {
            return 0.0;
        }
        let sr = sanitize_sample_rate(sample_rate);
        let low = BiquadCoeffs::low_shelf(
            params.eq_low_freq.value(),
            params.eq_low_q.value(),
            params.eq_low_gain.value(),
            sr,
        );
        let mid = BiquadCoeffs::peaking(
            params.eq_mid_freq.value(),
            params.eq_mid_q.value(),
            params.eq_mid_gain.value(),
            sr,
        );
        let high = BiquadCoeffs::high_shelf(
            params.eq_high_freq.value(),
            params.eq_high_q.value(),
            params.eq_high_gain.value(),
            sr,
        );
        let mag = low.magnitude(freq_hz, sr) * mid.magnitude(freq_hz, sr) * high.magnitude(freq_hz, sr);
        20.0 * mag.max(1e-6).log10()
    }
}

// ============================================================================
// X-Comp — Spire-Style 3-Band Upward/Downward OTT-like Exciter Compressor
// ============================================================================

pub struct XComp {
    sample_rate: f32,
    lp_low: StereoBiquad,
    lp_mid: StereoBiquad,
    env: [f32; 3],
    att_coeff: f32,
    rel_coeff: f32,
}

impl XComp {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        let mut x = Self {
            sample_rate: sr,
            lp_low: StereoBiquad::default(),
            lp_mid: StereoBiquad::default(),
            env: [0.0; 3],
            att_coeff: 0.0,
            rel_coeff: 0.0,
        };
        x.recompute_filters();
        x
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sanitize_sample_rate(sample_rate);
        self.recompute_filters();
        self.reset();
    }

    pub fn reset(&mut self) {
        self.lp_low.reset();
        self.lp_mid.reset();
        self.env = [0.0; 3];
    }

    fn recompute_filters(&mut self) {
        let sr = self.sample_rate;
        self.lp_low.coeffs = BiquadCoeffs::lowpass_lr2(220.0, sr);
        self.lp_mid.coeffs = BiquadCoeffs::lowpass_lr2(2800.0, sr);
        // Fast punchy attack (4 ms), musical glue release (75 ms)
        self.att_coeff = 1.0 - (-1.0 / (0.004 * sr).max(1.0)).exp();
        self.rel_coeff = 1.0 - (-1.0 / (0.075 * sr).max(1.0)).exp();
    }

    #[inline]
    fn compute_band_gain(env_lin: f32, up_thresh_db: f32, down_thresh_db: f32, makeup_db: f32) -> f32 {
        // Noise floor gate below -58 dBFS so silence never pumps noise
        if env_lin < 0.0012 {
            return 1.0;
        }
        let level_db = 20.0 * env_lin.max(1e-6).log10();

        let delta_db = if level_db > down_thresh_db {
            // Downward compression (4.5:1 ratio)
            let over = level_db - down_thresh_db;
            -(over * (1.0 - 1.0 / 4.5))
        } else if level_db < up_thresh_db {
            // Upward OTT-style compression (lifts quiet detail smoothly, capped at +11 dB)
            // Softly fades out near -58 dBFS noise floor
            let gate_fade = ((level_db + 58.0) / 14.0).clamp(0.0, 1.0);
            let under = (up_thresh_db - level_db).min(24.0);
            (under * 0.55).min(11.0) * gate_fade
        } else {
            0.0
        };

        let total_db = (delta_db + makeup_db).clamp(-18.0, 14.0);
        10.0_f32.powf(total_db / 20.0)
    }

    #[inline]
    pub fn process_sample(&mut self, left: f32, right: f32, amount: f32) -> (f32, f32) {
        let amt = amount.clamp(0.0, 1.0);
        if amt <= 1e-4 {
            return (left, right);
        }

        // Complementary 3-band split: Low + Mid + High == Input (exact flat reconstruction)
        let (low_l, low_r) = self.lp_low.process(left, right);
        let (low_mid_l, low_mid_r) = self.lp_mid.process(left, right);
        let mid_l = low_mid_l - low_l;
        let mid_r = low_mid_r - low_r;
        let high_l = left - low_mid_l;
        let high_r = right - low_mid_r;

        let bands = [(low_l, low_r), (mid_l, mid_r), (high_l, high_r)];
        // Per-band (upward thresh, downward thresh, makeup) tuned for Spire EDM sheen
        const THRESHOLDS: [(f32, f32, f32); 3] = [
            (-26.0, -13.0, 1.5), // Low: tight punch
            (-28.0, -14.0, 1.8), // Mid: forward presence
            (-30.0, -16.0, 2.5), // High: Spire silky air excitement
        ];

        let mut wet_l = 0.0_f32;
        let mut wet_r = 0.0_f32;

        for b in 0..3 {
            let (bl, br) = bands[b];
            let peak = bl.abs().max(br.abs());
            let coeff = if peak > self.env[b] {
                self.att_coeff
            } else {
                self.rel_coeff
            };
            self.env[b] = flush_denormal(self.env[b] + (peak - self.env[b]) * coeff);

            let (up_t, dn_t, mk) = THRESHOLDS[b];
            let g = Self::compute_band_gain(self.env[b], up_t, dn_t, mk);
            wet_l += bl * g;
            wet_r += br * g;
        }

        // Gentle glue soft-clip on compressed wet bus
        wet_l = (wet_l * 0.85).tanh() * 1.1;
        wet_r = (wet_r * 0.85).tanh() * 1.1;

        let out_l = flush_denormal(left + (wet_l - left) * amt);
        let out_r = flush_denormal(right + (wet_r - right) * amt);
        (out_l, out_r)
    }
}

// ============================================================================
// Master Section (X-Comp -> EQ -> Warm/Soft/Boost -> Vol/Pan -> DC Block -> Limiter)
// ============================================================================

#[derive(Debug, Clone, Copy, Default)]
struct DcBlocker {
    x1: f32,
    y1: f32,
}

impl DcBlocker {
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

#[inline]
fn soft_knee_sample(x: f32, knee: f32, span: f32) -> f32 {
    if x.abs() > knee {
        x.signum() * (knee + span * ((x.abs() - knee) / span).tanh())
    } else {
        x
    }
}

pub struct MasterSection {
    sample_rate: f32,
    pub x_comp: XComp,
    pub eq: MasterEq,
    // Warm analog tone filters
    warm_low: StereoBiquad,
    warm_hi: StereoBiquad,
    // Boost high-air shelf
    boost_air: StereoBiquad,
    // Output DC blocker
    dc_l: DcBlocker,
    dc_r: DcBlocker,
    dc_r_coeff: f32,
    // Safety limiter envelope
    lim_env: f32,
    lim_att_coeff: f32,
    lim_rel_coeff: f32,
}

impl Default for MasterSection {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl MasterSection {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sanitize_sample_rate(sample_rate);
        let mut m = Self {
            sample_rate: sr,
            x_comp: XComp::new(sr),
            eq: MasterEq::new(sr),
            warm_low: StereoBiquad::default(),
            warm_hi: StereoBiquad::default(),
            boost_air: StereoBiquad::default(),
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
            dc_r_coeff: 0.9985,
            lim_env: 0.0,
            lim_att_coeff: 0.1,
            lim_rel_coeff: 0.001,
        };
        m.recompute_character_filters();
        m
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sanitize_sample_rate(sample_rate);
        self.sample_rate = sr;
        self.x_comp.set_sample_rate(sr);
        self.eq.set_sample_rate(sr);
        self.recompute_character_filters();
        self.reset();
    }

    pub fn reset(&mut self) {
        self.x_comp.reset();
        self.eq.reset();
        self.warm_low.reset();
        self.warm_hi.reset();
        self.boost_air.reset();
        self.dc_l.reset();
        self.dc_r.reset();
        self.lim_env = 0.0;
    }

    fn recompute_character_filters(&mut self) {
        let sr = self.sample_rate;
        self.warm_low.coeffs = BiquadCoeffs::low_shelf(160.0, 0.7, 2.2, sr);
        self.warm_hi.coeffs = BiquadCoeffs::high_shelf(9500.0, 0.7, -1.8, sr);
        self.boost_air.coeffs = BiquadCoeffs::high_shelf(5500.0, 0.7, 2.4, sr);
        // 10 Hz DC blocker
        self.dc_r_coeff = (1.0 - TAU * 10.0 / sr).clamp(0.95, 0.9999);
        // Smooth 0.3 ms attack / 75 ms release soft-knee limiter envelope
        self.lim_att_coeff = 1.0 - (-1.0 / (0.0003 * sr).max(1.0)).exp();
        self.lim_rel_coeff = 1.0 - (-1.0 / (0.075 * sr).max(1.0)).exp();
    }

    /// Process one stereo sample through the full Master chain.
    ///
    /// Pass `volume_gain` from `params.master.volume.smoothed.next()` (for zipper-free volume
    /// automation in `process()`), or `params.master.volume.value()` in tests.
    #[inline]
    pub fn process_sample(
        &mut self,
        left: f32,
        right: f32,
        params: &MasterParams,
        mods: &GlobalModOffsets,
        volume_gain: f32,
    ) -> (f32, f32) {
        let mut l = if left.is_finite() { left } else { 0.0 };
        let mut r = if right.is_finite() { right } else { 0.0 };

        // 1. X-Comp (Spire-style 3-band upward/downward exciter compressor)
        let (xl, xr) = self.x_comp.process_sample(l, r, params.x_comp.value());
        l = xl;
        r = xr;

        // 2. 3-Band Parametric EQ (Low shelf, Mid peak, High shelf, w/ eq_on bypass)
        let (el, er) = self.eq.process_sample(l, r, params);
        l = el;
        r = er;

        // 3. Character Toggles (Warm / Soft / Boost)
        if params.warm.value() {
            let (wl, wr) = self.warm_low.process(l, r);
            let (wl, wr) = self.warm_hi.process(wl, wr);
            // Subtle even-harmonic analog transformer saturation
            l = (wl + 0.08 * wl * wl).tanh();
            r = (wr + 0.08 * wr * wr).tanh();
        }

        if params.boost.value() {
            let (bl, br) = self.boost_air.process(l, r);
            // Spire output punch maximizer (+4.5 dB drive into smooth knee)
            let drive = 1.68;
            l = (bl * drive).tanh() * 0.94;
            r = (br * drive).tanh() * 0.94;
        }

        if params.soft.value() {
            // Silky soft-clipping saturator rounding digital transients
            let norm = 1.0 / 1.35_f32.tanh();
            l = (l * 1.35).tanh() * norm * 0.96;
            r = (r * 1.35).tanh() * norm * 0.96;
        }

        // 4. Master Volume + Modulated Gain + Global Pan
        let base_vol = if volume_gain.is_finite() {
            volume_gain.clamp(0.0, 4.0)
        } else {
            0.7
        };
        let mod_vol_scale = (1.0 + mods.master_volume).clamp(0.0, 2.0);
        let gain = base_vol * mod_vol_scale;
        l *= gain;
        r *= gain;

        let pan = mods.global_pan.clamp(-1.0, 1.0);
        if pan.abs() > 1e-4 {
            let angle = (pan + 1.0) * 0.5 * FRAC_PI_2;
            let pan_l = angle.cos() * std::f32::consts::SQRT_2;
            let pan_r = angle.sin() * std::f32::consts::SQRT_2;
            l *= pan_l;
            r *= pan_r;
        }

        // 5. DC Blocker (guarantees |DC| <= 0.005 even with asymmetric warmth/shaper)
        l = self.dc_l.process(l, self.dc_r_coeff);
        r = self.dc_r.process(r, self.dc_r_coeff);

        // 6. Gentle Soft-Knee Output Limiter & Saturation (keeps chords clean & peak <= 0.99)
        let peak = l.abs().max(r.abs());
        if params.soft_limit.value() {
            let coeff = if peak > self.lim_env {
                self.lim_att_coeff
            } else {
                self.lim_rel_coeff
            };
            self.lim_env = flush_denormal(self.lim_env + (peak - self.lim_env) * coeff);

            const LIM_THRESH: f32 = 0.84;
            const LIM_HEADROOM: f32 = 0.08; // Asymptotic ceiling 0.92
            if self.lim_env > LIM_THRESH {
                let over = self.lim_env - LIM_THRESH;
                let target_env = LIM_THRESH + LIM_HEADROOM * (over / LIM_HEADROOM).tanh();
                let lim_gain = target_env / self.lim_env;
                l *= lim_gain;
                r *= lim_gain;
            }
            // Transparent soft-knee transient catcher from 0.88 -> 0.98
            l = soft_knee_sample(l, 0.88, 0.10);
            r = soft_knee_sample(r, 0.88, 0.10);
        } else {
            // Even with soft_limit disabled, gently round harsh digital clipping spikes above 0.92 -> 0.985
            l = soft_knee_sample(l, 0.92, 0.065);
            r = soft_knee_sample(r, 0.92, 0.065);
        }

        // Absolute brickwall finite safety guard (peak <= 0.99)
        let out_l = if l.is_finite() {
            flush_denormal(l).clamp(-0.99, 0.99)
        } else {
            0.0
        };
        let out_r = if r.is_finite() {
            flush_denormal(r).clamp(-0.99, 0.99)
        } else {
            0.0
        };
        (out_l, out_r)
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{set_bool_param, set_float_param};

    fn rms(sig: &[(f32, f32)]) -> f32 {
        let sum: f32 = sig.iter().map(|(l, r)| l * l + r * r).sum();
        (sum / (sig.len() as f32 * 2.0)).sqrt()
    }

    fn rms_diff(a: &[(f32, f32)], b: &[(f32, f32)]) -> f32 {
        let sum: f32 = a
            .iter()
            .zip(b.iter())
            .map(|((al, ar), (bl, br))| (al - bl).powi(2) + (ar - br).powi(2))
            .sum();
        (sum / (a.len() as f32 * 2.0)).sqrt()
    }

    fn sine_wave(freq: f32, amp: f32, n: usize, sr: f32) -> Vec<(f32, f32)> {
        (0..n)
            .map(|i| {
                let s = amp * (TAU * freq * (i as f32) / sr).sin();
                (s, s)
            })
            .collect()
    }

    #[test]
    fn master_eq_bands_and_bypass_produce_expected_frequency_changes() {
        let sr = 48000.0;
        let mods = GlobalModOffsets::default();

        // Low shelf +12 dB at 100 Hz boosts 80 Hz sine strongly
        let low_in = sine_wave(80.0, 0.2, 4800, sr);
        let mut p = MasterParams::default();
        set_bool_param(&mut p.eq_on, false);
        set_float_param(&mut p.eq_low_freq, 120.0);
        set_float_param(&mut p.eq_low_gain, 12.0);
        let mut m_off = MasterSection::new(sr);
        let out_off: Vec<_> = low_in
            .iter()
            .map(|&(l, r)| m_off.process_sample(l, r, &p, &mods, 1.0))
            .collect();

        set_bool_param(&mut p.eq_on, true);
        let mut m_low = MasterSection::new(sr);
        let out_low: Vec<_> = low_in
            .iter()
            .map(|&(l, r)| m_low.process_sample(l, r, &p, &mods, 1.0))
            .collect();
        assert!(
            rms(&out_low[1000..]) > rms(&out_off[1000..]) * 2.0,
            "Low shelf +12 dB should strongly boost 80 Hz"
        );

        // Mid peak -12 dB at 1000 Hz cuts 1000 Hz sine strongly
        let mid_in = sine_wave(1000.0, 0.3, 4800, sr);
        let mut p_mid = MasterParams::default();
        set_bool_param(&mut p_mid.eq_on, true);
        set_float_param(&mut p_mid.eq_mid_freq, 1000.0);
        set_float_param(&mut p_mid.eq_mid_q, 1.5);
        set_float_param(&mut p_mid.eq_mid_gain, -12.0);
        let mut m_mid = MasterSection::new(sr);
        let out_mid: Vec<_> = mid_in
            .iter()
            .map(|&(l, r)| m_mid.process_sample(l, r, &p_mid, &mods, 1.0))
            .collect();
        assert!(
            rms(&out_mid[1000..]) < rms(&mid_in[1000..]) * 0.4,
            "Mid peak -12 dB should strongly cut 1 kHz"
        );

        // High shelf +10 dB at 6000 Hz boosts 10 kHz sine strongly
        let hi_in = sine_wave(10000.0, 0.2, 4800, sr);
        let mut p_hi = MasterParams::default();
        set_bool_param(&mut p_hi.eq_on, true);
        set_float_param(&mut p_hi.eq_high_freq, 6000.0);
        set_float_param(&mut p_hi.eq_high_gain, 10.0);
        let mut m_hi = MasterSection::new(sr);
        let out_hi: Vec<_> = hi_in
            .iter()
            .map(|&(l, r)| m_hi.process_sample(l, r, &p_hi, &mods, 1.0))
            .collect();
        assert!(
            rms(&out_hi[1000..]) > rms(&hi_in[1000..]) * 1.8,
            "High shelf +10 dB should boost 10 kHz"
        );

        // Magnitude response helper matches boost sign
        assert!(MasterEq::magnitude_db(10000.0, sr, &p_hi) > 6.0);
    }

    #[test]
    fn x_comp_lifts_quiet_signals_and_stays_silent_on_zero_input() {
        let sr = 48000.0;
        let mut xc = XComp::new(sr);

        // Silence must remain exact 0.0 (no noise-floor pumping)
        for _ in 0..2000 {
            let (l, r) = xc.process_sample(0.0, 0.0, 1.0);
            assert_eq!(l, 0.0);
            assert_eq!(r, 0.0);
        }

        // Quiet signal (-30 dBFS ≈ 0.031) gets upward-lifted by X-Comp
        let quiet = sine_wave(1000.0, 0.03, 9600, sr);
        let mut xc2 = XComp::new(sr);
        let out: Vec<_> = quiet
            .iter()
            .map(|&(l, r)| xc2.process_sample(l, r, 1.0))
            .collect();
        assert!(out.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
        assert!(
            rms(&out[2000..]) > rms(&quiet[2000..]) * 1.35,
            "X-Comp should lift quiet signals (in={}, out={})",
            rms(&quiet[2000..]),
            rms(&out[2000..])
        );
    }

    #[test]
    fn character_modes_dc_blocker_velocity_curve_and_brickwall_limiter() {
        for sr in [44100.0_f32, 48000.0, 96000.0] {
            let mods = GlobalModOffsets::default();
            let sig = sine_wave(330.0, 0.45, 4800, sr);

            let p_clean = MasterParams::default();
            let mut m_clean = MasterSection::new(sr);
            let out_clean: Vec<_> = sig
                .iter()
                .map(|&(l, r)| m_clean.process_sample(l, r, &p_clean, &mods, 1.0))
                .collect();

            // Warm
            let mut p_warm = MasterParams::default();
            set_bool_param(&mut p_warm.warm, true);
            let mut m_warm = MasterSection::new(sr);
            let out_warm: Vec<_> = sig
                .iter()
                .map(|&(l, r)| m_warm.process_sample(l, r, &p_warm, &mods, 1.0))
                .collect();
            assert!(rms_diff(&out_clean, &out_warm) > 0.015, "Warm should alter signal at sr={sr}");

            // Soft
            let mut p_soft = MasterParams::default();
            set_bool_param(&mut p_soft.soft, true);
            let mut m_soft = MasterSection::new(sr);
            let out_soft: Vec<_> = sig
                .iter()
                .map(|&(l, r)| m_soft.process_sample(l, r, &p_soft, &mods, 1.0))
                .collect();
            assert!(rms_diff(&out_clean, &out_soft) > 0.015, "Soft should alter signal at sr={sr}");

            // Boost
            let mut p_boost = MasterParams::default();
            set_bool_param(&mut p_boost.boost, true);
            let mut m_boost = MasterSection::new(sr);
            let out_boost: Vec<_> = sig
                .iter()
                .map(|&(l, r)| m_boost.process_sample(l, r, &p_boost, &mods, 1.0))
                .collect();
            assert!(rms_diff(&out_clean, &out_boost) > 0.05, "Boost should alter signal at sr={sr}");

            // DC Blocker rejects heavy +0.4 DC offset down to |DC| <= 0.005
            let mut m_dc = MasterSection::new(sr);
            let mut dc_sum = 0.0_f32;
            let settle = (0.3 * sr) as usize;
            let measure = (0.2 * sr) as usize;
            for i in 0..(settle + measure) {
                let s = 0.4 + 0.1 * (TAU * 220.0 * i as f32 / sr).sin();
                let (l, _) = m_dc.process_sample(s, s, &p_warm, &mods, 1.0);
                if i >= settle {
                    dc_sum += l;
                }
            }
            let dc_mean = (dc_sum / measure as f32).abs();
            assert!(
                dc_mean <= 0.005,
                "DC blocker failed at sr={sr}: |DC| = {dc_mean}"
            );

            // Brickwall safety limiter keeps peak <= 0.99 under extreme +24 dB hot input
            let mut m_lim = MasterSection::new(sr);
            let hot = sine_wave(440.0, 16.0, 2048, sr);
            for &(l, r) in &hot {
                let (ol, or) = m_lim.process_sample(l, r, &p_boost, &mods, 2.0);
                assert!(ol.is_finite() && or.is_finite());
                assert!(ol.abs() <= 0.99 && or.abs() <= 0.99);
            }
        }

        // Velocity curve sanity
        assert!((apply_velocity_curve(0.2, 0.0) - 1.0).abs() < 1e-6);
        assert!((apply_velocity_curve(0.2, 1.0) - 0.2).abs() < 1e-6);
        assert!((apply_velocity_curve(0.2, 0.5) - 0.6).abs() < 1e-6);
    }
}
