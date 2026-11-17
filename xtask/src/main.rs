use std::path::PathBuf;
use std::process::Command;

fn main() -> nih_plug_xtask::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    nih_plug_xtask::main()?;

    let is_bundle = args.iter().any(|a| a == "bundle");
    let mentions_pkg = args.iter().any(|a| a == "phantom_spire")
        || !args.iter().skip(2).any(|a| !a.starts_with('-'));

    if is_bundle && mentions_pkg {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask must live in workspace root")
            .to_path_buf();

        let is_release = args.iter().any(|a| a == "--release" || a == "-r");
        let custom_profile = args
            .windows(2)
            .find(|w| w[0] == "--profile")
            .map(|w| w[1].clone());

        let mut cmd = Command::new("cargo");
        cmd.current_dir(&workspace_root)
            .args(["build", "--bin", "phantom_spire"]);

        let profile_dir = if let Some(ref prof) = custom_profile {
            cmd.args(["--profile", prof]);
            prof.as_str()
        } else if is_release {
            cmd.arg("--release");
            "release"
        } else {
            "debug"
        };

        let status = cmd.status()?;
        if !status.success() {
            return Err(anyhow::anyhow!(
                "Failed to build standalone binary 'phantom_spire.exe'"
            ));
        }

        let src_exe = workspace_root
            .join("target")
            .join(profile_dir)
            .join("phantom_spire.exe");
        let bundled_dir = workspace_root.join("target").join("bundled");
        std::fs::create_dir_all(&bundled_dir)?;
        let dst_exe = bundled_dir.join("phantom_spire.exe");
        std::fs::copy(&src_exe, &dst_exe)?;
        println!(
            "Bundled standalone executable at '{}'",
            dst_exe.display()
        );
    }

    Ok(())
}
