use crate::audio_eval::{measure_3band_balance, measure_decay_tail_seconds};
use phantom_spire::presets::PresetData;
use serde_json::json;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RefProfile {
    pub file_name: String,
    pub keywords: Vec<String>,
    pub brightness_high_pct: f32,
    pub low_weight_pct: f32,
    pub decay_tail_s: f32,
}

fn collect_wavs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|x| x.path())).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_wavs(&p, out);
        } else if p
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("wav"))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

/// Minimal RIFF WAV reader supporting 16/24/32-bit PCM and 32-bit float (mono or stereo).
fn load_wav_mono(path: &Path) -> Result<(u32, Vec<f32>), String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| format!("open '{}': {e}", path.display()))?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;

    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("'{}' is not a valid RIFF/WAVE file", path.display()));
    }

    let mut pos = 12usize;
    let mut format_tag = 1u16;
    let mut channels = 1u16;
    let mut sample_rate = 44100u32;
    let mut bits_per_sample = 16u16;
    let mut data_slice: Option<&[u8]> = None;

    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]])
            as usize;
        let payload_start = pos + 8;
        let payload_end = (payload_start + size).min(bytes.len());
        if id == b"fmt " && size >= 16 && payload_end >= payload_start + 16 {
            let p = &bytes[payload_start..payload_end];
            format_tag = u16::from_le_bytes([p[0], p[1]]);
            channels = u16::from_le_bytes([p[2], p[3]]).max(1);
            sample_rate = u32::from_le_bytes([p[4], p[5], p[6], p[7]]).max(8000);
            bits_per_sample = u16::from_le_bytes([p[14], p[15]]);
        } else if id == b"data" {
            data_slice = Some(&bytes[payload_start..payload_end]);
            break;
        }
        pos = payload_start + size + (size & 1);
    }

    let data = data_slice.ok_or_else(|| format!("'{}' missing data chunk", path.display()))?;
    let bytes_per_sample = (bits_per_sample / 8) as usize;
    if bytes_per_sample == 0 {
        return Err(String::from("Invalid bits_per_sample"));
    }
    let frame_stride = bytes_per_sample * channels as usize;
    let frames = data.len() / frame_stride;
    let mut mono = Vec::with_capacity(frames);

    for f in 0..frames {
        let mut ch_sum = 0.0_f32;
        for c in 0..channels as usize {
            let off = f * frame_stride + c * bytes_per_sample;
            let s = match (format_tag, bits_per_sample) {
                (3, 32) => f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]),
                (1, 16) => i16::from_le_bytes([data[off], data[off + 1]]) as f32 / 32768.0,
                (1, 24) => {
                    let raw = ((data[off + 2] as i8 as i32) << 16)
                        | ((data[off + 1] as i32) << 8)
                        | (data[off] as i32);
                    raw as f32 / 8_388_608.0
                }
                (1, 32) => {
                    i32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as f32
                        / 2_147_483_648.0
                }
                _ => return Err(format!("Unsupported WAV fmt tag={format_tag} bits={bits_per_sample}")),
            };
            ch_sum += if s.is_finite() { s } else { 0.0 };
        }
        mono.push(ch_sum / channels as f32);
    }

    Ok((sample_rate, mono))
}

fn find_peak_index(mono: &[f32], sr: f32) -> usize {
    let win = (0.02 * sr) as usize;
    if mono.len() <= win {
        return 0;
    }
    let mut best_idx = 0usize;
    let mut best_rms = 0.0_f32;
    let mut idx = 0usize;
    while idx + win <= mono.len() {
        let r = (mono[idx..idx + win].iter().map(|s| s * s).sum::<f32>() / win as f32).sqrt();
        if r > best_rms {
            best_rms = r;
            best_idx = idx + win / 2;
        }
        idx += (win / 2).max(1);
    }
    best_idx
}

pub fn scan_reference_wavs(refs_dir: &Path) -> Vec<RefProfile> {
    if !refs_dir.is_dir() {
        return Vec::new();
    }
    let mut files = Vec::new();
    collect_wavs(refs_dir, &mut files);
    let mut profiles = Vec::new();

    for path in files {
        match load_wav_mono(&path) {
            Ok((sr, mono)) if !mono.is_empty() => {
                let (low_pct, _mid_pct, high_pct) = measure_3band_balance(&mono, sr as f32);
                let peak_pos = find_peak_index(&mono, sr as f32);
                let decay_tail_s = measure_decay_tail_seconds(&mono, peak_pos, sr as f32);
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("ref")
                    .to_string();
                let keywords: Vec<String> = stem
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .filter(|w| w.len() >= 2)
                    .map(|w| w.to_ascii_lowercase())
                    .collect();
                profiles.push(RefProfile {
                    file_name: path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("ref.wav")
                        .to_string(),
                    keywords,
                    brightness_high_pct: high_pct,
                    low_weight_pct: low_pct,
                    decay_tail_s,
                });
            }
            Ok(_) => {}
            Err(e) => eprintln!("[refs] Warning skipping {}: {e}", path.display()),
        }
    }
    profiles
}

/// Match a preset against reference `.wav` keywords/category and gently tune its filter cutoff,
/// sub/low-end EQ weight, and envelope release/decay tail to mirror the matched reference.
pub fn tune_preset_with_refs(data: &mut PresetData, refs: &[RefProfile]) -> Option<String> {
    if refs.is_empty() {
        return None;
    }
    let hay = format!(
        "{} {} {}",
        data.category.to_ascii_lowercase(),
        data.name.to_ascii_lowercase(),
        data.tags.join(" ").to_ascii_lowercase()
    );

    let mut best: Option<(usize, &RefProfile)> = None;
    for r in refs {
        let mut score = 0usize;
        for kw in &r.keywords {
            if hay.contains(kw) {
                score += 1;
            }
        }
        if score > 0 && best.map_or(true, |(bs, _)| score > bs) {
            best = Some((score, r));
        }
    }
    let (_, matched) = best?;

    // Map reference brightness (high_pct: 2..40%) -> filter cutoff target (~1600..11500 Hz)
    let bright_norm = (matched.brightness_high_pct / 35.0).clamp(0.0, 1.0);
    let target_cut = (1500.0 + bright_norm * 9500.0).round();
    data.params.insert("f_cut_1".into(), json!(target_cut));

    // Map reference low-end weight (low_weight_pct: 10..70%) -> warm low shelf / sub balance
    if matched.low_weight_pct > 38.0 {
        let boost_db = ((matched.low_weight_pct - 35.0) * 0.14).clamp(1.0, 4.5);
        data.params.insert("k_eqon".into(), json!(true));
        data.params.insert("k_elf".into(), json!(110.0));
        data.params.insert("k_elg".into(), json!(boost_db));
    }

    // Mirror measured decay tail onto release and decay envelopes (within musical 2..5ms floors)
    let tail = matched.decay_tail_s.clamp(0.08, 3.0);
    data.params.insert("e_rel_1".into(), json!((tail * 0.55).clamp(0.05, 1.2)));
    let cur_dec = data
        .params
        .get("e_dec_1")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.45) as f32;
    data.params.insert(
        "e_dec_1".into(),
        json!((0.5 * cur_dec + 0.5 * tail).clamp(0.12, 2.2)),
    );

    Some(matched.file_name.clone())
}
