//! Dual stereo multimode filters supporting Spire's 5 filter characters
//! (`Perfecto`, `Infecto`, `Acido`, `Scorpio`, `Combo`), 11 filter types/slopes,
//! keytracking, `PAR` (parallel vs serial), `LINK`, and `filter_balance`.

use crate::dsp::osc::{fast_tanh, flush_denormal};
use crate::params::{FilterModel, FilterParams, FilterRoutingParams, FilterType};
use std::f32::consts::{PI, SQRT_2};

const COMB_BUF_LEN: usize = 4096;

/// Snapshot of one filter's parameters.
#[derive(Debug, Clone, Copy)]
pub struct FilterSettings {
    pub model: FilterModel,
    pub filter_type: FilterType,
    pub cutoff: f32,
    pub resonance: f32,
    pub keytrack: f32,
    pub drive: f32,
}

impl Default for FilterSettings {
    fn default() -> Self {
        Self {
            model: FilterModel::Perfecto,
            filter_type: FilterType::LP4,
            cutoff: 18000.0,
            resonance: 0.0,
            keytrack: 0.0,
            drive: 0.0,
        }
    }
}

impl FilterSettings {
    #[inline]
    pub fn from_params(p: &FilterParams) -> Self {
        Self {
            model: p.model.value(),
            filter_type: p.filter_type.value(),
            cutoff: p.cutoff.value(),
            resonance: p.resonance.value(),
            keytrack: p.keytrack.value(),
            drive: p.drive.value(),
        }
    }
}

/// Snapshot of dual-filter routing parameters (`PAR`, `LINK`, `filter_balance`).
#[derive(Debug, Clone, Copy, Default)]
pub struct FilterRoutingSettings {
    pub filter_balance: f32,
    pub parallel: bool,
    pub link: bool,
}

impl FilterRoutingSettings {
    #[inline]
    pub fn from_params(p: &FilterRoutingParams) -> Self {
        Self {
            filter_balance: p.filter_balance.value(),
            parallel: p.parallel.value(),
            link: p.link.value(),
        }
    }
}

/// Per-sample modulation offsets for the dual filter bank.
#[derive(Debug, Clone, Copy, Default)]
pub struct DualFilterModInput {
    /// Cutoff modulation in octaves for Filter 1 and Filter 2.
    pub cutoff_oct: [f32; 2],
    /// Resonance modulation (-1.0..=1.0) for Filter 1 and Filter 2.
    pub res: [f32; 2],
    /// Filter balance modulation (-1.0..=1.0).
    pub balance: f32,
}

/// 2-pole Cytomic TPT (Zero-Delay Feedback) State Variable Filter state.
#[derive(Debug, Clone, Copy, Default)]
struct Svf2Pole {
    ic1eq: f32,
    ic2eq: f32,
}

impl Svf2Pole {
    #[inline]
    fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    /// Process one sample returning `(low, band, high)`.
    #[inline]
    fn tick(&mut self, input: f32, g: f32, k: f32) -> (f32, f32, f32) {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;

        self.ic1eq = flush_denormal(2.0 * v1 - self.ic1eq);
        self.ic2eq = flush_denormal(2.0 * v2 - self.ic2eq);

        let low = v2;
        let band = v1;
        let high = input - k * v1 - v2;
        (low, band, high)
    }

    /// Nonlinear Sallen-Key / Scorpio-style tick with asymmetric saturation in state.
    #[inline]
    fn tick_scorpio(&mut self, input: f32, g: f32, k: f32, drive_gain: f32) -> (f32, f32, f32) {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        // Asymmetric gritty diode feedback inside the resonant state
        let fb_sat = fast_tanh(self.ic1eq * drive_gain + 0.12) - 0.1194;
        let ic1 = 0.55 * self.ic1eq + 0.45 * (fb_sat / drive_gain.max(1.0));

        let v3 = input - self.ic2eq;
        let v1 = a1 * ic1 + a2 * v3;
        let v2 = self.ic2eq + a2 * ic1 + a3 * v3;

        self.ic1eq = flush_denormal(fast_tanh((2.0 * v1 - ic1) * 0.85) / 0.85);
        self.ic2eq = flush_denormal(fast_tanh((2.0 * v2 - self.ic2eq) * 0.85) / 0.85);

        let low = v2;
        let band = v1;
        let high = input - k * v1 - v2;
        (low, band, high)
    }
}

/// 4-pole ZDF Ladder filter state (used by `Infecto` and `Acido`).
#[derive(Debug, Clone, Copy, Default)]
struct Ladder4Pole {
    s: [f32; 4],
    fb_hp: f32,
}

impl Ladder4Pole {
    #[inline]
    fn reset(&mut self) {
        self.s = [0.0; 4];
        self.fb_hp = 0.0;
    }

    /// Virus-style `Infecto` warm transistor ladder with bass-preserving resonance.
    #[inline]
    fn tick_infecto(
        &mut self,
        input: f32,
        g: f32,
        res: f32,
        drive: f32,
        filter_type: FilterType,
    ) -> f32 {
        let big_g = g / (1.0 + g);
        let g2 = big_g * big_g;
        let g3 = g2 * big_g;
        let g4 = g3 * big_g;

        // Warm asymmetric input saturation
        let drv = 1.15 + drive * 3.0;
        let x = fast_tanh(input * drv + 0.06 * input * input) / (1.0 + drive * 0.45);

        let k = res * 3.82;
        let s_sum = g3 * self.s[0] + g2 * self.s[1] + big_g * self.s[2] + self.s[3];
        // Bass-preserving feedback compensation
        let comp = 0.42 * k * x;
        let u = fast_tanh((x + comp - k * s_sum) / (1.0 + k * g4));

        let v0 = (u - self.s[0]) * big_g;
        let y0 = fast_tanh(v0 + self.s[0]);
        self.s[0] = flush_denormal(y0 + v0);

        let v1 = (y0 - self.s[1]) * big_g;
        let y1 = fast_tanh(v1 + self.s[1]);
        self.s[1] = flush_denormal(y1 + v1);

        let v2 = (y1 - self.s[2]) * big_g;
        let y2 = fast_tanh(v2 + self.s[2]);
        self.s[2] = flush_denormal(y2 + v2);

        let v3 = (y2 - self.s[3]) * big_g;
        let y3 = fast_tanh(v3 + self.s[3]);
        self.s[3] = flush_denormal(y3 + v3);

        Self::pole_mix(u, y0, y1, y2, y3, res, filter_type)
    }

    /// TB-303 diode-ladder inspired `Acido` filter: 18 dB/oct asymmetric diode cascade
    /// with high-pass squelch feedback and biting resonant peak.
    #[inline]
    fn tick_acido(
        &mut self,
        input: f32,
        g_main: f32,
        g_first: f32,
        res: f32,
        drive: f32,
        filter_type: FilterType,
    ) -> f32 {
        // Real 303 diode ladder has a 1-octave higher first pole (creating 18 dB/oct slope)
        // and resonant squelch boosted by a high-passed nonlinear feedback loop.
        let g0 = g_first / (1.0 + g_first);
        let g = g_main / (1.0 + g_main);
        let g2 = g * g;
        let g3 = g2 * g;

        let drv = 1.6 + drive * 4.5;
        let x = fast_tanh(input * drv * 1.15);

        // High-pass the resonant feedback so sub-bass stays tight while mid-cutoff squelches
        let fb_raw = g3 * self.s[0] + g2 * self.s[1] + g * self.s[2] + self.s[3];
        self.fb_hp = flush_denormal(self.fb_hp + 0.14 * (fb_raw - self.fb_hp));
        let fb_sq = fb_raw - 0.65 * self.fb_hp;

        // Higher diode feedback gain with saturating limiter keeps self-oscillation musical & stable
        let k = res * 5.2;
        let fb_sat = fast_tanh(fb_sq * (1.4 + res * 1.2) + 0.08) - 0.079;
        let u = fast_tanh((x - k * fb_sat) / (1.0 + k * 0.65 * g0 * g3));

        // 4 cascaded TPT poles with nonlinear diode shaping (stage 1 at ~2.4x cutoff)
        let v0 = (u - self.s[0]) * g0;
        let y0 = fast_tanh((v0 + self.s[0]) * 1.25);
        self.s[0] = flush_denormal(y0 + v0);

        let v1 = (y0 - self.s[1]) * g;
        let y1 = fast_tanh((v1 + self.s[1]) * 1.15);
        self.s[1] = flush_denormal(y1 + v1);

        let v2 = (y1 - self.s[2]) * g;
        let y2 = fast_tanh((v2 + self.s[2]) * 1.12);
        self.s[2] = flush_denormal(y2 + v2);

        let v3 = (y2 - self.s[3]) * g;
        let y3 = fast_tanh((v3 + self.s[3]) * 1.10);
        self.s[3] = flush_denormal(y3 + v3);

        // Emphasize 3-pole/4-pole 18 dB/oct acid bite on LP4
        let out = match filter_type {
            FilterType::LP4 => 0.45 * y2 + 0.65 * y3 + res * 0.45 * (y1 - y2),
            _ => Self::pole_mix(u, y0, y1, y2, y3, res, filter_type),
        };
        fast_tanh(out * (1.1 + res * 0.45))
    }

    #[inline]
    fn pole_mix(
        u: f32,
        y0: f32,
        y1: f32,
        y2: f32,
        y3: f32,
        res: f32,
        filter_type: FilterType,
    ) -> f32 {
        match filter_type {
            FilterType::LP2 => y1,
            FilterType::LP4 => y3,
            FilterType::LR4 => 0.7 * y3 + 0.3 * y2,
            FilterType::BP2 => 2.0 * (y0 - y1),
            FilterType::BP4 => 4.0 * (y1 - 2.0 * y2 + y3),
            FilterType::HP2 => u - 2.0 * y0 + y1,
            FilterType::HP4 => u - 4.0 * y0 + 6.0 * y1 - 4.0 * y2 + y3,
            FilterType::Peak => u + (1.4 + res * 2.4) * (y0 - y1),
            FilterType::Notch => u - 2.0 * (y0 - y1),
            FilterType::LpBp => 0.65 * y1 + 1.35 * (y0 - y1),
            FilterType::HpBp => 0.65 * (u - 2.0 * y0 + y1) + 1.25 * (y0 - y1),
        }
    }
}

/// Interpolated feedback comb resonator used by `FilterModel::Combo`.
struct CombChannel {
    buf: Box<[f32; COMB_BUF_LEN]>,
    write_pos: usize,
    damp_state: f32,
    delay_smooth: f32,
}

impl CombChannel {
    fn new() -> Self {
        Self {
            buf: Box::new([0.0; COMB_BUF_LEN]),
            write_pos: 0,
            damp_state: 0.0,
            delay_smooth: 64.0,
        }
    }

    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write_pos = 0;
        self.damp_state = 0.0;
        self.delay_smooth = 64.0;
    }

    #[inline]
    fn tick(&mut self, input: f32, target_delay: f32, feedback: f32) -> f32 {
        let clamped_target = target_delay.clamp(2.0, (COMB_BUF_LEN - 3) as f32);
        self.delay_smooth += 0.05 * (clamped_target - self.delay_smooth);

        let read_pos = (self.write_pos as f32) - self.delay_smooth;
        let wrapped = if read_pos < 0.0 {
            read_pos + COMB_BUF_LEN as f32
        } else {
            read_pos
        };
        let idx0 = (wrapped as usize) & (COMB_BUF_LEN - 1);
        let idx1 = (idx0 + 1) & (COMB_BUF_LEN - 1);
        let frac = wrapped - wrapped.floor();

        let delayed = self.buf[idx0] + frac * (self.buf[idx1] - self.buf[idx0]);
        self.damp_state = flush_denormal(0.72 * delayed + 0.28 * self.damp_state);

        let fb = feedback.clamp(-0.91, 0.91);
        let to_write = flush_denormal(fast_tanh(input + fb * self.damp_state));
        self.buf[self.write_pos] = to_write;
        self.write_pos = (self.write_pos + 1) & (COMB_BUF_LEN - 1);

        // Return resonant comb output
        0.55 * input + 0.65 * self.damp_state
    }
}

/// One stereo channel of a multimode filter.
struct FilterChannel {
    svf1: Svf2Pole,
    svf2: Svf2Pole,
    ladder: Ladder4Pole,
    comb: CombChannel,
    dc_x1: f32,
    dc_y1: f32,
}

impl FilterChannel {
    fn new() -> Self {
        Self {
            svf1: Svf2Pole::default(),
            svf2: Svf2Pole::default(),
            ladder: Ladder4Pole::default(),
            comb: CombChannel::new(),
            dc_x1: 0.0,
            dc_y1: 0.0,
        }
    }

    fn reset(&mut self) {
        self.svf1.reset();
        self.svf2.reset();
        self.ladder.reset();
        self.comb.reset();
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
    }

    #[inline]
    fn block_dc(&mut self, x: f32) -> f32 {
        // Gentle ~7 Hz high-pass DC blocker to remove any DC from asymmetric saturation
        let y = x - self.dc_x1 + 0.999 * self.dc_y1;
        self.dc_x1 = flush_denormal(x);
        self.dc_y1 = flush_denormal(y);
        y
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn tick(
        &mut self,
        input: f32,
        sample_rate: f32,
        fc: f32,
        res: f32,
        drive: f32,
        model: FilterModel,
        filter_type: FilterType,
    ) -> f32 {
        let max_fc = (sample_rate * 0.44).min(20000.0);
        let safe_fc = fc.clamp(20.0, max_fc);
        let safe_res = res.clamp(0.0, 0.985);
        let safe_drv = drive.clamp(0.0, 1.0);
        let g = (PI * safe_fc / sample_rate).tan();

        let out = match model {
            FilterModel::Perfecto => {
                let x = if safe_drv > 1e-4 {
                    let drv_g = 1.0 + safe_drv * 2.5;
                    fast_tanh(input * drv_g) / (1.0 + safe_drv * 0.5)
                } else {
                    input
                };
                self.tick_svf_pair(x, g, safe_res, filter_type, false, 1.0)
            }
            FilterModel::Infecto => {
                let y = self
                    .ladder
                    .tick_infecto(input, g, safe_res, safe_drv, filter_type);
                self.block_dc(y)
            }
            FilterModel::Acido => {
                let fc_first = (safe_fc * 2.4).min(max_fc);
                let g_first = (PI * fc_first / sample_rate).tan();
                let y = self
                    .ladder
                    .tick_acido(input, g, g_first, safe_res, safe_drv, filter_type);
                self.block_dc(y)
            }
            FilterModel::Scorpio => {
                let drv_g = 1.45 + safe_drv * 3.4;
                let x = fast_tanh(input * (1.1 + safe_drv * 2.0));
                let y = self.tick_svf_pair(x, g, safe_res, filter_type, true, drv_g);
                self.block_dc(y)
            }
            FilterModel::Combo => {
                let x = if safe_drv > 1e-4 {
                    fast_tanh(input * (1.0 + safe_drv * 2.8))
                } else {
                    input
                };
                let delay_samples = sample_rate / safe_fc;
                let comb_fb = 0.35 + safe_res * 0.55;
                let combed = self.comb.tick(x, delay_samples, comb_fb);
                // Shape the comb output through the SVF multimode stage so LP/BP/HP/Notch all act musically
                let shaped = self.tick_svf_pair(combed, g, safe_res * 0.65, filter_type, false, 1.0);
                0.7 * shaped + 0.3 * combed
            }
        };

        flush_denormal(out.clamp(-8.0, 8.0))
    }

    #[inline]
    fn tick_svf_pair(
        &mut self,
        x: f32,
        g: f32,
        res: f32,
        filter_type: FilterType,
        scorpio: bool,
        scorpio_drv: f32,
    ) -> f32 {
        // Damping k: 2.0 at res=0 (critically/Butterworth-ish), down to 0.03 at max res
        let k_2p = (2.0 - 1.97 * res).clamp(0.03, 2.0);
        // For 4-pole cascade, distribute damping slightly more gently per stage so total Q matches musical expectation
        let k_4p = (2.0 - 1.85 * res).clamp(0.08, 2.0);
        let k_lr4 = (SQRT_2 * (1.0 - 0.88 * res)).clamp(0.12, SQRT_2);

        let g2 = if scorpio { (g * 1.07).min(5.5) } else { g };

        macro_rules! stage1 {
            ($inp:expr, $k:expr) => {
                if scorpio {
                    self.svf1.tick_scorpio($inp, g, $k, scorpio_drv)
                } else {
                    self.svf1.tick($inp, g, $k)
                }
            };
        }
        macro_rules! stage2 {
            ($inp:expr, $k:expr) => {
                if scorpio {
                    self.svf2.tick_scorpio($inp, g2, $k, scorpio_drv)
                } else {
                    self.svf2.tick($inp, g2, $k)
                }
            };
        }

        match filter_type {
            FilterType::LP2 => stage1!(x, k_2p).0,
            FilterType::LP4 => {
                let (lp1, _, _) = stage1!(x, k_4p);
                let mid = fast_tanh(lp1 * 0.9) / 0.9;
                stage2!(mid, k_4p).0
            }
            FilterType::LR4 => {
                // Linkwitz-Riley 4th-order: two cascaded Butterworth-damped LP2 stages (-6 dB at cutoff)
                let (lp1, _, _) = stage1!(x, k_lr4);
                stage2!(lp1, k_lr4).0
            }
            FilterType::BP2 => {
                let (_, bp, _) = stage1!(x, k_2p);
                bp * (1.0 + (1.0 - res) * 0.35)
            }
            FilterType::BP4 => {
                let (_, bp1, _) = stage1!(x, k_4p);
                let mid = fast_tanh(bp1 * 0.95) / 0.95;
                let (_, bp2, _) = stage2!(mid, k_4p);
                bp2 * 1.35
            }
            FilterType::HP2 => stage1!(x, k_2p).2,
            FilterType::HP4 => {
                let (_, _, hp1) = stage1!(x, k_4p);
                let mid = fast_tanh(hp1 * 0.9) / 0.9;
                stage2!(mid, k_4p).2
            }
            FilterType::Peak => {
                let (_, bp, _) = stage1!(x, k_2p);
                x + bp * (0.9 + res * 2.2)
            }
            FilterType::Notch => {
                let (lp, _, hp) = stage1!(x, k_2p);
                lp + hp
            }
            FilterType::LpBp => {
                let (lp, bp, _) = stage1!(x, k_2p);
                0.65 * lp + 0.75 * bp
            }
            FilterType::HpBp => {
                let (_, bp, hp) = stage1!(x, k_2p);
                0.65 * hp + 0.75 * bp
            }
        }
    }
}

/// Stereo single multimode filter (Filter 1 or Filter 2).
pub struct MultimodeFilter {
    sample_rate: f32,
    left: FilterChannel,
    right: FilterChannel,
}

impl MultimodeFilter {
    pub fn new(sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() && sample_rate >= 8000.0 {
            sample_rate
        } else {
            44100.0
        };
        Self {
            sample_rate: sr,
            left: FilterChannel::new(),
            right: FilterChannel::new(),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate >= 8000.0 {
            self.sample_rate = sample_rate;
        }
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    #[inline]
    pub fn process_stereo(
        &mut self,
        in_l: f32,
        in_r: f32,
        fc: f32,
        res: f32,
        drive: f32,
        model: FilterModel,
        filter_type: FilterType,
    ) -> (f32, f32) {
        let out_l = self
            .left
            .tick(in_l, self.sample_rate, fc, res, drive, model, filter_type);
        let out_r = self
            .right
            .tick(in_r, self.sample_rate, fc, res, drive, model, filter_type);
        (out_l, out_r)
    }
}

/// Dual stereo filter bank (`Filter 1 & 2`) with keytracking, `PAR`/Serial routing,
/// `LINK`, and `filter_balance`.
pub struct DualFilter {
    sample_rate: f32,
    filter1: MultimodeFilter,
    filter2: MultimodeFilter,
}

impl DualFilter {
    pub fn new(sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() && sample_rate >= 8000.0 {
            sample_rate
        } else {
            44100.0
        };
        Self {
            sample_rate: sr,
            filter1: MultimodeFilter::new(sr),
            filter2: MultimodeFilter::new(sr),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate.is_finite() && sample_rate >= 8000.0 {
            self.sample_rate = sample_rate;
            self.filter1.set_sample_rate(sample_rate);
            self.filter2.set_sample_rate(sample_rate);
        }
    }

    pub fn reset(&mut self) {
        self.filter1.reset();
        self.filter2.reset();
    }

    /// Compute effective cutoff frequency in Hz including keytracking and octave modulation.
    #[inline]
    fn effective_cutoff(
        base_hz: f32,
        keytrack: f32,
        midi_note: f32,
        mod_octaves: f32,
        sample_rate: f32,
    ) -> f32 {
        let ktr_oct = keytrack.clamp(-1.0, 1.0) * (midi_note - 60.0) / 12.0;
        let total_oct = (ktr_oct + mod_octaves).clamp(-10.0, 10.0);
        let max_fc = (sample_rate * 0.44).min(20000.0);
        (base_hz.clamp(20.0, 20000.0) * 2.0_f32.powf(total_oct)).clamp(20.0, max_fc)
    }

    /// Process stereo Filter 1 and Filter 2 input busses through the dual filter bank.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn process_stereo(
        &mut self,
        f1_in_l: f32,
        f1_in_r: f32,
        f2_in_l: f32,
        f2_in_r: f32,
        midi_note: f32,
        f1_cfg: &FilterSettings,
        f2_cfg: &FilterSettings,
        routing: &FilterRoutingSettings,
        mod_in: &DualFilterModInput,
    ) -> (f32, f32) {
        let fc1 = Self::effective_cutoff(
            f1_cfg.cutoff,
            f1_cfg.keytrack,
            midi_note,
            mod_in.cutoff_oct[0],
            self.sample_rate,
        );

        let fc2 = if routing.link {
            // LINK: Filter 2 tracks Filter 1's effective cutoff (+ any Filter 2 own cutoff mod/keytrack)
            let f2_extra_oct =
                (f2_cfg.keytrack.clamp(-1.0, 1.0) - f1_cfg.keytrack.clamp(-1.0, 1.0))
                    * (midi_note - 60.0)
                    / 12.0
                    + mod_in.cutoff_oct[1];
            let max_fc = (self.sample_rate * 0.44).min(20000.0);
            (fc1 * 2.0_f32.powf(f2_extra_oct.clamp(-10.0, 10.0))).clamp(20.0, max_fc)
        } else {
            Self::effective_cutoff(
                f2_cfg.cutoff,
                f2_cfg.keytrack,
                midi_note,
                mod_in.cutoff_oct[1],
                self.sample_rate,
            )
        };

        let res1 = (f1_cfg.resonance + mod_in.res[0]).clamp(0.0, 0.985);
        let res2 = (f2_cfg.resonance + mod_in.res[1]).clamp(0.0, 0.985);
        let bal = (routing.filter_balance + mod_in.balance).clamp(-1.0, 1.0);

        if routing.parallel {
            let (y1_l, y1_r) = self.filter1.process_stereo(
                f1_in_l,
                f1_in_r,
                fc1,
                res1,
                f1_cfg.drive,
                f1_cfg.model,
                f1_cfg.filter_type,
            );
            let (y2_l, y2_r) = self.filter2.process_stereo(
                f2_in_l,
                f2_in_r,
                fc2,
                res2,
                f2_cfg.drive,
                f2_cfg.model,
                f2_cfg.filter_type,
            );
            // At bal = 0.0 both outputs sum at 1.0; -1.0 = F1 only; +1.0 = F2 only.
            let g1 = (1.0 - bal).clamp(0.0, 1.0);
            let g2 = (1.0 + bal).clamp(0.0, 1.0);
            (y1_l * g1 + y2_l * g2, y1_r * g1 + y2_r * g2)
        } else {
            // Serial routing: Filter 1 -> Filter 2, with filter_balance smoothly morphing
            // -1.0 (Filter 1 only) <-> 0.0 (100% Serial F1->F2) <-> +1.0 (Filter 2 only).
            let (y1_l, y1_r) = self.filter1.process_stereo(
                f1_in_l,
                f1_in_r,
                fc1,
                res1,
                f1_cfg.drive,
                f1_cfg.model,
                f1_cfg.filter_type,
            );
            let bypass_f1 = bal.max(0.0);
            let f2_src_l = y1_l * (1.0 - bypass_f1) + f1_in_l * bypass_f1 + f2_in_l;
            let f2_src_r = y1_r * (1.0 - bypass_f1) + f1_in_r * bypass_f1 + f2_in_r;

            let (y2_l, y2_r) = self.filter2.process_stereo(
                f2_src_l,
                f2_src_r,
                fc2,
                res2,
                f2_cfg.drive,
                f2_cfg.model,
                f2_cfg.filter_type,
            );

            let tap_f1 = (-bal).max(0.0);
            let out_l = y2_l * (1.0 - tap_f1) + y1_l * tap_f1;
            let out_r = y2_r * (1.0 - tap_f1) + y1_r * tap_f1;
            (out_l, out_r)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saw_stimulus(n: usize, freq: f32, sr: f32) -> Vec<f32> {
        let mut ph = 0.0_f32;
        let dt = freq / sr;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(2.0 * ph - 1.0);
            ph = (ph + dt).fract();
        }
        v
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt()
    }

    fn rms_diff(a: &[f32], b: &[f32]) -> f32 {
        (a.iter()
            .zip(b.iter())
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f32>()
            / a.len() as f32)
            .sqrt()
    }

    #[test]
    fn all_five_filter_models_produce_distinct_stable_audio() {
        let models = [
            FilterModel::Perfecto,
            FilterModel::Infecto,
            FilterModel::Acido,
            FilterModel::Scorpio,
            FilterModel::Combo,
        ];
        let sr = 48000.0;
        let input = saw_stimulus(4096, 130.81, sr);
        let mut outputs = Vec::new();

        for &model in &models {
            let mut filt = MultimodeFilter::new(sr);
            let mut out = Vec::with_capacity(input.len());
            for &x in &input {
                let (l, r) = filt.process_stereo(x, x, 850.0, 0.72, 0.35, model, FilterType::LP4);
                assert!(l.is_finite() && r.is_finite(), "Non-finite in model {model:?}");
                assert!(l.abs() <= 8.0 && r.abs() <= 8.0);
                out.push(l);
            }
            assert!(rms(&out) > 0.05, "Model {model:?} too quiet");
            outputs.push((model, out));
        }

        for i in 0..outputs.len() {
            for j in (i + 1)..outputs.len() {
                let d = rms_diff(&outputs[i].1, &outputs[j].1);
                assert!(
                    d > 0.04,
                    "Filter models {:?} and {:?} too similar: rms_diff={d}",
                    outputs[i].0,
                    outputs[j].0
                );
            }
        }
    }

    #[test]
    fn all_filter_types_produce_expected_frequency_shaping() {
        let sr = 48000.0;
        // Mix low (150 Hz) + mid (1200 Hz) + high (6000 Hz)
        let n = 4096;
        let mut low_only = Vec::with_capacity(n);
        let mut high_only = Vec::with_capacity(n);
        let mut mid_only = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / sr;
            low_only.push((std::f32::consts::TAU * 150.0 * t).sin());
            mid_only.push((std::f32::consts::TAU * 1200.0 * t).sin());
            high_only.push((std::f32::consts::TAU * 6000.0 * t).sin());
        }

        let eval = |ft: FilterType, sig: &[f32]| -> f32 {
            let mut f = MultimodeFilter::new(sr);
            let mut out = Vec::with_capacity(sig.len());
            for &x in sig {
                let (l, _) = f.process_stereo(x, x, 1200.0, 0.3, 0.0, FilterModel::Perfecto, ft);
                out.push(l);
            }
            rms(&out[512..])
        };

        // LP4 passes low, attenuates high strongly
        assert!(eval(FilterType::LP4, &low_only) > 4.0 * eval(FilterType::LP4, &high_only));
        // HP4 passes high, attenuates low strongly
        assert!(eval(FilterType::HP4, &high_only) > 4.0 * eval(FilterType::HP4, &low_only));
        // BP4 favors mid over low and high
        assert!(eval(FilterType::BP4, &mid_only) > 2.5 * eval(FilterType::BP4, &low_only));
        assert!(eval(FilterType::BP4, &mid_only) > 2.5 * eval(FilterType::BP4, &high_only));
        // Notch attenuates mid relative to low & high
        assert!(eval(FilterType::Notch, &mid_only) < 0.35 * eval(FilterType::Notch, &low_only));
        // Peak boosts mid relative to low
        assert!(eval(FilterType::Peak, &mid_only) > 1.3 * eval(FilterType::Peak, &low_only));
        // LR4 passes low cleanly and attenuates high steeply
        assert!(eval(FilterType::LR4, &low_only) > 4.0 * eval(FilterType::LR4, &high_only));
    }

    #[test]
    fn parallel_vs_serial_and_link_and_keytrack_produce_measurable_differences() {
        let sr = 48000.0;
        let stim = saw_stimulus(2048, 220.0, sr);

        let f1 = FilterSettings {
            model: FilterModel::Perfecto,
            filter_type: FilterType::LP4,
            cutoff: 600.0,
            resonance: 0.2,
            keytrack: 0.0,
            drive: 0.0,
        };
        let f2 = FilterSettings {
            model: FilterModel::Perfecto,
            filter_type: FilterType::HP4,
            cutoff: 2000.0,
            resonance: 0.2,
            keytrack: 0.0,
            drive: 0.0,
        };

        // Serial (LP 600 -> HP 2000 cancels almost everything) vs Parallel (LP 600 + HP 2000 passes lows & highs!)
        let mut df_serial = DualFilter::new(sr);
        let mut df_par = DualFilter::new(sr);
        let r_ser = FilterRoutingSettings {
            filter_balance: 0.0,
            parallel: false,
            link: false,
        };
        let r_par = FilterRoutingSettings {
            filter_balance: 0.0,
            parallel: true,
            link: false,
        };
        let m = DualFilterModInput::default();

        let mut ser_out = Vec::new();
        let mut par_out = Vec::new();
        for &x in &stim {
            let (sl, _) = df_serial.process_stereo(0.5 * x, 0.5 * x, 0.5 * x, 0.5 * x, 60.0, &f1, &f2, &r_ser, &m);
            let (pl, _) = df_par.process_stereo(0.5 * x, 0.5 * x, 0.5 * x, 0.5 * x, 60.0, &f1, &f2, &r_par, &m);
            ser_out.push(sl);
            par_out.push(pl);
        }
        assert!(rms_diff(&ser_out, &par_out) > 0.1, "Parallel vs Serial must differ strongly");

        // LINK test: F2 tracks F1 cutoff (600 Hz) instead of 2000 Hz
        let mut df_nolink = DualFilter::new(sr);
        let mut df_link = DualFilter::new(sr);
        let r_link = FilterRoutingSettings {
            filter_balance: 0.0,
            parallel: true,
            link: true,
        };
        let mut nl_out = Vec::new();
        let mut lk_out = Vec::new();
        for &x in &stim {
            let (a, _) = df_nolink.process_stereo(0.0, 0.0, x, x, 60.0, &f1, &f2, &r_par, &m);
            let (b, _) = df_link.process_stereo(0.0, 0.0, x, x, 60.0, &f1, &f2, &r_link, &m);
            nl_out.push(a);
            lk_out.push(b);
        }
        assert!(rms_diff(&nl_out, &lk_out) > 0.08, "LINK must shift Filter 2 cutoff to track Filter 1");

        // Keytrack test: high note (MIDI 84) opens filter when keytrack=1.0 vs keytrack=0.0
        let mut f1_kt = f1;
        f1_kt.keytrack = 1.0;
        let mut df_k0 = DualFilter::new(sr);
        let mut df_k1 = DualFilter::new(sr);
        let mut k0 = Vec::new();
        let mut k1 = Vec::new();
        let high_stim = saw_stimulus(2048, 1046.5, sr);
        for &x in &high_stim {
            let (a, _) = df_k0.process_stereo(x, x, 0.0, 0.0, 84.0, &f1, &f2, &r_par, &m);
            let (b, _) = df_k1.process_stereo(x, x, 0.0, 0.0, 84.0, &f1_kt, &f2, &r_par, &m);
            k0.push(a);
            k1.push(b);
        }
        assert!(rms(&k1[256..]) > 2.0 * rms(&k0[256..]), "Keytrack must open filter on high notes");
    }

    #[test]
    fn extreme_modulation_is_stable_across_sample_rates() {
        let models = [
            FilterModel::Perfecto,
            FilterModel::Infecto,
            FilterModel::Acido,
            FilterModel::Scorpio,
            FilterModel::Combo,
        ];
        let types = [
            FilterType::LP2,
            FilterType::LP4,
            FilterType::LR4,
            FilterType::BP2,
            FilterType::BP4,
            FilterType::HP2,
            FilterType::HP4,
            FilterType::Peak,
            FilterType::Notch,
            FilterType::LpBp,
            FilterType::HpBp,
        ];
        for &sr in &[44100.0_f32, 48000.0, 96000.0] {
            for &model in &models {
                for &ft in &types {
                    let mut filt = MultimodeFilter::new(sr);
                    for i in 0..256 {
                        // Extreme audio-rate cutoff and resonance modulation
                        let x = if i % 2 == 0 { 1.0 } else { -1.0 };
                        let fc = if i % 4 < 2 { 20.0 } else { 20000.0 };
                        let (l, r) = filt.process_stereo(x, -x, fc, 1.0, 1.0, model, ft);
                        assert!(
                            l.is_finite() && r.is_finite() && l.abs() <= 8.0 && r.abs() <= 8.0,
                            "Unstable at sr={sr} model={model:?} type={ft:?} i={i}: ({l}, {r})"
                        );
                    }
                }
            }
        }
    }
}
