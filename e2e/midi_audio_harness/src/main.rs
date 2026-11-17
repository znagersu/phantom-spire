mod analyzers;
mod suite_cases;
mod vst3_host;

use serde::Serialize;
use std::path::PathBuf;
use suite_cases::TestCaseResult;

#[derive(Serialize)]
struct HarnessReport {
    suite: String,
    dry_run: bool,
    plugin: String,
    total: usize,
    passed: usize,
    failed: usize,
    cases: Vec<TestCaseResult>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut dry_run = false;
    let mut plugin_path = PathBuf::from("target/bundled/phantom_spire.vst3");
    let mut out_path = PathBuf::from("e2e/.artifacts/audio_vst3_report.json");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => dry_run = true,
            "--plugin" => {
                if i + 1 < args.len() {
                    plugin_path = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--out" => {
                if i + 1 < args.len() {
                    out_path = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let cases: Vec<TestCaseResult> = if dry_run {
        analyzers::run_dry_run_self_tests()
            .into_iter()
            .map(|(id, name, ok, details)| TestCaseResult::from_bool(&id, &name, ok, details))
            .collect()
    } else {
        suite_cases::run_vst3_opaque_suite(&plugin_path)
    };

    let passed = cases.iter().filter(|c| c.status == "pass").count();
    let failed = cases.len() - passed;

    let report = HarnessReport {
        suite: "midi_audio_harness".to_string(),
        dry_run,
        plugin: plugin_path.display().to_string(),
        total: cases.len(),
        passed,
        failed,
        cases,
    };

    if let Some(parent) = out_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&report) {
        let _ = std::fs::write(&out_path, json);
    }

    println!(
        "[midi_audio_harness] dry_run={} total={} passed={} failed={} report={}",
        dry_run,
        report.total,
        report.passed,
        report.failed,
        out_path.display()
    );
    for c in &report.cases {
        println!("  [{}] {} - {} ({})", c.status.to_uppercase(), c.id, c.name, c.details);
    }

    if failed > 0 {
        std::process::exit(1);
    }
}
