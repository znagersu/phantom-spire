mod audio_eval;
mod param_apply;
mod refs;
mod rewriter;

use audio_eval::{
    autofix_and_level_preset, evaluate_preset, write_wav_pcm16, AudioMetrics, HealthStatus,
    EVAL_SAMPLE_RATE,
};
use phantom_spire::presets::{normalize_category, PresetData, SPIRE_CATEGORIES};
use refs::{scan_reference_wavs, tune_preset_with_refs, RefProfile};
use rewriter::{classify_role, rewrite_preset_from_scratch};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn collect_preset_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|x| x.path())).collect();
    items.sort();
    for p in items {
        if p.is_dir() {
            collect_preset_files(&p, out);
        } else if p
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("json"))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

fn slug_for_temp_wav(rel_path: &str) -> String {
    rel_path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        + ".wav"
}

fn format_freq_balance(m: &AudioMetrics) -> String {
    format!(
        "L{:>2.0}% M{:>2.0}% H{:>2.0}%",
        m.low_pct.round(),
        m.mid_pct.round(),
        m.high_pct.round()
    )
}

#[derive(Clone)]
struct EvaluatedEntry {
    name: String,
    category: String,
    role: String,
    metrics: AudioMetrics,
}

fn print_table(rows: &[EvaluatedEntry]) {
    println!(
        "{:<24} | {:<11} | {:>10} | {:>7} | {:<15} | {:>5} | {:>5} | {:<7} | {}",
        "Preset", "Role", "Peak(dBFS)", "RMS(dB)", "Freq Balance", "Tail", "Score", "Status", "Notes"
    );
    println!("{}", "-".repeat(122));
    for r in rows {
        let short_name: String = r.name.chars().take(24).collect();
        let m = &r.metrics;
        println!(
            "{:<24} | {:<11} | {:>6.2} ({:>2.0})| {:>7.1} | {:<15} | {:>4.2}s | {:>5.1} | {:<7} | {}",
            short_name,
            r.role,
            m.peak_db,
            m.peak * 100.0,
            m.active_rms_db,
            format_freq_balance(m),
            m.decay_tail_s,
            m.total_score,
            m.status.as_str(),
            m.issue_detail
        );
    }
}

fn build_best_of_preview(rows: &[EvaluatedEntry], out_path: &Path) -> Result<(), String> {
    let mut picks: Vec<&EvaluatedEntry> = Vec::new();
    for &cat in SPIRE_CATEGORIES {
        let mut cat_entries: Vec<&EvaluatedEntry> = rows
            .iter()
            .filter(|e| normalize_category(&e.category).eq_ignore_ascii_case(cat))
            .collect();
        cat_entries.sort_by(|a, b| {
            b.metrics
                .total_score
                .partial_cmp(&a.metrics.total_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
        });
        for item in cat_entries.into_iter().take(2) {
            picks.push(item);
        }
    }

    let pause_samples = (0.45 * EVAL_SAMPLE_RATE) as usize;
    let fade_samples = (0.008 * EVAL_SAMPLE_RATE) as usize;
    let mut stitch_l = Vec::new();
    let mut stitch_r = Vec::new();

    println!();
    println!("==================================================================================================");
    println!(
        " BEST-OF PREVIEW SELECTIONS ({} presets -> {})",
        picks.len(),
        out_path.display()
    );
    println!("==================================================================================================");
    println!(
        "{:>3} | {:<6} | {:<10} | {:<26} | {:>5} | {:>15} | {:>7}",
        "#", "Start", "Category", "Preset", "Score", "L / M / H %", "RMS(dB)"
    );
    println!("{}", "-".repeat(94));

    for (idx, p) in picks.iter().enumerate() {
        let start_s = stitch_l.len() as f32 / EVAL_SAMPLE_RATE;
        let len = p.metrics.preview_l.len().min(p.metrics.preview_r.len());
        let seg_start = stitch_l.len();
        stitch_l.extend_from_slice(&p.metrics.preview_l[..len]);
        stitch_r.extend_from_slice(&p.metrics.preview_r[..len]);

        // Short edge micro-fade so concatenated clips never click on segment boundaries
        let fade = fade_samples.min(len / 2);
        for i in 0..fade {
            let g_in = i as f32 / fade as f32;
            stitch_l[seg_start + i] *= g_in;
            stitch_r[seg_start + i] *= g_in;
            let end_i = seg_start + len - 1 - i;
            stitch_l[end_i] *= g_in;
            stitch_r[end_i] *= g_in;
        }

        if idx + 1 < picks.len() {
            stitch_l.resize(stitch_l.len() + pause_samples, 0.0);
            stitch_r.resize(stitch_r.len() + pause_samples, 0.0);
        }

        println!(
            "{:>3} | {:>5.1}s | {:<10} | {:<26} | {:>5.1} | {:<15} | {:>7.1}",
            idx + 1,
            start_s,
            p.category,
            p.name.chars().take(26).collect::<String>(),
            p.metrics.total_score,
            format_freq_balance(&p.metrics),
            p.metrics.active_rms_db
        );
    }

    write_wav_pcm16(out_path, EVAL_SAMPLE_RATE as u32, &stitch_l, &stitch_r)?;
    let total_dur_s = stitch_l.len() as f32 / EVAL_SAMPLE_RATE;
    println!(
        "\n[preview] Wrote {} ({:.1}s total across {} top-2-per-category presets)",
        out_path.display(),
        total_dur_s,
        picks.len()
    );
    Ok(())
}

fn run_batch(
    presets_dir: &Path,
    refs_dir: &Path,
    wav_dir: &Path,
    preview_out: Option<&Path>,
    do_rewrite: bool,
    do_fix: bool,
) -> Result<(), String> {
    let mut files = Vec::new();
    collect_preset_files(presets_dir, &mut files);
    if files.is_empty() {
        return Err(format!("No preset .json files found under {}", presets_dir.display()));
    }

    let ref_profiles: Vec<RefProfile> = scan_reference_wavs(refs_dir);
    if ref_profiles.is_empty() {
        println!(
            "[refs] No .wav files found in {} (reference matching skipped).",
            refs_dir.display()
        );
    } else {
        println!("[refs] Loaded {} reference .wav profile(s):", ref_profiles.len());
        for r in &ref_profiles {
            println!(
                "  - {}: brightness={:.1}% high, low_weight={:.1}% low, decay_tail={:.2}s",
                r.file_name, r.brightness_high_pct, r.low_weight_pct, r.decay_tail_s
            );
        }
    }

    let mut rows = Vec::with_capacity(files.len());
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut rewritten_count = 0usize;
    let mut ref_matched_count = 0usize;
    let mut leveled_or_fixed_count = 0usize;

    for path in &files {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read '{}': {e}", path.display()))?;
        let mut preset: PresetData = serde_json::from_str(&raw)
            .map_err(|e| format!("Failed to parse '{}': {e}", path.display()))?;

        let mut modified = false;
        let role = if do_rewrite {
            let (new_preset, r) = rewrite_preset_from_scratch(&preset);
            preset = new_preset;
            modified = true;
            rewritten_count += 1;
            r
        } else {
            classify_role(&preset)
        };

        if (do_rewrite || do_fix) && !ref_profiles.is_empty() {
            if let Some(_ref_name) = tune_preset_with_refs(&mut preset, &ref_profiles) {
                modified = true;
                ref_matched_count += 1;
            }
        }

        let rel = path
            .strip_prefix(presets_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let wav_path = wav_dir.join(slug_for_temp_wav(&rel));
        let mut metrics = evaluate_preset(&preset, Some(&wav_path));

        if do_fix && metrics.status != HealthStatus::Error {
            let was_bad = metrics.status != HealthStatus::Ok;
            for _attempt in 0..3 {
                if !autofix_and_level_preset(&mut preset, &metrics) {
                    break;
                }
                modified = true;
                leveled_or_fixed_count += 1;
                metrics = evaluate_preset(&preset, Some(&wav_path));
                if metrics.status == HealthStatus::Ok
                    && (audio_eval::target_rms_db_for_role(role) - metrics.active_rms_db).abs() <= 1.6
                    && metrics.peak <= 0.78
                {
                    if was_bad {
                        metrics.issue_detail = String::from("auto-fixed");
                    }
                    break;
                }
            }
        }

        if modified {
            let json_out = serde_json::to_string_pretty(&preset)
                .map_err(|e| format!("Serialize error for '{}': {e}", path.display()))?;
            std::fs::write(path, format!("{json_out}\n"))
                .map_err(|e| format!("Failed writing '{}': {e}", path.display()))?;
        }

        *counts.entry(metrics.status.as_str()).or_insert(0) += 1;
        rows.push(EvaluatedEntry {
            name: preset.name.clone(),
            category: preset.category.clone(),
            role: role.label().to_string(),
            metrics,
        });
    }

    println!();
    print_table(&rows);
    println!("{}", "=".repeat(122));
    println!(
        "Total: {} | OK: {} | CLIP: {} | QUIET: {} | MUFFLED: {} | PIERCE: {} | ERROR: {} | Rewritten: {} | Ref-tuned: {} | Adjusted passes: {}",
        rows.len(),
        counts.get("OK").copied().unwrap_or(0),
        counts.get("CLIP").copied().unwrap_or(0),
        counts.get("QUIET").copied().unwrap_or(0),
        counts.get("MUFFLED").copied().unwrap_or(0),
        counts.get("PIERCE").copied().unwrap_or(0),
        counts.get("ERROR").copied().unwrap_or(0),
        rewritten_count,
        ref_matched_count,
        leveled_or_fixed_count
    );
    println!("Temp WAV folder: {}", wav_dir.display());

    if let Some(out_wav) = preview_out {
        build_best_of_preview(&rows, out_wav)?;
    }

    let bad = counts.get("CLIP").copied().unwrap_or(0)
        + counts.get("QUIET").copied().unwrap_or(0)
        + counts.get("MUFFLED").copied().unwrap_or(0)
        + counts.get("PIERCE").copied().unwrap_or(0)
        + counts.get("ERROR").copied().unwrap_or(0);
    if bad > 0 {
        Err(format!("{bad} preset(s) flagged with issues"))
    } else {
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut presets_dir = PathBuf::from("presets/factory");
    let mut refs_dir = PathBuf::from("refs");
    let mut wav_dir = std::env::temp_dir().join("phantom_spire_preset_batch");
    let mut preview_out: Option<PathBuf> = None;
    let mut do_rewrite = false;
    let mut do_fix = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--presets" if i + 1 < args.len() => {
                presets_dir = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--refs" if i + 1 < args.len() => {
                refs_dir = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--wav-dir" if i + 1 < args.len() => {
                wav_dir = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--preview-out" if i + 1 < args.len() => {
                preview_out = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--rewrite" => do_rewrite = true,
            "--fix" => do_fix = true,
            _ => {}
        }
        i += 1;
    }

    if let Err(e) = run_batch(
        &presets_dir,
        &refs_dir,
        &wav_dir,
        preview_out.as_deref(),
        do_rewrite,
        do_fix,
    ) {
        eprintln!("[preset_batch_check] FAIL: {e}");
        std::process::exit(1);
    }
}
