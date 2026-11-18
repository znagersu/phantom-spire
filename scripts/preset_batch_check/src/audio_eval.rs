use crate::param_apply::apply_preset_to_engine_state;
use crate::rewriter::{classify_role, InstrumentRole};
use nih_plug::prelude::Buffer;
use phantom_spire::dsp::{SynthEngine, TransportInfo};
use phantom_spire::presets::PresetData;
use phantom_spire::{SharedSynthState, SharedUiState};
use serde_json::json;
use std::f32::consts::{SQRT_2, TAU};
use std::fs::File;
use std::io::Write;
use std::path::Path;

pub const EVAL_SAMPLE_RATE: f32 = 44100.0;
const BLOCK_SIZE: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Ok,
    Clip,
    Quiet,
    Muffled,
    Piercing,
    Error,
}

impl HealthStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            HealthStatus::Ok => "OK",
            HealthStatus::Clip => "CLIP",
            HealthStatus::Quiet => "QUIET",
            HealthStatus::Muffled => "MUFFLED",
            HealthStatus::Piercing => "PIERCE",
            HealthStatus::Error => "ERROR",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AudioMetrics {
    pub peak: f32,
    pub peak_db: f32,
    pub active_rms_db: f32,
    pub low_pct: f32,
    pub mid_pct: f32,
    pub high_pct: f32,
    pub decay_tail_s: f32,
    pub crest_db: f32,
    pub clarity_score: f32,
    pub punch_score: f32,
    pub balance_score: f32,
    pub total_score: f32,
    pub status: HealthStatus,
    pub issue_detail: String,
    pub preview_l: Vec<f32>,
    pub preview_r: Vec<f32>,
}

#[derive(Debug, Clone, Copy)]
struct BiquadLp {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl BiquadLp {
    fn new(cutoff_hz: f32, sr: f32) -> Self {
        let fc = cutoff_hz.clamp(20.0, sr * 0.45);
        let w0 = TAU * fc / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * SQRT_2 * 0.5);
        let b0 = (1.0 - cos_w0) * 0.5;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        let inv = 1.0 / a0;
        Self {
            b0: b0 * inv,
            b1: b1 * inv,
            b2: b2 * inv,
            a1: a1 * inv,
            a2: a2 * inv,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Measure 3-band spectral energy percentage: Low (<250 Hz), Mid (250..2500 Hz), High (>2500 Hz).
pub fn measure_3band_balance(mono: &[f32], sr: f32) -> (f32, f32, f32) {
    let mut lp_low = BiquadLp::new(250.0, sr);
    let mut lp_mid = BiquadLp::new(2500.0, sr);
    let mut e_low = 0.0_f64;
    let mut e_mid = 0.0_f64;
    let mut e_high = 0.0_f64;

    for &x in mono {
        let low = lp_low.tick(x);
        let low_mid = lp_mid.tick(x);
        let mid = low_mid - low;
        let high = x - low_mid;
        e_low += (low as f64) * (low as f64);
        e_mid += (mid as f64) * (mid as f64);
        e_high += (high as f64) * (high as f64);
    }
    let total = (e_low + e_mid + e_high).max(1e-18);
    (
        (100.0 * e_low / total) as f32,
        (100.0 * e_mid / total) as f32,
        (100.0 * e_high / total) as f32,
    )
}

/// Estimate decay tail duration (seconds from release start down to -30 dB).
pub fn measure_decay_tail_seconds(mono: &[f32], release_start_sample: usize, sr: f32) -> f32 {
    if release_start_sample >= mono.len() {
        return 0.05;
    }
    let win = (0.015 * sr) as usize;
    let mut ref_rms = 0.0_f32;
    let ref_start = release_start_sample.saturating_sub(win * 4);
    if release_start_sample > ref_start {
        let slice = &mono[ref_start..release_start_sample];
        ref_rms = (slice.iter().map(|s| s * s).sum::<f32>() / slice.len() as f32).sqrt();
    }
    if ref_rms < 1e-5 {
        return 0.05;
    }
    let target = ref_rms * 0.0316;
    let mut pos = release_start_sample;
    while pos + win <= mono.len() {
        let slice = &mono[pos..pos + win];
        let r = (slice.iter().map(|s| s * s).sum::<f32>() / slice.len() as f32).sqrt();
        if r <= target {
            return ((pos - release_start_sample) as f32 / sr).max(0.02);
        }
        pos += (win / 2).max(1);
    }
    ((mono.len() - release_start_sample) as f32 / sr).max(0.05)
}

#[inline]
pub fn lin_to_db(x: f32) -> f32 {
    if x > 1e-6 {
        20.0 * x.log10()
    } else {
        -120.0
    }
}

pub fn write_wav_pcm16(path: &Path, sr: u32, left: &[f32], right: &[f32]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create temp dir failed: {e}"))?;
    }
    let frames = left.len().min(right.len());
    let data_bytes = (frames * 4) as u32;
    let mut f = File::create(path).map_err(|e| format!("create wav failed: {e}"))?;
    f.write_all(b"RIFF").map_err(|e| e.to_string())?;
    f.write_all(&(36u32 + data_bytes).to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(b"WAVE").map_err(|e| e.to_string())?;
    f.write_all(b"fmt ").map_err(|e| e.to_string())?;
    f.write_all(&16u32.to_le_bytes()).map_err(|e| e.to_string())?;
    f.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?;
    f.write_all(&2u16.to_le_bytes()).map_err(|e| e.to_string())?;
    f.write_all(&sr.to_le_bytes()).map_err(|e| e.to_string())?;
    f.write_all(&(sr * 4).to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&4u16.to_le_bytes()).map_err(|e| e.to_string())?;
    f.write_all(&16u16.to_le_bytes()).map_err(|e| e.to_string())?;
    f.write_all(b"data").map_err(|e| e.to_string())?;
    f.write_all(&data_bytes.to_le_bytes())
        .map_err(|e| e.to_string())?;

    let mut raw = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        let l = (left[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
        let r = (right[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
        raw.extend_from_slice(&l.to_le_bytes());
        raw.extend_from_slice(&r.to_le_bytes());
    }
    f.write_all(&raw).map_err(|e| e.to_string())?;
    Ok(())
}

fn render_blocks(
    engine: &mut SynthEngine,
    params: &phantom_spire::params::PhantomSpireParams,
    shared: &SharedSynthState,
    ui: &SharedUiState,
    transport: &TransportInfo,
    blocks: usize,
    out_l: &mut Vec<f32>,
    out_r: &mut Vec<f32>,
) {
    let mut blk_l = vec![0.0_f32; BLOCK_SIZE];
    let mut blk_r = vec![0.0_f32; BLOCK_SIZE];
    for _ in 0..blocks {
        let mut buf = Buffer::default();
        unsafe {
            buf.set_slices(BLOCK_SIZE, |s| {
                s.clear();
                s.push(&mut blk_l);
                s.push(&mut blk_r);
            });
        }
        engine.process(&mut buf, params, shared, ui, transport);
        out_l.extend_from_slice(&blk_l);
        out_r.extend_from_slice(&blk_r);
    }
}

fn render_step_phrase(
    engine: &mut SynthEngine,
    params: &phantom_spire::params::PhantomSpireParams,
    shared: &SharedSynthState,
    ui: &SharedUiState,
    transport: &TransportInfo,
    steps: &[(&[u8], f32, usize, usize)],
    out_l: &mut Vec<f32>,
    out_r: &mut Vec<f32>,
) -> (usize, usize) {
    let mut last_rel = 0usize;
    let mut attack_windows = Vec::new();
    for &(notes, vel, hold_blocks, gap_blocks) in steps {
        let start = out_l.len();
        for &n in notes {
            engine.note_on(n, vel, 0);
        }
        attack_windows.push(start);
        render_blocks(engine, params, shared, ui, transport, hold_blocks, out_l, out_r);
        last_rel = out_l.len();
        for &n in notes {
            engine.note_off(n, 0);
        }
        if gap_blocks > 0 {
            render_blocks(engine, params, shared, ui, transport, gap_blocks, out_l, out_r);
        }
    }
    let _ = attack_windows;
    (last_rel, out_l.len())
}

fn compute_role_score(
    role: InstrumentRole,
    name: &str,
    peak: f32,
    active_rms_db: f32,
    low_pct: f32,
    mid_pct: f32,
    high_pct: f32,
    diff_ratio: f32,
    crest_db: f32,
    status: HealthStatus,
) -> (f32, f32, f32, f32) {
    // 1. Clarity score (0..35): clean headroom, healthy stereo dynamics, zero harsh static/aliasing
    let headroom_pts = if peak <= 0.80 && peak >= 0.16 {
        15.0
    } else if peak < 0.96 {
        11.0
    } else {
        2.0
    };
    let smooth_pts = if diff_ratio <= 0.45 {
        12.0
    } else if diff_ratio <= 0.68 {
        8.5
    } else {
        3.0
    };
    let level_pts = (8.0 - (active_rms_db - (-18.5)).abs() * 0.65).clamp(0.0, 8.0);
    let clarity = (headroom_pts + smooth_pts + level_pts).clamp(0.0, 35.0);

    // 2. Punch & envelope dynamics (0..30): crest factor matched to instrument role
    let ideal_crest = match role {
        InstrumentRole::DrumKick | InstrumentRole::DrumPerc | InstrumentRole::PluckAcoustic | InstrumentRole::PluckSynth => 11.5,
        InstrumentRole::PunchBass | InstrumentRole::AcidBass | InstrumentRole::SeqArp => 9.5,
        InstrumentRole::KeysEp | InstrumentRole::KeysClavPluck | InstrumentRole::SoloLead | InstrumentRole::SupersawLead => 8.5,
        _ => 7.5,
    };
    let crest_err = (crest_db - ideal_crest).abs();
    let punch = (30.0 - crest_err * 2.4).clamp(5.0, 30.0);

    // 3. Frequency balance (0..35): role-calibrated Low / Mid / High spectral targets
    let (target_l, target_m, target_h) = match role {
        InstrumentRole::SubBass => (52.0, 44.0, 4.0),
        InstrumentRole::AcidBass | InstrumentRole::ReeseBass | InstrumentRole::PunchBass => (44.0, 46.0, 10.0),
        InstrumentRole::KeysEp | InstrumentRole::KeysOrgan | InstrumentRole::KeysClavPluck => (20.0, 60.0, 20.0),
        InstrumentRole::StringsPad | InstrumentRole::ChoirPad | InstrumentRole::AmbientPad => (22.0, 60.0, 18.0),
        InstrumentRole::SupersawLead | InstrumentRole::SoloLead => (16.0, 58.0, 26.0),
        InstrumentRole::PluckAcoustic | InstrumentRole::PluckSynth => (18.0, 58.0, 24.0),
        InstrumentRole::SynthBrass | InstrumentRole::SynthPoly => (20.0, 62.0, 18.0),
        InstrumentRole::SeqArp => (22.0, 56.0, 22.0),
        InstrumentRole::DrumKick => (62.0, 32.0, 6.0),
        InstrumentRole::DrumPerc => (20.0, 45.0, 35.0),
        InstrumentRole::SoundFx | InstrumentRole::Init => (22.0, 56.0, 22.0),
    };
    let spec_err = 0.55 * (low_pct - target_l).abs()
        + 0.35 * (mid_pct - target_m).abs()
        + 0.65 * (high_pct - target_h).abs();
    let balance = (35.0 - spec_err * 0.55).clamp(0.0, 35.0);

    let penalty = if status == HealthStatus::Ok { 0.0 } else { 40.0 };
    let _ = name;
    let total = (clarity + punch + balance - penalty).clamp(0.0, 100.0);
    (clarity, punch, balance, total)
}

pub fn evaluate_preset(data: &PresetData, wav_out: Option<&Path>) -> AudioMetrics {
    let shared = SharedSynthState::default();
    let params = match apply_preset_to_engine_state(data, &shared) {
        Ok(p) => p,
        Err(e) => {
            return AudioMetrics {
                peak: 0.0,
                peak_db: -120.0,
                active_rms_db: -120.0,
                low_pct: 0.0,
                mid_pct: 0.0,
                high_pct: 0.0,
                decay_tail_s: 0.0,
                crest_db: 0.0,
                clarity_score: 0.0,
                punch_score: 0.0,
                balance_score: 0.0,
                total_score: 0.0,
                status: HealthStatus::Error,
                issue_detail: e,
                preview_l: Vec::new(),
                preview_r: Vec::new(),
            };
        }
    };

    let ui = SharedUiState::default();
    let transport = TransportInfo {
        tempo_bpm: 120.0,
        playing: true,
        sample_rate: EVAL_SAMPLE_RATE,
        ppq_pos: Some(0.0),
        pos_samples: Some(0),
    };
    let mut engine = SynthEngine::new(EVAL_SAMPLE_RATE);
    let role = classify_role(data);

    let mut out_l = Vec::with_capacity(96000);
    let mut out_r = Vec::with_capacity(96000);

    // Role-specific short musical bass line or chord progression (~1.8s + tail)
    let (last_rel, _) = match role {
        InstrumentRole::SubBass
        | InstrumentRole::AcidBass
        | InstrumentRole::ReeseBass
        | InstrumentRole::PunchBass
        | InstrumentRole::DrumKick => {
            // Driving 4-note bass line (C2 -> C2 -> G1 -> A#1 -> C2)
            render_step_phrase(
                &mut engine,
                &params,
                &shared,
                &ui,
                &transport,
                &[
                    (&[36], 0.88, 28, 3),
                    (&[36], 0.84, 16, 2),
                    (&[31], 0.86, 26, 3),
                    (&[34], 0.85, 22, 2),
                    (&[36], 0.88, 34, 24),
                ],
                &mut out_l,
                &mut out_r,
            )
        }
        InstrumentRole::SoloLead | InstrumentRole::SupersawLead => {
            // Expressive synth lead motif ending in octave harmonic sustain
            if matches!(role, InstrumentRole::SoloLead) {
                render_step_phrase(
                    &mut engine,
                    &params,
                    &shared,
                    &ui,
                    &transport,
                    &[
                        (&[60], 0.84, 22, 1),
                        (&[63], 0.82, 18, 1),
                        (&[67], 0.86, 22, 1),
                        (&[65], 0.82, 18, 1),
                        (&[72], 0.88, 36, 24),
                    ],
                    &mut out_l,
                    &mut out_r,
                )
            } else {
                // Supersaw stack chord progression: Am -> F -> C -> G
                render_step_phrase(
                    &mut engine,
                    &params,
                    &shared,
                    &ui,
                    &transport,
                    &[
                        (&[57, 60, 64], 0.82, 34, 2),
                        (&[53, 57, 60], 0.82, 34, 2),
                        (&[60, 64, 67], 0.84, 34, 2),
                        (&[55, 59, 62], 0.82, 36, 26),
                    ],
                    &mut out_l,
                    &mut out_r,
                )
            }
        }
        _ => {
            // Musical 4-bar chord progression (Am -> F -> C -> G)
            render_step_phrase(
                &mut engine,
                &params,
                &shared,
                &ui,
                &transport,
                &[
                    (&[57, 60, 64], 0.82, 36, 2),
                    (&[53, 57, 60], 0.82, 36, 2),
                    (&[48, 55, 60, 64], 0.82, 36, 2),
                    (&[55, 59, 62], 0.82, 38, 28),
                ],
                &mut out_l,
                &mut out_r,
            )
        }
    };

    if let Some(wav_path) = wav_out {
        let _ = write_wav_pcm16(wav_path, EVAL_SAMPLE_RATE as u32, &out_l, &out_r);
    }

    let mut peak = 0.0_f32;
    let mut non_finite = false;
    for &s in out_l.iter().chain(out_r.iter()) {
        if !s.is_finite() {
            non_finite = true;
        } else {
            peak = peak.max(s.abs());
        }
    }

    let mono: Vec<f32> = out_l
        .iter()
        .zip(out_r.iter())
        .map(|(&l, &r)| 0.5 * (l + r))
        .collect();

    let active_len = last_rel.min(mono.len()).max(1);
    let active_slice = &mono[..active_len];
    let active_rms =
        (active_slice.iter().map(|x| x * x).sum::<f32>() / active_slice.len() as f32).sqrt();
    let (low_pct, mid_pct, high_pct) = measure_3band_balance(active_slice, EVAL_SAMPLE_RATE);
    let decay_tail_s = measure_decay_tail_seconds(&mono, last_rel, EVAL_SAMPLE_RATE);

    let mut diff_sq = 0.0_f64;
    if active_slice.len() > 1 {
        for i in 1..active_slice.len() {
            let d = (active_slice[i] - active_slice[i - 1]) as f64;
            diff_sq += d * d;
        }
    }
    let diff_rms = (diff_sq / (active_slice.len().saturating_sub(1).max(1) as f64)).sqrt() as f32;
    let diff_ratio = diff_rms / active_rms.max(1e-6);

    let peak_db = lin_to_db(peak);
    let active_rms_db = lin_to_db(active_rms);
    let crest_db = (peak_db - active_rms_db).max(0.0);

    let is_explicit_sub_or_kick = matches!(role, InstrumentRole::SubBass | InstrumentRole::DrumKick);
    let is_hat_or_fx = matches!(role, InstrumentRole::DrumPerc | InstrumentRole::SoundFx)
        && data.name.to_ascii_lowercase().contains("hat");

    let (status, issue_detail) = if non_finite {
        (HealthStatus::Error, String::from("non-finite samples"))
    } else if peak >= 0.96 {
        (HealthStatus::Clip, format!("peak={peak:.3} >= 0.96"))
    } else if active_rms_db < -30.0 || peak_db < -15.5 {
        (
            HealthStatus::Quiet,
            format!("peak={peak_db:.1}dB rms={active_rms_db:.1}dB"),
        )
    } else if !is_explicit_sub_or_kick && ((high_pct < 1.8 && mid_pct < 20.0) || high_pct < 0.7) {
        (
            HealthStatus::Muffled,
            format!("L/M/H={low_pct:.0}/{mid_pct:.0}/{high_pct:.0}%"),
        )
    } else if !is_hat_or_fx
        && !matches!(role, InstrumentRole::DrumPerc | InstrumentRole::SoundFx)
        && (high_pct > 44.0 || (high_pct > 36.0 && low_pct < 10.0) || diff_ratio > 0.75)
    {
        (
            HealthStatus::Piercing,
            format!("high={high_pct:.0}% diff={diff_ratio:.2}"),
        )
    } else {
        (HealthStatus::Ok, String::new())
    };

    let (clarity_score, punch_score, balance_score, total_score) = compute_role_score(
        role,
        &data.name,
        peak,
        active_rms_db,
        low_pct,
        mid_pct,
        high_pct,
        diff_ratio,
        crest_db,
        status,
    );

    AudioMetrics {
        peak,
        peak_db,
        active_rms_db,
        low_pct,
        mid_pct,
        high_pct,
        decay_tail_s,
        crest_db,
        clarity_score,
        punch_score,
        balance_score,
        total_score,
        status,
        issue_detail,
        preview_l: out_l,
        preview_r: out_r,
    }
}

/// Target RMS loudness by role so sustained patches, basses, and percussive plucks all feel balanced.
pub fn target_rms_db_for_role(role: InstrumentRole) -> f32 {
    match role {
        InstrumentRole::SubBass | InstrumentRole::AcidBass | InstrumentRole::ReeseBass | InstrumentRole::PunchBass => -18.5,
        InstrumentRole::PluckAcoustic | InstrumentRole::PluckSynth | InstrumentRole::SeqArp | InstrumentRole::DrumPerc => -21.0,
        InstrumentRole::StringsPad | InstrumentRole::ChoirPad | InstrumentRole::AmbientPad => -19.5,
        _ => -19.0,
    }
}

/// Automatically repair clipping, muffled, piercing, or quiet presets AND level-match
/// output volume (`k_vol`) so all patches sit at a consistent, non-clipping loudness.
pub fn autofix_and_level_preset(data: &mut PresetData, metrics: &AudioMetrics) -> bool {
    let mut changed = false;
    let role = classify_role(data);

    let get_f = |d: &PresetData, k: &str, default: f32| -> f32 {
        d.params
            .get(k)
            .and_then(|v| v.as_f64())
            .map(|x| x as f32)
            .unwrap_or(default)
    };

    match metrics.status {
        HealthStatus::Clip => {
            if data.params.get("k_boos").and_then(|v| v.as_bool()) == Some(true) {
                data.params.insert("k_boos".into(), json!(false));
            }
            data.params.insert("k_lim".into(), json!(true));
            let vol = get_f(data, "k_vol", 0.708);
            data.params.insert("k_vol".into(), json!((vol * 0.78).clamp(0.25, 1.50)));
            changed = true;
        }
        HealthStatus::Quiet => {
            let l1 = get_f(data, "o_lvl_1", 0.8).max(0.84);
            data.params.insert("o_lvl_1".into(), json!(l1));
            let cut1 = get_f(data, "f_cut_1", 18000.0);
            if cut1 < 2200.0 {
                data.params.insert("f_cut_1".into(), json!(3400.0));
            }
            let dec1 = get_f(data, "e_dec_1", 0.4).max(0.45);
            data.params.insert("e_dec_1".into(), json!(dec1));
            let sus1 = get_f(data, "e_sus_1", 0.8).max(0.32);
            data.params.insert("e_sus_1".into(), json!(sus1));
            changed = true;
        }
        HealthStatus::Muffled => {
            let cut1 = get_f(data, "f_cut_1", 1800.0).max(3400.0);
            data.params.insert("f_cut_1".into(), json!(cut1));
            data.params.insert("f_typ_1".into(), json!("LP2"));
            let res1 = get_f(data, "f_res_1", 0.0).min(0.40);
            data.params.insert("f_res_1".into(), json!(res1));
            let env_amt = get_f(data, "es_amt_1_2", 0.0).max(0.32);
            data.params.insert("es_amt_1_2".into(), json!(env_amt));
            let oct2 = data.params.get("o_oct_2").and_then(|v| v.as_i64()).unwrap_or(0);
            if oct2 < 0 {
                data.params.insert("o_oct_2".into(), json!(0));
            }
            let lvl2 = get_f(data, "o_lvl_2", 0.3).max(0.42);
            data.params.insert("o_lvl_2".into(), json!(lvl2));
            changed = true;
        }
        HealthStatus::Piercing => {
            let cut1 = get_f(data, "f_cut_1", 8000.0);
            data.params.insert("f_typ_1".into(), json!("LP4"));
            data.params.insert("f_cut_1".into(), json!((cut1 * 0.62).clamp(1800.0, 4000.0)));
            let ktr1 = get_f(data, "f_ktr_1", 0.0).min(0.15);
            data.params.insert("f_ktr_1".into(), json!(ktr1));
            let res1 = get_f(data, "f_res_1", 0.0).min(0.20);
            data.params.insert("f_res_1".into(), json!(res1));
            let drv1 = get_f(data, "f_drv_1", 0.0).min(0.10);
            data.params.insert("f_drv_1".into(), json!(drv1));
            let sdw = get_f(data, "x_sdw", 0.0).min(0.10);
            data.params.insert("x_sdw".into(), json!(sdw));
            let env_amt = get_f(data, "es_amt_1_2", 0.0).min(0.18);
            data.params.insert("es_amt_1_2".into(), json!(env_amt));
            let mod1 = data.params.get("o_mod_1").and_then(|v| v.as_str()).unwrap_or("");
            if mod1 == "AMSync" || mod1 == "FM" {
                let ca1 = get_f(data, "o_ca_1", 0.5).min(0.24);
                let cb1 = get_f(data, "o_cb_1", 0.3).min(0.14);
                data.params.insert("o_ca_1".into(), json!(ca1));
                data.params.insert("o_cb_1".into(), json!(cb1));
            }
            changed = true;
        }
        _ => {}
    }

    // Level-match loudness across all presets within non-clipping bounds (peak <= -2.5 dBFS)
    let target_rms = target_rms_db_for_role(role);
    let rms_err = target_rms - metrics.active_rms_db;
    let cur_vol = get_f(data, "k_vol", 0.708);
    if rms_err.abs() > 1.2 || metrics.peak > 0.78 {
        let mut desired_gain = 10.0_f32.powf(rms_err / 20.0);
        // Never allow normalized peak to exceed 0.76 (-2.4 dBFS)
        if metrics.peak > 1e-4 {
            desired_gain = desired_gain.min(0.76 / metrics.peak);
        }
        let next_vol = (cur_vol * desired_gain).clamp(0.28, 1.55);
        if (next_vol - cur_vol).abs() > 0.015 {
            data.params.insert("k_vol".into(), json!((next_vol * 1000.0).round() / 1000.0));
            changed = true;
        }
    }

    let att1 = get_f(data, "e_att_1", 0.003).max(0.0025);
    let rel1 = get_f(data, "e_rel_1", 0.15).max(0.004);
    data.params.insert("e_att_1".into(), json!(att1));
    data.params.insert("e_rel_1".into(), json!(rel1));

    changed
}
