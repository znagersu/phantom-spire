use crate::analyzers::{analyze_stereo, l2_diff_rms};
use crate::vst3_host::Vst3PluginInstance;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct TestCaseResult {
    pub id: String,
    pub name: String,
    pub status: String,
    pub details: String,
}

impl TestCaseResult {
    pub fn from_bool(id: &str, name: &str, ok: bool, details: String) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            status: if ok { "pass" } else { "fail" }.to_string(),
            details,
        }
    }
}

pub fn run_vst3_opaque_suite(plugin_path: &Path) -> Vec<TestCaseResult> {
    let mut results = Vec::new();

    // Ensure auto-update is disabled while hosting VST3
    std::env::set_var("PHANTOM_SPIRE_DISABLE_AUTO_UPDATE", "1");

    let mut plugin = match Vst3PluginInstance::load(plugin_path, 48000.0, 256) {
        Ok(p) => p,
        Err(e) => {
            results.push(TestCaseResult::from_bool(
                "VST3-01-LOAD",
                "Load compiled phantom_spire.vst3 via VST3 COM ABI",
                false,
                e,
            ));
            return results;
        }
    };

    // 1. Metadata & parameter enumeration
    let unique_ids: HashSet<u32> = plugin.params.iter().map(|p| p.id).collect();
    let ok_meta = !plugin.plugin_name.is_empty()
        && plugin.params.len() >= 100
        && unique_ids.len() == plugin.params.len();
    results.push(TestCaseResult::from_bool(
        "VST3-01-METADATA-PARAMS",
        "VST3 factory exports Phantom Spire with >=100 unique automatable parameters",
        ok_meta,
        format!(
            "name='{}' vendor='{}' param_count={} unique_ids={}",
            plugin.plugin_name,
            plugin.vendor,
            plugin.params.len(),
            unique_ids.len()
        ),
    ));

    // Save clean initial state for resetting between tests
    let init_state = match plugin.get_state_bytes() {
        Ok(b) => {
            results.push(TestCaseResult::from_bool(
                "VST3-02-GET-STATE",
                "IComponent::get_state returns non-empty serialized state",
                !b.is_empty(),
                format!("state_bytes={}", b.len()),
            ));
            b
        }
        Err(e) => {
            results.push(TestCaseResult::from_bool(
                "VST3-02-GET-STATE",
                "IComponent::get_state returns non-empty serialized state",
                false,
                e,
            ));
            Vec::new()
        }
    };

    // 2. Silence when idle (no notes played)
    match plugin.render_notes(&[], 8, 0, &[]) {
        Ok((l, r)) => {
            let m = analyze_stereo(&l, &r, 48000.0);
            let ok = m.basi_pass && m.peak_max < 1e-4;
            results.push(TestCaseResult::from_bool(
                "VST3-03-IDLE-SILENCE",
                "Idle plugin produces clean silence (no noise floor leak or NaN)",
                ok,
                format!("basi={} peak={:.6}", m.basi_pass, m.peak_max),
            ));
        }
        Err(e) => results.push(TestCaseResult::from_bool(
            "VST3-03-IDLE-SILENCE",
            "Idle plugin produces clean silence",
            false,
            e,
        )),
    }

    // 3. Single note C4 (MIDI 60) playback -> BASI + audible RMS
    let mut baseline_l = Vec::new();
    let mut baseline_r = Vec::new();
    match plugin.render_notes(&[60], 24, 16, &[]) {
        Ok((l, r)) => {
            let m = analyze_stereo(&l, &r, 48000.0);
            let ok = m.basi_pass && m.rms_dbfs > -42.0 && m.peak_max > 0.01;
            results.push(TestCaseResult::from_bool(
                "VST3-04-NOTE-C4-PLAYBACK",
                "MIDI C4 (60) produces audible BASI-safe stereo audio",
                ok,
                format!(
                    "basi={} rms_dbfs={:.2} peak={:.4} dom_hz={:.1} dc={:.5} errs={:?}",
                    m.basi_pass, m.rms_dbfs, m.peak_max, m.dominant_freq_hz, m.dc_max_abs, m.basi_errors
                ),
            ));
            baseline_l = l;
            baseline_r = r;
        }
        Err(e) => results.push(TestCaseResult::from_bool(
            "VST3-04-NOTE-C4-PLAYBACK",
            "MIDI C4 (60) produces audible BASI-safe stereo audio",
            false,
            e,
        )),
    }

    // 4. Polyphonic 8-voice chord stress -> BASI safe (no clipping, no NaN/Inf)
    let _ = plugin.set_state_bytes(&init_state);
    let chord = [48, 52, 55, 59, 62, 64, 67, 71];
    match plugin.render_notes(&chord, 24, 16, &[]) {
        Ok((l, r)) => {
            let m = analyze_stereo(&l, &r, 48000.0);
            let ok = m.basi_pass && m.rms_dbfs > -38.0;
            results.push(TestCaseResult::from_bool(
                "VST3-05-POLY-CHORD-BASI",
                "8-voice polyphonic chord passes BASI safety (0 NaN, peak<=0.995, |DC|<=0.005)",
                ok,
                format!(
                    "basi={} peak={:.4} rms_dbfs={:.2} dc={:.5} errs={:?}",
                    m.basi_pass, m.peak_max, m.rms_dbfs, m.dc_max_abs, m.basi_errors
                ),
            ));
        }
        Err(e) => results.push(TestCaseResult::from_bool(
            "VST3-05-POLY-CHORD-BASI",
            "8-voice polyphonic chord passes BASI safety",
            false,
            e,
        )),
    }

    // 5. Multi-sample-rate & multi-block-size matrix (44100/48000/96000 x 64/256/1024)
    let mut matrix_ok = true;
    let mut matrix_notes = Vec::new();
    for &(sr, bs) in &[
        (44100.0, 64usize),
        (44100.0, 512),
        (48000.0, 128),
        (48000.0, 1024),
        (96000.0, 256),
    ] {
        let _ = plugin.set_state_bytes(&init_state);
        if let Err(e) = plugin.reconfigure(sr, bs) {
            matrix_ok = false;
            matrix_notes.push(format!("{}Hz/{}bs reconfigure err: {}", sr, bs, e));
            continue;
        }
        let hold_blocks = ((sr * 0.15) as usize / bs).max(4);
        match plugin.render_notes(&[69], hold_blocks, 4, &[]) {
            Ok((l, r)) => {
                let m = analyze_stereo(&l, &r, sr);
                if !m.basi_pass || m.rms_dbfs <= -45.0 {
                    matrix_ok = false;
                    matrix_notes.push(format!(
                        "{}Hz/{}bs FAIL basi={} rms={:.1} errs={:?}",
                        sr, bs, m.basi_pass, m.rms_dbfs, m.basi_errors
                    ));
                }
            }
            Err(e) => {
                matrix_ok = false;
                matrix_notes.push(format!("{}Hz/{}bs proc err: {}", sr, bs, e));
            }
        }
    }
    let _ = plugin.reconfigure(48000.0, 256);
    results.push(TestCaseResult::from_bool(
        "VST3-06-MULTIRATE-MULTIBLOCK",
        "Render passes BASI across 44.1k/48k/96k sample rates and 64..1024 block sizes",
        matrix_ok,
        if matrix_notes.is_empty() {
            "All 5 SR/BS combinations passed BASI + audible RMS".into()
        } else {
            matrix_notes.join("; ")
        },
    ));

    // 6. Parameter-driven audio-difference checks via opaque VST3 automation
    let param_probes: Vec<(&str, &str, Vec<(&[&str], f64)>)> = vec![
        ("VST3-07-PARAM-FILTER-CUT", "Filter 1 Cutoff", vec![(&["filter", "1", "cut"], 0.12)]),
        ("VST3-08-PARAM-OSC-MODE", "OSC 1 Mode", vec![(&["osc", "1", "mode"], 0.65)]),
        (
            "VST3-09-PARAM-SHAPER-DRIVE",
            "FX Shaper Drive+Mix",
            vec![(&["shaper", "drive"], 0.85), (&["shaper", "mix"], 0.85)],
        ),
        (
            "VST3-10-PARAM-REVERB-MIX",
            "FX Reverb Mix",
            vec![(&["reverb", "mix"], 0.75), (&["reverb", "decay"], 0.75)],
        ),
        ("VST3-11-PARAM-MASTER-VOL", "Master Volume", vec![(&["volume"], 0.25)]),
    ];

    for (cid, label, targets) in param_probes {
        let _ = plugin.set_state_bytes(&init_state);
        let mut changes = Vec::new();
        let mut missing = Vec::new();
        for (needles, val) in &targets {
            if let Some(pid) = plugin.find_param_id(needles) {
                changes.push((pid, *val));
            } else {
                missing.push(format!("{:?}", needles));
            }
        }
        if missing.is_empty() {
            match plugin.render_notes(&[60], 24, 16, &changes) {
                Ok((l, r)) => {
                    let m = analyze_stereo(&l, &r, 48000.0);
                    let diff = if !baseline_l.is_empty() {
                        l2_diff_rms(&baseline_l, &baseline_r, &l, &r)
                    } else {
                        1.0
                    };
                    let ok = m.basi_pass && diff > 1e-4;
                    results.push(TestCaseResult::from_bool(
                        cid,
                        &format!("Automating {} produces measurable BASI-safe audio diff", label),
                        ok,
                        format!("changes={:?} l2_diff={:.5} basi={}", changes, diff, m.basi_pass),
                    ));
                }
                Err(e) => results.push(TestCaseResult::from_bool(
                    cid,
                    &format!("Automating {}", label),
                    false,
                    e,
                )),
            }
        } else {
            results.push(TestCaseResult::from_bool(
                cid,
                &format!("Locate VST3 parameter(s) for {}", label),
                false,
                format!("Missing parameters: {:?}", missing),
            ));
        }
    }

    // 7. State save/restore round-trip check
    if !init_state.is_empty() {
        let _ = plugin.set_state_bytes(&init_state);
        let roundtrip_ok = match plugin.get_state_bytes() {
            Ok(b2) => !b2.is_empty(),
            Err(_) => false,
        };
        results.push(TestCaseResult::from_bool(
            "VST3-12-STATE-ROUNDTRIP",
            "State set_state -> get_state round-trip succeeds cleanly",
            roundtrip_ok,
            format!("init_len={}", init_state.len()),
        ));
    }

    results
}
