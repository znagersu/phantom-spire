use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/znagersu/phantom-spire/releases/latest/download/latest.json";
pub const DEFAULT_TRUSTED_PREFIX: &str = "https://github.com/znagersu/phantom-spire/";
pub const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 8;

const UPDATE_TEMP_PREFIX: &str = "PhantomSpireUpdate-";
const CLEANSE_TEMP_PREFIX: &str = "PhantomSpireCleanse-";
const TRASH_TEMP_PREFIX: &str = "PhantomSpireTrash-";
const LOCAL_TRASH_PREFIX: &str = ".PhantomSpireTrash-";
const STAGING_ENTRY_PREFIX: &str = ".phantom_spire.";

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

// ============================================================================
// Configuration & Manifest Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestManifest {
    pub version: String,
    pub zip_url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub vst3_sha256: Option<String>,
    #[serde(default)]
    pub exe_sha256: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub release_notes: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct UpdaterConfigFile {
    current_version: Option<String>,
    manifest_url: Option<String>,
    trusted_url_prefix: Option<String>,
    scan_roots: Option<Vec<PathBuf>>,
    canonical_vst3_dir: Option<PathBuf>,
    canonical_exe_path: Option<PathBuf>,
    temp_dir: Option<PathBuf>,
    silent_mode: Option<bool>,
    auto_update_disabled: Option<bool>,
    http_timeout_secs: Option<u64>,
    require_sha256: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdaterConfig {
    pub current_version: String,
    pub manifest_url: String,
    pub trusted_url_prefix: String,
    pub scan_roots: Vec<PathBuf>,
    pub canonical_vst3_dir: PathBuf,
    pub canonical_exe_path: PathBuf,
    pub temp_dir: PathBuf,
    pub silent_mode: bool,
    pub auto_update_disabled: bool,
    pub http_timeout_secs: u64,
    pub require_sha256: bool,
}

impl Default for UpdaterConfig {
    fn default() -> Self {
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));

        let canonical_vst3_dir = local_app_data
            .join("Programs")
            .join("Common")
            .join("VST3")
            .join("phantom_spire.vst3");
        let canonical_exe_path = local_app_data
            .join("Programs")
            .join("PhantomSpire")
            .join("phantom_spire.exe");

        let mut scan_roots = vec![
            local_app_data.join("Programs").join("Common").join("VST3"),
            local_app_data.join("Programs").join("Common").join("CLAP"),
            local_app_data.join("Programs").join("PhantomSpire"),
        ];

        for env_var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(env_var).map(PathBuf::from) {
                scan_roots.push(base.join("Common Files").join("VST3"));
                scan_roots.push(base.join("Common Files").join("CLAP"));
                scan_roots.push(base.join("VST3"));
                scan_roots.push(base.join("Steinberg").join("VstPlugins"));
            }
        }
        if scan_roots.len() <= 3 {
            scan_roots.push(PathBuf::from("C:\\Program Files\\Common Files\\VST3"));
            scan_roots.push(PathBuf::from("C:\\Program Files\\Common Files\\CLAP"));
        }

        if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
            scan_roots.push(profile.join("Desktop"));
            scan_roots.push(profile.join("Downloads"));
            scan_roots.push(profile.join("Documents").join("Image-Line"));
        }

        Self {
            current_version: String::from(CURRENT_VERSION),
            manifest_url: String::from(DEFAULT_MANIFEST_URL),
            trusted_url_prefix: String::from(DEFAULT_TRUSTED_PREFIX),
            scan_roots,
            canonical_vst3_dir,
            canonical_exe_path,
            temp_dir: std::env::temp_dir(),
            silent_mode: false,
            auto_update_disabled: false,
            http_timeout_secs: DEFAULT_HTTP_TIMEOUT_SECS,
            require_sha256: false,
        }
    }
}

fn env_truthy(name: &str) -> Option<bool> {
    let raw = std::env::var(name).ok()?;
    let v = raw.trim();
    if v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes") {
        Some(true)
    } else if v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("no") {
        Some(false)
    } else {
        None
    }
}

impl UpdaterConfig {
    pub fn default_config_file_path() -> Option<PathBuf> {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("PhantomSpire").join("updater.json"))
    }

    pub fn from_file(path: &Path) -> Result<Self, String> {
        let mut cfg = Self::default();
        cfg.apply_file_overrides(path)?;
        Ok(cfg)
    }

    pub fn apply_file_overrides(&mut self, path: &Path) -> Result<(), String> {
        let raw = fs::read(path)
            .map_err(|e| format!("Failed to read updater config '{}': {e}", path.display()))?;
        let body = strip_utf8_bom(&raw);
        let parsed: UpdaterConfigFile = serde_json::from_slice(body)
            .map_err(|e| format!("Invalid updater config JSON '{}': {e}", path.display()))?;

        if let Some(v) = parsed.current_version.filter(|s| !s.trim().is_empty()) {
            self.current_version = v.trim().to_string();
        }
        if let Some(v) = parsed.manifest_url.filter(|s| !s.trim().is_empty()) {
            self.manifest_url = v.trim().to_string();
        }
        if let Some(v) = parsed.trusted_url_prefix.filter(|s| !s.trim().is_empty()) {
            self.trusted_url_prefix = v.trim().to_string();
        }
        if let Some(roots) = parsed.scan_roots.filter(|r| !r.is_empty()) {
            self.scan_roots = roots;
        }
        if let Some(v) = parsed.canonical_vst3_dir {
            if !v.as_os_str().is_empty() {
                self.canonical_vst3_dir = v;
            }
        }
        if let Some(v) = parsed.canonical_exe_path {
            if !v.as_os_str().is_empty() {
                self.canonical_exe_path = v;
            }
        }
        if let Some(v) = parsed.temp_dir {
            if !v.as_os_str().is_empty() {
                self.temp_dir = v;
            }
        }
        if let Some(v) = parsed.silent_mode {
            self.silent_mode = v;
        }
        if let Some(v) = parsed.auto_update_disabled {
            self.auto_update_disabled = v;
        }
        if let Some(v) = parsed.http_timeout_secs.filter(|&secs| secs > 0) {
            self.http_timeout_secs = v;
        }
        if let Some(v) = parsed.require_sha256 {
            self.require_sha256 = v;
        }
        Ok(())
    }

    pub fn apply_env_overrides(&mut self) {
        if let Some(disabled) = env_truthy("PHANTOM_SPIRE_DISABLE_AUTO_UPDATE") {
            self.auto_update_disabled = disabled;
        }
        if let Ok(ver) = std::env::var("PHANTOM_SPIRE_CURRENT_VERSION") {
            let trimmed = ver.trim();
            if !trimmed.is_empty() {
                self.current_version = trimmed.to_string();
            }
        }
        if let Ok(url) = std::env::var("PHANTOM_SPIRE_MANIFEST_URL") {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                self.manifest_url = trimmed.to_string();
            }
        }
        if let Ok(prefix) = std::env::var("PHANTOM_SPIRE_TRUSTED_PREFIX") {
            let trimmed = prefix.trim();
            if !trimmed.is_empty() {
                self.trusted_url_prefix = trimmed.to_string();
            }
        }
        if let Ok(roots) = std::env::var("PHANTOM_SPIRE_SCAN_ROOTS") {
            let parsed: Vec<PathBuf> = std::env::split_paths(&roots)
                .filter(|p| !p.as_os_str().is_empty())
                .collect();
            if !parsed.is_empty() {
                self.scan_roots = parsed;
            }
        }
        if let Ok(vst3) = std::env::var("PHANTOM_SPIRE_CANONICAL_VST3") {
            let trimmed = vst3.trim();
            if !trimmed.is_empty() {
                self.canonical_vst3_dir = PathBuf::from(trimmed);
            }
        }
        if let Ok(exe) = std::env::var("PHANTOM_SPIRE_CANONICAL_EXE") {
            let trimmed = exe.trim();
            if !trimmed.is_empty() {
                self.canonical_exe_path = PathBuf::from(trimmed);
            }
        }
        if let Ok(temp) = std::env::var("PHANTOM_SPIRE_TEMP_DIR") {
            let trimmed = temp.trim();
            if !trimmed.is_empty() {
                self.temp_dir = PathBuf::from(trimmed);
            }
        }
        if let Some(silent) = env_truthy("PHANTOM_SPIRE_UPDATER_SILENT") {
            self.silent_mode = silent;
        }
        if let Ok(secs_str) = std::env::var("PHANTOM_SPIRE_HTTP_TIMEOUT_SECS") {
            if let Ok(secs) = secs_str.trim().parse::<u64>() {
                if secs > 0 {
                    self.http_timeout_secs = secs;
                }
            }
        }
        if let Some(req) = env_truthy("PHANTOM_SPIRE_REQUIRE_SHA256") {
            self.require_sha256 = req;
        }
    }

    pub fn from_env() -> Self {
        let mut cfg = Self::default();

        // Layer 1: Optional config file (explicit env path or %APPDATA%\PhantomSpire\updater.json)
        let config_path = std::env::var("PHANTOM_SPIRE_UPDATER_CONFIG")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(Self::default_config_file_path);

        if let Some(path) = config_path {
            if path.is_file() {
                let _ = cfg.apply_file_overrides(&path);
            }
        }

        // Layer 2: Environment variable overrides (highest precedence)
        cfg.apply_env_overrides();
        cfg
    }

    pub fn validate_download_url(&self, url: &str) -> Result<(), String> {
        let trimmed = url.trim();
        if trimmed.is_empty()
            || trimmed
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(format!("Malformed download URL '{url}'"));
        }
        if !trimmed.starts_with(&self.trusted_url_prefix) {
            return Err(format!(
                "Untrusted download URL '{trimmed}' (expected prefix '{}')",
                self.trusted_url_prefix
            ));
        }
        Ok(())
    }
}

// ============================================================================
// Status, Cleanse Report & Handle
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate {
        stale_copies: usize,
    },
    Available {
        version: String,
        zip_url: String,
        stale_copies: usize,
    },
    Installing(String),
    ReadyToRestart {
        version: String,
        message: String,
    },
    Error(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaleDiscovery {
    pub stale_vst3: Vec<PathBuf>,
    pub stale_bak: Vec<PathBuf>,
    pub stale_clap: Vec<PathBuf>,
    pub stale_exe: Vec<PathBuf>,
    pub nested_bundles: Vec<PathBuf>,
}

impl StaleDiscovery {
    pub fn total_count(&self) -> usize {
        self.stale_vst3.len()
            + self.stale_bak.len()
            + self.stale_clap.len()
            + self.stale_exe.len()
            + self.nested_bundles.len()
    }

    pub fn all_paths(&self) -> Vec<PathBuf> {
        let mut out = Vec::with_capacity(self.total_count());
        out.extend(self.nested_bundles.iter().cloned());
        out.extend(self.stale_bak.iter().cloned());
        out.extend(self.stale_vst3.iter().cloned());
        out.extend(self.stale_clap.iter().cloned());
        out.extend(self.stale_exe.iter().cloned());
        out
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanseReport {
    pub stale_removed: usize,
    pub stale_hot_swapped: usize,
    pub temp_artifacts_removed: usize,
    pub deferred_locked_trash: usize,
    pub removed_paths: Vec<PathBuf>,
}

#[derive(Clone)]
pub struct UpdaterHandle {
    config: Arc<UpdaterConfig>,
    status: Arc<Mutex<UpdateStatus>>,
    last_manifest: Arc<Mutex<Option<LatestManifest>>>,
    startup_triggered: Arc<AtomicBool>,
}

impl Default for UpdaterHandle {
    /// Non-blocking, zero-spawn initialization safe for DAW and `pluginval` scans.
    fn default() -> Self {
        Self::new(UpdaterConfig::from_env())
    }
}

impl UpdaterHandle {
    pub fn new(config: UpdaterConfig) -> Self {
        Self {
            config: Arc::new(config),
            status: Arc::new(Mutex::new(UpdateStatus::Idle)),
            last_manifest: Arc::new(Mutex::new(None)),
            startup_triggered: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| UpdateStatus::Error(String::from("Updater lock poisoned")))
    }

    pub fn config(&self) -> &UpdaterConfig {
        &self.config
    }

    pub fn is_auto_update_disabled(&self) -> bool {
        self.config.auto_update_disabled
    }

    pub fn last_manifest(&self) -> Option<LatestManifest> {
        self.last_manifest.lock().ok().and_then(|g| g.clone())
    }

    /// Deferred startup check (called when editor opens, never from `Plugin::default()`).
    pub fn trigger_startup_check_if_enabled(&self) {
        if self.config.auto_update_disabled {
            return;
        }
        if self
            .startup_triggered
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.check_async();
        }
    }

    /// Spawns a non-blocking background thread to check GitHub Releases and scan for stale copies.
    pub fn check_async(&self) {
        {
            let mut guard = match self.status.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if matches!(
                *guard,
                UpdateStatus::Checking
                    | UpdateStatus::Installing(_)
                    | UpdateStatus::ReadyToRestart { .. }
            ) {
                return;
            }
            *guard = UpdateStatus::Checking;
        }

        let config = self.config.clone();
        let status = self.status.clone();
        let last_manifest = self.last_manifest.clone();

        let _ = std::thread::Builder::new()
            .name(String::from("phantom-spire-update-check"))
            .spawn(move || {
                let (next_status, manifest) = Self::check_with_config_detailed(&config);
                if let Some(m) = manifest {
                    if let Ok(mut mg) = last_manifest.lock() {
                        *mg = Some(m);
                    }
                }
                if let Ok(mut sg) = status.lock() {
                    *sg = next_status;
                }
            });
    }

    /// Spawns a non-blocking background thread to download, verify, hot-swap install, and cleanse.
    pub fn trigger_update_async(&self, version: String, zip_url: String) {
        {
            let mut guard = match self.status.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if matches!(
                *guard,
                UpdateStatus::Installing(_) | UpdateStatus::ReadyToRestart { .. }
            ) {
                return;
            }
            *guard = UpdateStatus::Installing(format!("v{version}"));
        }

        let cached_manifest = self.last_manifest();
        let (expected_zip_sha, expected_vst3_sha, expected_exe_sha) = match cached_manifest {
            Some(ref m) if m.version == version && m.zip_url == zip_url => (
                m.sha256.clone(),
                m.vst3_sha256.clone(),
                m.exe_sha256.clone(),
            ),
            _ => (None, None, None),
        };

        let config = self.config.clone();
        let status = self.status.clone();

        let _ = std::thread::Builder::new()
            .name(String::from("phantom-spire-update-install"))
            .spawn(move || {
                let result = run_update_pipeline_internal(
                    &config,
                    &version,
                    &zip_url,
                    expected_zip_sha.as_deref(),
                    expected_vst3_sha.as_deref(),
                    expected_exe_sha.as_deref(),
                    !config.silent_mode,
                );
                if let Ok(mut guard) = status.lock() {
                    *guard = match result {
                        Ok(done) => done,
                        Err(err) => UpdateStatus::Error(err),
                    };
                }
            });
    }

    /// Alias matching Phantom Voicer's `install_async` method name.
    pub fn install_async(&self, version: String, zip_url: String) {
        self.trigger_update_async(version, zip_url);
    }

    /// Convenience helper for UI buttons when status is `UpdateStatus::Available`.
    pub fn trigger_update_from_available_async(&self) {
        if let UpdateStatus::Available {
            version, zip_url, ..
        } = self.status()
        {
            self.trigger_update_async(version, zip_url);
        }
    }

    /// Spawns a non-blocking background thread to cleanse all stale binaries and temp directories.
    pub fn cleanse_async(&self) {
        {
            let mut guard = match self.status.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if matches!(
                *guard,
                UpdateStatus::Installing(_) | UpdateStatus::ReadyToRestart { .. }
            ) {
                return;
            }
            *guard = UpdateStatus::Installing(String::from("Cleanse"));
        }

        let config = self.config.clone();
        let status = self.status.clone();

        let _ = std::thread::Builder::new()
            .name(String::from("phantom-spire-cleanse"))
            .spawn(move || {
                let result = run_cleanse_internal(&config, !config.silent_mode);
                if let Ok(mut guard) = status.lock() {
                    *guard = match result {
                        Ok(report) => {
                            let total = report.stale_removed + report.stale_hot_swapped;
                            if report.stale_hot_swapped > 0 {
                                UpdateStatus::ReadyToRestart {
                                    version: config.current_version.clone(),
                                    message: format!(
                                        "Cleansed {total} stale copy(s) ({} hot-swapped) & {} temp item(s)",
                                        report.stale_hot_swapped, report.temp_artifacts_removed
                                    ),
                                }
                            } else {
                                UpdateStatus::UpToDate { stale_copies: 0 }
                            }
                        }
                        Err(err) => UpdateStatus::Error(err),
                    };
                }
            });
    }

    // ---- Synchronous / Deterministic Helpers for Tests & Direct Calls ----

    pub fn fetch_manifest(config: &UpdaterConfig) -> Result<LatestManifest, String> {
        fetch_manifest_with_config(config)
    }

    pub fn check_with_config(config: &UpdaterConfig) -> UpdateStatus {
        Self::check_with_config_detailed(config).0
    }

    pub fn check_with_config_detailed(
        config: &UpdaterConfig,
    ) -> (UpdateStatus, Option<LatestManifest>) {
        let stale_copies = count_stale_copies(config);
        match fetch_manifest_with_config(config) {
            Ok(manifest) => {
                if !is_valid_semver(&manifest.version) {
                    return (
                        UpdateStatus::Error(format!(
                            "Invalid release version in manifest: '{}'",
                            manifest.version
                        )),
                        None,
                    );
                }
                if let Err(err) = config.validate_download_url(&manifest.zip_url) {
                    return (UpdateStatus::Error(err), None);
                }
                if let Some(ref sha) = manifest.sha256 {
                    if !is_valid_sha256_hex(sha) {
                        return (
                            UpdateStatus::Error(format!("Malformed manifest sha256: '{sha}'")),
                            None,
                        );
                    }
                } else if config.require_sha256 {
                    return (
                        UpdateStatus::Error(String::from(
                            "Manifest missing required sha256 checksum",
                        )),
                        None,
                    );
                }

                if is_newer_version(&manifest.version, &config.current_version) {
                    let status = UpdateStatus::Available {
                        version: manifest.version.clone(),
                        zip_url: manifest.zip_url.clone(),
                        stale_copies,
                    };
                    (status, Some(manifest))
                } else {
                    (UpdateStatus::UpToDate { stale_copies }, Some(manifest))
                }
            }
            Err(err) => (UpdateStatus::Error(err), None),
        }
    }

    pub fn run_update_pipeline(
        config: &UpdaterConfig,
        version: &str,
        zip_url: &str,
    ) -> Result<UpdateStatus, String> {
        let mut expected_zip_sha = None;
        let mut expected_vst3_sha = None;
        let mut expected_exe_sha = None;

        if config.require_sha256 {
            if let Ok(manifest) = fetch_manifest_with_config(config) {
                if manifest.zip_url == zip_url {
                    expected_zip_sha = manifest.sha256;
                    expected_vst3_sha = manifest.vst3_sha256;
                    expected_exe_sha = manifest.exe_sha256;
                }
            }
        }

        run_update_pipeline_internal(
            config,
            version,
            zip_url,
            expected_zip_sha.as_deref(),
            expected_vst3_sha.as_deref(),
            expected_exe_sha.as_deref(),
            false,
        )
    }

    pub fn run_update_pipeline_verified(
        config: &UpdaterConfig,
        version: &str,
        zip_url: &str,
        expected_zip_sha256: Option<&str>,
        expected_vst3_sha256: Option<&str>,
        expected_exe_sha256: Option<&str>,
    ) -> Result<UpdateStatus, String> {
        run_update_pipeline_internal(
            config,
            version,
            zip_url,
            expected_zip_sha256,
            expected_vst3_sha256,
            expected_exe_sha256,
            false,
        )
    }

    pub fn run_update_from_manifest(
        config: &UpdaterConfig,
        manifest: &LatestManifest,
    ) -> Result<UpdateStatus, String> {
        run_update_pipeline_internal(
            config,
            &manifest.version,
            &manifest.zip_url,
            manifest.sha256.as_deref(),
            manifest.vst3_sha256.as_deref(),
            manifest.exe_sha256.as_deref(),
            false,
        )
    }

    pub fn apply_update_from_zip_for_test(
        config: &UpdaterConfig,
        zip_path: &Path,
    ) -> Result<UpdateStatus, String> {
        Self::apply_update_from_zip(config, &config.current_version, zip_path, None, None, None)
    }

    pub fn apply_update_from_zip(
        config: &UpdaterConfig,
        version: &str,
        zip_path: &Path,
        expected_zip_sha256: Option<&str>,
        expected_vst3_sha256: Option<&str>,
        expected_exe_sha256: Option<&str>,
    ) -> Result<UpdateStatus, String> {
        apply_update_from_zip_internal(
            config,
            version,
            zip_path,
            expected_zip_sha256,
            expected_vst3_sha256,
            expected_exe_sha256,
            false,
        )
    }

    pub fn cleanse_with_config(config: &UpdaterConfig) -> Result<CleanseReport, String> {
        run_cleanse_internal(config, false)
    }
}

// Public module-level convenience wrappers matching the brief
pub fn check_with_config(config: &UpdaterConfig) -> UpdateStatus {
    UpdaterHandle::check_with_config(config)
}

pub fn run_update_pipeline(
    config: &UpdaterConfig,
    version: &str,
    zip_url: &str,
) -> Result<UpdateStatus, String> {
    UpdaterHandle::run_update_pipeline(config, version, zip_url)
}

pub fn cleanse_with_config(config: &UpdaterConfig) -> Result<CleanseReport, String> {
    UpdaterHandle::cleanse_with_config(config)
}

// ============================================================================
// Strict SemVer 2.0.0 Parsing & Comparison
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreIdent {
    Num(u64),
    AlphaNum(String),
}

impl Ord for PreIdent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (PreIdent::Num(a), PreIdent::Num(b)) => a.cmp(b),
            (PreIdent::Num(_), PreIdent::AlphaNum(_)) => Ordering::Less,
            (PreIdent::AlphaNum(_), PreIdent::Num(_)) => Ordering::Greater,
            (PreIdent::AlphaNum(a), PreIdent::AlphaNum(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PreIdent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<Vec<PreIdent>>,
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)) {
            Ordering::Equal => match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => {
                    for (x, y) in a.iter().zip(b.iter()) {
                        let ord = x.cmp(y);
                        if ord != Ordering::Equal {
                            return ord;
                        }
                    }
                    a.len().cmp(&b.len())
                }
            },
            ord => ord,
        }
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_semver(raw: &str) -> Option<SemVer> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let stripped = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    let no_build = stripped.split('+').next()?;
    if no_build.is_empty() {
        return None;
    }
    let (core, pre_str) = match no_build.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (no_build, None),
    };

    let core_parts: Vec<&str> = core.split('.').collect();
    if core_parts.is_empty() || core_parts.len() > 3 {
        return None;
    }
    let major = parse_strict_u64(core_parts[0])?;
    let minor = if core_parts.len() >= 2 {
        parse_strict_u64(core_parts[1])?
    } else {
        0
    };
    let patch = if core_parts.len() == 3 {
        parse_strict_u64(core_parts[2])?
    } else {
        0
    };

    let pre = match pre_str {
        None => None,
        Some(p) => {
            if p.is_empty() {
                return None;
            }
            let mut idents = Vec::new();
            for part in p.split('.') {
                if part.is_empty()
                    || !part
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-')
                {
                    return None;
                }
                if part.chars().all(|c| c.is_ascii_digit()) {
                    idents.push(PreIdent::Num(part.parse::<u64>().ok()?));
                } else {
                    idents.push(PreIdent::AlphaNum(part.to_string()));
                }
            }
            Some(idents)
        }
    };

    Some(SemVer {
        major,
        minor,
        patch,
        pre,
    })
}

fn parse_strict_u64(s: &str) -> Option<u64> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse::<u64>().ok()
}

pub fn is_valid_semver(raw: &str) -> bool {
    parse_semver(raw).is_some()
}

pub fn is_newer_version(remote: &str, current: &str) -> bool {
    match (parse_semver(remote), parse_semver(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

// ============================================================================
// Pure-Rust FIPS 180-4 SHA-256 (Zero External Crate Dependencies)
// ============================================================================

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
    0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
    0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
    0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
    0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
    0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                0x1f83d9ab, 0x5be0cd19,
            ],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let block: [u8; 64] = data[..64].try_into().unwrap();
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn finalize_hex(mut self) -> String {
        let bit_len = self.total_len.wrapping_mul(8);
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;

        if self.buf_len > 56 {
            for b in &mut self.buf[self.buf_len..64] {
                *b = 0;
            }
            let block = self.buf;
            self.compress(&block);
            self.buf_len = 0;
        }
        for b in &mut self.buf[self.buf_len..56] {
            *b = 0;
        }
        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.buf;
        self.compress(&block);

        let mut out = String::with_capacity(64);
        for word in self.state {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{word:08x}");
        }
        out
    }
}

pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize_hex()
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|e| format!("Failed to open '{}' for SHA-256: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Failed to read '{}' for SHA-256: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize_hex())
}

fn is_valid_sha256_hex(s: &str) -> bool {
    let trimmed = s.trim();
    trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())
}

// ============================================================================
// HTTP Fetching (Cache-Busted, BOM-Tolerant, Short Timeout)
// ============================================================================

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn unique_nonce() -> String {
    let seq = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{seq}", std::process::id(), unix_millis())
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn cache_bust_url(url: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}t={}", unix_millis())
}

fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn fetch_manifest_with_config(config: &UpdaterConfig) -> Result<LatestManifest, String> {
    let manifest_url = config.manifest_url.trim();
    if manifest_url.is_empty() {
        return Err(String::from("Manifest URL is empty"));
    }

    let busted = cache_bust_url(manifest_url);
    let max_time = config.http_timeout_secs.max(1);
    let connect_timeout = max_time.min(5);

    let output = hidden_command("curl.exe")
        .args([
            "-fsSL",
            "-H",
            "Cache-Control: no-cache",
            "-H",
            "Pragma: no-cache",
            "--connect-timeout",
            &connect_timeout.to_string(),
            "--max-time",
            &max_time.to_string(),
            &busted,
        ])
        .output()
        .map_err(|e| format!("Failed to invoke curl.exe: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("exit code {:?}", output.status.code())
        } else {
            stderr
        };
        return Err(format!("Offline or release unavailable: {detail}"));
    }

    let body = strip_utf8_bom(&output.stdout);
    serde_json::from_slice::<LatestManifest>(body)
        .map_err(|e| format!("Invalid manifest JSON: {e}"))
}

fn download_file_to(config: &UpdaterConfig, url: &str, dest_path: &Path) -> Result<(), String> {
    config.validate_download_url(url)?;
    let busted = cache_bust_url(url);
    let base_timeout = config.http_timeout_secs.max(1);
    let download_timeout = if base_timeout < DEFAULT_HTTP_TIMEOUT_SECS {
        base_timeout
    } else {
        (base_timeout.saturating_mul(8)).min(120)
    };
    let connect_timeout = base_timeout.min(5);

    let output = hidden_command("curl.exe")
        .args([
            "-fsSL",
            "-L",
            "-H",
            "Cache-Control: no-cache",
            "--connect-timeout",
            &connect_timeout.to_string(),
            "--max-time",
            &download_timeout.to_string(),
            "-o",
            &dest_path.to_string_lossy(),
            &busted,
        ])
        .output()
        .map_err(|e| format!("Failed to invoke curl.exe for download: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            String::from("Download failed")
        } else {
            format!("Download failed: {stderr}")
        });
    }

    let meta = fs::metadata(dest_path)
        .map_err(|e| format!("Downloaded archive missing '{}': {e}", dest_path.display()))?;
    if meta.len() == 0 {
        return Err(String::from("Downloaded archive is empty"));
    }
    Ok(())
}

// ============================================================================
// Active Module Detection (`GetModuleHandleExW`) & Windows File Locking
// ============================================================================

#[cfg(target_os = "windows")]
pub fn current_module_path() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    extern "system" {
        fn GetModuleHandleExW(dwFlags: u32, lpModuleName: *const u8, phModule: *mut usize) -> i32;
        fn GetModuleFileNameW(hModule: usize, lpFilename: *mut u16, nSize: u32) -> u32;
    }

    const FLAG_FROM_ADDRESS: u32 = 0x0000_0004;
    const FLAG_UNCHANGED_REFCOUNT: u32 = 0x0000_0002;

    let mut hmod: usize = 0;
    let ok = unsafe {
        GetModuleHandleExW(
            FLAG_FROM_ADDRESS | FLAG_UNCHANGED_REFCOUNT,
            current_module_path as *const u8,
            &mut hmod,
        )
    };
    if ok == 0 || hmod == 0 {
        return None;
    }

    let mut buf = vec![0u16; 1024];
    let len = unsafe { GetModuleFileNameW(hmod, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&buf[..len])))
}

#[cfg(not(target_os = "windows"))]
pub fn current_module_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Detects the currently loaded plugin bundle (`.vst3`) or standalone executable via `GetModuleHandleExW`.
pub fn detect_active_target() -> Result<(PathBuf, PathBuf), String> {
    let mod_path = current_module_path().ok_or_else(|| String::from("Could not locate plugin path"))?;

    for anc in mod_path.ancestors() {
        if let Some(ext) = anc.extension() {
            if ext.eq_ignore_ascii_case("vst3") && anc.is_dir() {
                let parent = anc
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or_else(|| String::from("Missing VST3 parent directory"))?;
                return Ok((anc.to_path_buf(), parent));
            }
        }
    }

    let parent = mod_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("Missing install directory"))?;
    Ok((mod_path, parent))
}

#[cfg(target_os = "windows")]
fn schedule_delete_on_reboot_best_effort(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    extern "system" {
        fn MoveFileExW(
            lpExistingFileName: *const u16,
            lpNewFileName: *const u16,
            dwFlags: u32,
        ) -> i32;
    }
    const MOVEFILE_DELAY_UNTIL_REBOOT: u32 = 0x0000_0004;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let _ = MoveFileExW(wide.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT);
    }
}

#[cfg(not(target_os = "windows"))]
fn schedule_delete_on_reboot_best_effort(_path: &Path) {}

#[cfg(target_os = "windows")]
fn is_file_locked(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    if !path.is_file() {
        return false;
    }
    // Probe exclusive access (share_mode = 0). If another handle has the file open
    // (including LoadLibraryW / running EXE with FILE_SHARE_READ|FILE_SHARE_DELETE),
    // exclusive open fails cleanly without marking the file delete-pending.
    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
        .is_err()
}

#[cfg(not(target_os = "windows"))]
fn is_file_locked(_path: &Path) -> bool {
    false
}

fn dir_contains_locked_file(dir: &Path) -> bool {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if dir_contains_locked_file(&p) {
                return true;
            }
        } else if is_file_locked(&p) {
            return true;
        }
    }
    false
}

fn is_path_locked(path: &Path) -> bool {
    if path.is_dir() {
        dir_contains_locked_file(path)
    } else {
        is_file_locked(path)
    }
}

fn path_volume_root(path: &Path) -> Option<String> {
    let normalized = normalize_path_key(path);
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return Some(format!("{}:\\", bytes[0] as char).to_ascii_lowercase());
    }
    None
}

fn normalize_path_key(path: &Path) -> String {
    let resolved = if path.exists() {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if parent.exists() {
            fs::canonicalize(parent)
                .map(|p| p.join(name))
                .unwrap_or_else(|_| path.to_path_buf())
        } else {
            path.to_path_buf()
        }
    } else {
        path.to_path_buf()
    };

    let s = resolved.to_string_lossy();
    let stripped = s
        .strip_prefix("\\\\?\\")
        .or_else(|| s.strip_prefix("//?/"))
        .unwrap_or(&s);
    let cleaned = stripped.replace('/', "\\").trim_end_matches('\\').to_string();
    #[cfg(target_os = "windows")]
    {
        cleaned.to_ascii_lowercase()
    }
    #[cfg(not(target_os = "windows"))]
    {
        cleaned
    }
}

fn is_dir_writable(dir: &Path) -> bool {
    let mut probe_dir = dir;
    while !probe_dir.exists() {
        match probe_dir.parent() {
            Some(p) if p != probe_dir => probe_dir = p,
            _ => break,
        }
    }
    if !probe_dir.is_dir() {
        return false;
    }
    let probe = probe_dir.join(format!(".ps_probe_{}", unique_nonce()));
    match File::create(&probe) {
        Ok(mut f) => {
            let ok = f.write_all(b"ok").is_ok();
            drop(f);
            let _ = fs::remove_file(&probe);
            ok
        }
        Err(_) => false,
    }
}

// ============================================================================
// RAII Temp Directory Guard & Artifact Cleanup
// ============================================================================

struct TempDirGuard {
    path: PathBuf,
    disarmed: bool,
}

impl TempDirGuard {
    fn create(base: &Path, prefix: &str) -> Result<Self, String> {
        fs::create_dir_all(base)
            .map_err(|e| format!("Failed to create temp base '{}': {e}", base.display()))?;
        let path = base.join(format!("{prefix}{}", unique_nonce()));
        fs::create_dir_all(&path)
            .map_err(|e| format!("Failed to create staging dir '{}': {e}", path.display()))?;
        Ok(Self {
            path,
            disarmed: false,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if !self.disarmed && self.path.exists() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Removes all `PhantomSpireUpdate-*`, `PhantomSpireCleanse-*`, and `PhantomSpireTrash-*`
/// artifacts from `temp_dir`, plus any `.PhantomSpireTrash-*` / `.phantom_spire.*.new-*`
/// leftovers in `extra_roots`.
/// Returns `(removed_count, deferred_locked_count)`.
pub fn cleanup_temp_artifacts(temp_dir: &Path, extra_roots: &[PathBuf]) -> (usize, usize) {
    let mut removed = 0usize;
    let mut deferred_locked = 0usize;

    if let Ok(entries) = fs::read_dir(temp_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            let is_managed = lower.starts_with(&UPDATE_TEMP_PREFIX.to_ascii_lowercase())
                || lower.starts_with(&CLEANSE_TEMP_PREFIX.to_ascii_lowercase())
                || lower.starts_with(&TRASH_TEMP_PREFIX.to_ascii_lowercase());
            if !is_managed {
                continue;
            }
            if try_remove_path_completely(&p) {
                removed += 1;
            } else {
                deferred_locked += 1;
                schedule_delete_on_reboot_best_effort(&p);
            }
        }
    }

    let mut seen_roots = HashSet::new();
    for root in extra_roots {
        let key = normalize_path_key(root);
        if !seen_roots.insert(key) {
            continue;
        }
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let p = entry.path();
                let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let lower = name.to_ascii_lowercase();
                let is_local_temp = lower.starts_with(&LOCAL_TRASH_PREFIX.to_ascii_lowercase())
                    || (lower.starts_with(STAGING_ENTRY_PREFIX) && lower.contains(".new-"));
                if !is_local_temp {
                    continue;
                }
                if try_remove_path_completely(&p) {
                    removed += 1;
                } else {
                    deferred_locked += 1;
                    schedule_delete_on_reboot_best_effort(&p);
                }
            }
        }
    }

    (removed, deferred_locked)
}

fn try_remove_path_completely(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    if path.is_dir() {
        if is_path_locked(path) {
            // Remove any unlocked children inside while leaving only actively locked files
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    let child = entry.path();
                    if !is_path_locked(&child) {
                        let _ = if child.is_dir() {
                            fs::remove_dir_all(&child)
                        } else {
                            fs::remove_file(&child)
                        };
                    }
                }
            }
            return fs::remove_dir(path).is_ok();
        }
        fs::remove_dir_all(path).is_ok()
    } else {
        if is_file_locked(path) {
            return false;
        }
        fs::remove_file(path).is_ok()
    }
}

// ============================================================================
// NTFS Same-Volume Move-to-Trash Hot-Swap
// ============================================================================

enum RemoveOutcome {
    NotFound,
    Deleted,
    HotSwappedToTrash(PathBuf),
}

fn select_same_volume_trash_dir(target: &Path, config: &UpdaterConfig) -> Result<PathBuf, String> {
    let target_vol = path_volume_root(target);
    let temp_vol = path_volume_root(&config.temp_dir);

    let trash_dir = if target_vol.is_some() && target_vol == temp_vol {
        config
            .temp_dir
            .join(format!("{TRASH_TEMP_PREFIX}{}", unique_nonce()))
    } else if let Some(parent) = target.parent() {
        parent.join(format!("{LOCAL_TRASH_PREFIX}{}", unique_nonce()))
    } else {
        config
            .temp_dir
            .join(format!("{TRASH_TEMP_PREFIX}{}", unique_nonce()))
    };

    fs::create_dir_all(&trash_dir)
        .map_err(|e| format!("Failed to create trash dir '{}': {e}", trash_dir.display()))?;
    Ok(trash_dir)
}

fn move_locked_children_out(dir: &Path, trash_dir: &Path) -> Result<bool, String> {
    let mut moved_any = false;
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("Failed to inspect locked dir '{}': {e}", dir.display()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if move_locked_children_out(&p, trash_dir)? {
                moved_any = true;
            }
        } else if is_file_locked(&p) {
            let leaf = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| String::from("locked.bin"));
            let dest = trash_dir.join(format!("{leaf}.{}", unique_nonce()));
            fs::rename(&p, &dest).map_err(|e| {
                format!(
                    "Failed to hot-swap locked file '{}' -> '{}': {e}",
                    p.display(),
                    dest.display()
                )
            })?;
            schedule_delete_on_reboot_best_effort(&dest);
            moved_any = true;
        }
    }
    Ok(moved_any)
}

fn remove_or_hot_swap(target: &Path, config: &UpdaterConfig) -> Result<RemoveOutcome, String> {
    if !target.exists() {
        return Ok(RemoveOutcome::NotFound);
    }

    let locked = is_path_locked(target);
    if !locked {
        let del = if target.is_dir() {
            fs::remove_dir_all(target)
        } else {
            fs::remove_file(target)
        };
        if del.is_ok() {
            return Ok(RemoveOutcome::Deleted);
        }
    }

    // File or bundle is locked in RAM (e.g. DAW holding VST3 DLL or running EXE).
    // NTFS allows moving locked files on the same volume out of the canonical folder!
    let trash_dir = select_same_volume_trash_dir(target, config)?;
    let leaf = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("phantom_spire.old"));
    let dest = trash_dir.join(format!("{leaf}.{}", unique_nonce()));

    if fs::rename(target, &dest).is_ok() {
        schedule_delete_on_reboot_best_effort(&dest);
        return Ok(RemoveOutcome::HotSwappedToTrash(dest));
    }

    // On some Windows versions, renaming a parent directory containing an open handle
    // fails even though renaming the open child file itself succeeds.
    if target.is_dir() {
        move_locked_children_out(target, &trash_dir)?;
        fs::remove_dir_all(target).map_err(|e| {
            format!(
                "Failed to remove dir '{}' after moving locked files: {e}",
                target.display()
            )
        })?;
        return Ok(RemoveOutcome::HotSwappedToTrash(trash_dir));
    }

    Err(format!(
        "Cannot delete or hot-swap locked file '{}'",
        target.display()
    ))
}

// ============================================================================
// Stale Binary Discovery & Aggressive Cleansing
// ============================================================================

fn scan_dir_for_stale(
    dir: &Path,
    depth: usize,
    canon_vst3_key: &str,
    canon_vst3_dll_key: &str,
    canon_exe_key: &str,
    seen: &mut HashSet<String>,
    out: &mut StaleDiscovery,
) {
    if depth == 0 || !dir.is_dir() {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let p = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        // Avoid symlink/junction cycles
        if ft.is_symlink() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let key = normalize_path_key(&p);

        if lower == "phantom_spire.vst3" {
            if ft.is_dir() {
                if key == canon_vst3_key {
                    // Canonical bundle: check ONLY for legacy nested bundles or .bak inside it
                    scan_canonical_vst3_for_nested(&p, seen, out);
                } else if seen.insert(key) {
                    out.stale_vst3.push(p);
                }
                continue; // Do not recurse inside VST3 bundles
            } else if ft.is_file() {
                // Never flag the legitimate DLL inside canonical_vst3_dir/Contents/x86_64-win/
                if key != canon_vst3_dll_key && seen.insert(key) {
                    out.stale_vst3.push(p);
                }
                continue;
            }
        }

        if lower == "phantom_spire.vst3.bak" {
            if seen.insert(key) {
                out.stale_bak.push(p);
            }
            continue;
        }

        if lower == "phantom_spire.clap" {
            if seen.insert(key) {
                out.stale_clap.push(p);
            }
            continue;
        }

        if lower == "phantom_spire.exe" && ft.is_file() {
            if key != canon_exe_key && seen.insert(key) {
                out.stale_exe.push(p);
            }
            continue;
        }

        if ft.is_dir() {
            // Skip VCS / hidden trash folders during stale scan
            if lower == ".git"
                || lower == "node_modules"
                || lower.starts_with(&LOCAL_TRASH_PREFIX.to_ascii_lowercase())
            {
                continue;
            }
            scan_dir_for_stale(
                &p,
                depth - 1,
                canon_vst3_key,
                canon_vst3_dll_key,
                canon_exe_key,
                seen,
                out,
            );
        }
    }
}

fn scan_canonical_vst3_for_nested(
    canon_vst3: &Path,
    seen: &mut HashSet<String>,
    out: &mut StaleDiscovery,
) {
    let nested_dir = canon_vst3.join("phantom_spire.vst3");
    if nested_dir.exists() {
        let key = normalize_path_key(&nested_dir);
        if seen.insert(key) {
            out.nested_bundles.push(nested_dir);
        }
    }
    let nested_bak = canon_vst3.join("phantom_spire.vst3.bak");
    if nested_bak.exists() {
        let key = normalize_path_key(&nested_bak);
        if seen.insert(key) {
            out.stale_bak.push(nested_bak);
        }
    }
    // Check Contents/ for stray nested .vst3 directories or .bak files
    let contents = canon_vst3.join("Contents");
    if contents.is_dir() {
        if let Ok(entries) = fs::read_dir(&contents) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    let lower = name.to_ascii_lowercase();
                    if lower == "phantom_spire.vst3" && p.is_dir() {
                        let key = normalize_path_key(&p);
                        if seen.insert(key) {
                            out.nested_bundles.push(p);
                        }
                    } else if lower == "phantom_spire.vst3.bak" {
                        let key = normalize_path_key(&p);
                        if seen.insert(key) {
                            out.stale_bak.push(p);
                        }
                    }
                }
            }
        }
    }
}

pub fn discover_stale_copies(config: &UpdaterConfig) -> StaleDiscovery {
    let canon_vst3_key = normalize_path_key(&config.canonical_vst3_dir);
    let canon_vst3_dll = config
        .canonical_vst3_dir
        .join("Contents")
        .join("x86_64-win")
        .join("phantom_spire.vst3");
    let canon_vst3_dll_key = normalize_path_key(&canon_vst3_dll);
    let canon_exe_key = normalize_path_key(&config.canonical_exe_path);

    let mut seen = HashSet::new();
    let mut discovery = StaleDiscovery::default();

    // Always inspect canonical VST3 bundle itself for legacy nested bundles
    if config.canonical_vst3_dir.is_dir() {
        scan_canonical_vst3_for_nested(&config.canonical_vst3_dir, &mut seen, &mut discovery);
    }

    // Always inspect canonical parent directories for stray .bak / .clap / duplicate files
    let mut roots_to_scan = config.scan_roots.clone();
    if let Some(p) = config.canonical_vst3_dir.parent() {
        roots_to_scan.push(p.to_path_buf());
    }
    if let Some(p) = config.canonical_exe_path.parent() {
        roots_to_scan.push(p.to_path_buf());
    }

    let mut visited_roots = HashSet::new();
    for root in roots_to_scan {
        if !root.is_dir() {
            continue;
        }
        let rkey = normalize_path_key(&root);
        if !visited_roots.insert(rkey) {
            continue;
        }
        scan_dir_for_stale(
            &root,
            6,
            &canon_vst3_key,
            &canon_vst3_dll_key,
            &canon_exe_key,
            &mut seen,
            &mut discovery,
        );
    }

    // If the currently loaded module itself is a stale phantom_spire.* strictly inside
    // one of the configured scan roots (e.g. deeply nested > 6 levels), include it too.
    // Never wander outside configured scan roots (protects target/bundled during sandboxed E2E).
    if let Ok((active_target, _)) = detect_active_target() {
        let within_scan_root = config.scan_roots.iter().any(|root| {
            if !root.is_dir() {
                return false;
            }
            let rkey = normalize_path_key(root);
            let akey = normalize_path_key(&active_target);
            akey == rkey || akey.starts_with(&format!("{rkey}\\"))
        });
        if within_scan_root {
            if let Some(name) = active_target.file_name().and_then(|n| n.to_str()) {
                let lower = name.to_ascii_lowercase();
                let akey = normalize_path_key(&active_target);
                match lower.as_str() {
                    "phantom_spire.vst3" if akey != canon_vst3_key => {
                        if seen.insert(akey) {
                            discovery.stale_vst3.push(active_target);
                        }
                    }
                    "phantom_spire.clap" => {
                        if seen.insert(akey) {
                            discovery.stale_clap.push(active_target);
                        }
                    }
                    "phantom_spire.exe" if akey != canon_exe_key => {
                        if seen.insert(akey) {
                            discovery.stale_exe.push(active_target);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    discovery
}

pub fn count_stale_copies(config: &UpdaterConfig) -> usize {
    discover_stale_copies(config).total_count()
}

fn collect_extra_cleanup_roots(config: &UpdaterConfig) -> Vec<PathBuf> {
    let mut roots = config.scan_roots.clone();
    if let Some(p) = config.canonical_vst3_dir.parent() {
        roots.push(p.to_path_buf());
    }
    if let Some(p) = config.canonical_exe_path.parent() {
        roots.push(p.to_path_buf());
    }
    roots
}

fn run_cleanse_internal(
    config: &UpdaterConfig,
    allow_uac_elevation: bool,
) -> Result<CleanseReport, String> {
    let discovery = discover_stale_copies(config);
    let stale_paths = discovery.all_paths();

    // Check if any target parent directory requires elevation
    let mut unwritable: Vec<PathBuf> = Vec::new();
    for p in &stale_paths {
        if let Some(parent) = p.parent() {
            if !is_dir_writable(parent) {
                unwritable.push(p.clone());
            }
        }
    }

    if !unwritable.is_empty() {
        if allow_uac_elevation && !config.silent_mode {
            let mut stage_guard = TempDirGuard::create(&config.temp_dir, CLEANSE_TEMP_PREFIX)?;
            spawn_elevated_updater_script(config, &config.current_version, stage_guard.path(), true)?;
            stage_guard.disarm();
            return Ok(CleanseReport {
                stale_removed: stale_paths.len(),
                stale_hot_swapped: 0,
                temp_artifacts_removed: 0,
                deferred_locked_trash: 0,
                removed_paths: stale_paths,
            });
        }
        return Err(format!(
            "Directory not writable (elevation disabled): {}",
            unwritable[0].display()
        ));
    }

    let mut report = CleanseReport::default();

    for path in stale_paths {
        match remove_or_hot_swap(&path, config)? {
            RemoveOutcome::NotFound => {}
            RemoveOutcome::Deleted => {
                report.stale_removed += 1;
                report.removed_paths.push(path);
            }
            RemoveOutcome::HotSwappedToTrash(_) => {
                report.stale_hot_swapped += 1;
                report.removed_paths.push(path);
            }
        }
    }

    let extra_roots = collect_extra_cleanup_roots(config);
    let (temp_removed, deferred_locked) = cleanup_temp_artifacts(&config.temp_dir, &extra_roots);
    report.temp_artifacts_removed = temp_removed;
    report.deferred_locked_trash = deferred_locked;

    Ok(report)
}

// ============================================================================
// ZIP Extraction & Verified Atomic Install Pipeline
// ============================================================================

fn extract_zip_archive(zip_path: &Path, extract_dir: &Path) -> Result<(), String> {
    // Validate ZIP magic bytes before calling extractor so HTML/corrupt payloads fail cleanly
    let mut f = File::open(zip_path)
        .map_err(|e| format!("Cannot open ZIP '{}': {e}", zip_path.display()))?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)
        .map_err(|_| String::from("Corrupt or truncated ZIP archive (too small)"))?;
    drop(f);
    if magic != [0x50, 0x4B, 0x03, 0x04] && magic != [0x50, 0x4B, 0x05, 0x06] {
        return Err(String::from("Invalid ZIP header (corrupt archive)"));
    }

    fs::create_dir_all(extract_dir)
        .map_err(|e| format!("Failed to create extract dir: {e}"))?;

    // Primary: Windows built-in bsdtar (fast, validates CRC/structure, blocks zip-slip)
    let tar_res = hidden_command("tar.exe")
        .args([
            "-xf",
            &zip_path.to_string_lossy(),
            "-C",
            &extract_dir.to_string_lossy(),
        ])
        .output();

    let extracted_ok = match tar_res {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(format!("Corrupt ZIP archive (extract failed): {stderr}"));
        }
        Err(_) => {
            // Fallback: PowerShell Expand-Archive
            let ps_out = hidden_command("powershell.exe")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                        zip_path.to_string_lossy().replace('\'', "''"),
                        extract_dir.to_string_lossy().replace('\'', "''")
                    ),
                ])
                .output()
                .map_err(|e| format!("ZIP extraction failed: {e}"))?;
            if !ps_out.status.success() {
                let stderr = String::from_utf8_lossy(&ps_out.stderr).trim().to_string();
                return Err(format!("Corrupt ZIP archive (Expand-Archive failed): {stderr}"));
            }
            true
        }
    };

    if !extracted_ok {
        return Err(String::from("ZIP extraction failed"));
    }
    Ok(())
}

struct ExtractedPayloads {
    vst3_dir: Option<PathBuf>,
    vst3_hash: Option<String>,
    exe_file: Option<PathBuf>,
    exe_hash: Option<String>,
}

fn find_extracted_payloads(extract_dir: &Path) -> Result<ExtractedPayloads, String> {
    let mut search_dirs = vec![extract_dir.to_path_buf()];
    if let Ok(entries) = fs::read_dir(extract_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if !name.eq_ignore_ascii_case("phantom_spire.vst3") {
                        search_dirs.push(p);
                    }
                }
            }
        }
    }

    let mut vst3_dir = None;
    let mut exe_file = None;

    for dir in &search_dirs {
        if vst3_dir.is_none() {
            let candidate = dir.join("phantom_spire.vst3");
            let inner_dll = candidate
                .join("Contents")
                .join("x86_64-win")
                .join("phantom_spire.vst3");
            if candidate.is_dir() && inner_dll.is_file() {
                if fs::metadata(&inner_dll).map(|m| m.len()).unwrap_or(0) > 0 {
                    vst3_dir = Some(candidate);
                }
            }
        }
        if exe_file.is_none() {
            let candidate = dir.join("phantom_spire.exe");
            if candidate.is_file() && fs::metadata(&candidate).map(|m| m.len()).unwrap_or(0) > 0 {
                exe_file = Some(candidate);
            }
        }
    }

    if vst3_dir.is_none() && exe_file.is_none() {
        return Err(String::from(
            "Update archive contains neither a valid phantom_spire.vst3 bundle nor phantom_spire.exe",
        ));
    }

    let vst3_hash = match &vst3_dir {
        Some(dir) => {
            let dll = dir
                .join("Contents")
                .join("x86_64-win")
                .join("phantom_spire.vst3");
            Some(sha256_file(&dll)?)
        }
        None => None,
    };
    let exe_hash = match &exe_file {
        Some(path) => Some(sha256_file(path)?),
        None => None,
    };

    Ok(ExtractedPayloads {
        vst3_dir,
        vst3_hash,
        exe_file,
        exe_hash,
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create dir '{}': {e}", dst.display()))?;
    let entries = fs::read_dir(src)
        .map_err(|e| format!("Failed to read dir '{}': {e}", src.display()))?;
    for entry in entries.flatten() {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let ft = entry
            .file_type()
            .map_err(|e| format!("Failed to stat '{}': {e}", src_path.display()))?;
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            // Skip any accidental legacy nested phantom_spire.vst3 bundle
            if let Some(name) = src_path.file_name().and_then(|n| n.to_str()) {
                if name.eq_ignore_ascii_case("phantom_spire.vst3") {
                    continue;
                }
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|e| {
                format!(
                    "Failed to copy '{}' -> '{}': {e}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

/// Atomically installs a new VST3 bundle into `canonical_vst3_dir` with rollback on failure
/// and NTFS move-to-trash hot-swap if the existing DLL is locked in RAM.
fn install_vst3_bundle_atomic(
    src_vst3: &Path,
    expected_dll_hash: &str,
    config: &UpdaterConfig,
) -> Result<bool, String> {
    let target = &config.canonical_vst3_dir;
    let parent = target
        .parent()
        .ok_or_else(|| String::from("Invalid canonical_vst3_dir (no parent)"))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create VST3 parent '{}': {e}", parent.display()))?;

    let staged = parent.join(format!("{STAGING_ENTRY_PREFIX}vst3.new-{}", unique_nonce()));
    if let Err(e) = copy_dir_recursive(src_vst3, &staged) {
        let _ = fs::remove_dir_all(&staged);
        return Err(e);
    }

    let staged_dll = staged
        .join("Contents")
        .join("x86_64-win")
        .join("phantom_spire.vst3");
    let staged_hash = match sha256_file(&staged_dll) {
        Ok(h) => h,
        Err(e) => {
            let _ = fs::remove_dir_all(&staged);
            return Err(e);
        }
    };
    if !staged_hash.eq_ignore_ascii_case(expected_dll_hash) {
        let _ = fs::remove_dir_all(&staged);
        return Err(String::from("Pre-swap SHA-256 verification failed for VST3 bundle"));
    }

    // Move old bundle aside (backup/trash) BEFORE placing new bundle so rollback is possible
    let mut hot_swapped = false;
    let mut backup_path: Option<PathBuf> = None;

    if target.exists() {
        let locked = is_path_locked(target);
        if locked {
            match remove_or_hot_swap(target, config) {
                Ok(RemoveOutcome::HotSwappedToTrash(p)) => {
                    hot_swapped = true;
                    backup_path = Some(p);
                }
                Ok(RemoveOutcome::Deleted) | Ok(RemoveOutcome::NotFound) => {}
                Err(e) => {
                    let _ = fs::remove_dir_all(&staged);
                    return Err(e);
                }
            }
        } else {
            let bak = parent.join(format!("{STAGING_ENTRY_PREFIX}vst3.bak-{}", unique_nonce()));
            if fs::rename(target, &bak).is_ok() {
                backup_path = Some(bak);
            } else {
                match remove_or_hot_swap(target, config) {
                    Ok(RemoveOutcome::HotSwappedToTrash(p)) => {
                        hot_swapped = true;
                        backup_path = Some(p);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let _ = fs::remove_dir_all(&staged);
                        return Err(e);
                    }
                }
            }
        }
    }

    if let Err(e) = fs::rename(&staged, target) {
        let _ = fs::remove_dir_all(&staged);
        if let Some(ref bak) = backup_path {
            let _ = fs::rename(bak, target);
        }
        return Err(format!(
            "Failed to finalize VST3 install at '{}': {e}",
            target.display()
        ));
    }

    // Post-install SHA-256 check
    let installed_dll = target
        .join("Contents")
        .join("x86_64-win")
        .join("phantom_spire.vst3");
    let final_hash = sha256_file(&installed_dll).unwrap_or_default();
    if !final_hash.eq_ignore_ascii_case(expected_dll_hash) {
        // Rollback!
        let _ = fs::remove_dir_all(target);
        if let Some(ref bak) = backup_path {
            let _ = fs::rename(bak, target);
        }
        return Err(format!(
            "Post-install SHA-256 verification failed for '{}'",
            installed_dll.display()
        ));
    }

    // Delete unlocked backup immediately (locked backups stay in trash until unlocked/cleanse)
    if let Some(ref bak) = backup_path {
        if !hot_swapped {
            let _ = fs::remove_dir_all(bak);
        }
    }

    Ok(hot_swapped)
}

/// Atomically installs a new Standalone `.exe` into `canonical_exe_path` with rollback on failure
/// and NTFS move-to-trash hot-swap if the existing executable is currently running/locked.
fn install_exe_atomic(
    src_exe: &Path,
    expected_exe_hash: &str,
    config: &UpdaterConfig,
) -> Result<bool, String> {
    let target = &config.canonical_exe_path;
    let parent = target
        .parent()
        .ok_or_else(|| String::from("Invalid canonical_exe_path (no parent)"))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create EXE parent '{}': {e}", parent.display()))?;

    let staged = parent.join(format!("{STAGING_ENTRY_PREFIX}exe.new-{}", unique_nonce()));
    if let Err(e) = fs::copy(src_exe, &staged) {
        let _ = fs::remove_file(&staged);
        return Err(format!("Failed to stage phantom_spire.exe: {e}"));
    }

    let staged_hash = match sha256_file(&staged) {
        Ok(h) => h,
        Err(e) => {
            let _ = fs::remove_file(&staged);
            return Err(e);
        }
    };
    if !staged_hash.eq_ignore_ascii_case(expected_exe_hash) {
        let _ = fs::remove_file(&staged);
        return Err(String::from("Pre-swap SHA-256 verification failed for phantom_spire.exe"));
    }

    let mut hot_swapped = false;
    let mut backup_path: Option<PathBuf> = None;

    if target.exists() {
        let locked = is_file_locked(target);
        if locked {
            match remove_or_hot_swap(target, config) {
                Ok(RemoveOutcome::HotSwappedToTrash(p)) => {
                    hot_swapped = true;
                    backup_path = Some(p);
                }
                Ok(RemoveOutcome::Deleted) | Ok(RemoveOutcome::NotFound) => {}
                Err(e) => {
                    let _ = fs::remove_file(&staged);
                    return Err(e);
                }
            }
        } else {
            let bak = parent.join(format!("{STAGING_ENTRY_PREFIX}exe.bak-{}", unique_nonce()));
            if fs::rename(target, &bak).is_ok() {
                backup_path = Some(bak);
            } else {
                match remove_or_hot_swap(target, config) {
                    Ok(RemoveOutcome::HotSwappedToTrash(p)) => {
                        hot_swapped = true;
                        backup_path = Some(p);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let _ = fs::remove_file(&staged);
                        return Err(e);
                    }
                }
            }
        }
    }

    if let Err(e) = fs::rename(&staged, target) {
        let _ = fs::remove_file(&staged);
        if let Some(ref bak) = backup_path {
            let _ = fs::rename(bak, target);
        }
        return Err(format!(
            "Failed to finalize EXE install at '{}': {e}",
            target.display()
        ));
    }

    let final_hash = sha256_file(target).unwrap_or_default();
    if !final_hash.eq_ignore_ascii_case(expected_exe_hash) {
        let _ = fs::remove_file(target);
        if let Some(ref bak) = backup_path {
            let _ = fs::rename(bak, target);
        }
        return Err(format!(
            "Post-install SHA-256 verification failed for '{}'",
            target.display()
        ));
    }

    if let Some(ref bak) = backup_path {
        if !hot_swapped {
            let _ = fs::remove_file(bak);
        }
    }

    Ok(hot_swapped)
}

fn run_update_pipeline_internal(
    config: &UpdaterConfig,
    version: &str,
    zip_url: &str,
    expected_zip_sha256: Option<&str>,
    expected_vst3_sha256: Option<&str>,
    expected_exe_sha256: Option<&str>,
    allow_uac_elevation: bool,
) -> Result<UpdateStatus, String> {
    config.validate_download_url(zip_url)?;

    if config.require_sha256 && expected_zip_sha256.is_none() {
        return Err(String::from(
            "SHA-256 checksum is required by UpdaterConfig but none was provided",
        ));
    }
    if let Some(sha) = expected_zip_sha256 {
        if !is_valid_sha256_hex(sha) {
            return Err(format!("Malformed expected ZIP sha256: '{sha}'"));
        }
    }

    let stage_guard = TempDirGuard::create(&config.temp_dir, UPDATE_TEMP_PREFIX)?;
    let zip_path = stage_guard.path().join("PhantomSpire-win64.zip");

    download_file_to(config, zip_url, &zip_path)?;

    let res = apply_update_from_zip_internal(
        config,
        version,
        &zip_path,
        expected_zip_sha256,
        expected_vst3_sha256,
        expected_exe_sha256,
        allow_uac_elevation,
    );

    // Explicitly drop our download stage_guard and sweep any temp artifacts
    drop(stage_guard);
    let extra_roots = collect_extra_cleanup_roots(config);
    let _ = cleanup_temp_artifacts(&config.temp_dir, &extra_roots);

    res
}

fn apply_update_from_zip_internal(
    config: &UpdaterConfig,
    version: &str,
    zip_path: &Path,
    expected_zip_sha256: Option<&str>,
    expected_vst3_sha256: Option<&str>,
    expected_exe_sha256: Option<&str>,
    allow_uac_elevation: bool,
) -> Result<UpdateStatus, String> {
    if !zip_path.is_file() {
        return Err(format!("Update zip does not exist: {}", zip_path.display()));
    }

    // Step 1: Verify ZIP SHA-256 BEFORE extracting or touching any installed binary
    if let Some(expected) = expected_zip_sha256 {
        if !is_valid_sha256_hex(expected) {
            return Err(format!("Malformed expected ZIP sha256: '{expected}'"));
        }
        let actual = sha256_file(zip_path)?;
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(format!(
                "SHA-256 mismatch for update ZIP (expected {}, got {actual})",
                expected.trim().to_ascii_lowercase()
            ));
        }
    }

    // Step 2: Extract into RAII-guarded temp staging dir
    let mut extract_guard = TempDirGuard::create(&config.temp_dir, UPDATE_TEMP_PREFIX)?;
    let extract_dir = extract_guard.path().join("extracted");
    extract_zip_archive(zip_path, &extract_dir)?;

    // Step 3: Validate payloads & per-binary SHA-256 hashes BEFORE modifying disk
    let payloads = find_extracted_payloads(&extract_dir)?;
    if let (Some(expected), Some(actual)) = (expected_vst3_sha256, payloads.vst3_hash.as_deref()) {
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(format!(
                "SHA-256 mismatch for phantom_spire.vst3 (expected {}, got {actual})",
                expected.trim().to_ascii_lowercase()
            ));
        }
    }
    if let (Some(expected), Some(actual)) = (expected_exe_sha256, payloads.exe_hash.as_deref()) {
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(format!(
                "SHA-256 mismatch for phantom_spire.exe (expected {}, got {actual})",
                expected.trim().to_ascii_lowercase()
            ));
        }
    }

    // Step 4: Check write permissions across canonical install targets & stale copies
    let discovery = discover_stale_copies(config);
    let mut needs_elevation = false;
    if payloads.vst3_dir.is_some() {
        if let Some(p) = config.canonical_vst3_dir.parent() {
            if !is_dir_writable(p) {
                needs_elevation = true;
            }
        }
    }
    if payloads.exe_file.is_some() {
        if let Some(p) = config.canonical_exe_path.parent() {
            if !is_dir_writable(p) {
                needs_elevation = true;
            }
        }
    }
    for stale in discovery.all_paths() {
        if let Some(p) = stale.parent() {
            if !is_dir_writable(p) {
                needs_elevation = true;
            }
        }
    }

    if needs_elevation {
        if allow_uac_elevation && !config.silent_mode {
            spawn_elevated_updater_script(config, version, &extract_dir, false)?;
            extract_guard.disarm();
            return Ok(UpdateStatus::ReadyToRestart {
                version: version.to_string(),
                message: format!("Elevated updater launched for v{version}"),
            });
        }
        return Err(String::from(
            "Target directory is not writable and UAC elevation is disabled",
        ));
    }

    // Step 5: Install BOTH canonical VST3 bundle and canonical EXE atomically
    let mut hot_swapped = false;
    if let (Some(ref src_vst3), Some(ref hash)) = (&payloads.vst3_dir, &payloads.vst3_hash) {
        if install_vst3_bundle_atomic(src_vst3, hash, config)? {
            hot_swapped = true;
        }
    }
    if let (Some(ref src_exe), Some(ref hash)) = (&payloads.exe_file, &payloads.exe_hash) {
        if install_exe_atomic(src_exe, hash, config)? {
            hot_swapped = true;
        }
    }

    // Step 6: Drop staging dir early, then run full aggressive stale + temp cleansing
    drop(extract_guard);
    let cleanse_report = run_cleanse_internal(config, false)?;
    if cleanse_report.stale_hot_swapped > 0 {
        hot_swapped = true;
    }

    let cleansed_total = cleanse_report.stale_removed + cleanse_report.stale_hot_swapped;
    let message = if hot_swapped {
        format!(
            "Updated to v{version} (hot-swapped locked binary, cleansed {cleansed_total} old copy(s)). Reload plugin or restart Standalone."
        )
    } else {
        format!(
            "Updated to v{version} (cleansed {cleansed_total} old copy(s)). Reload plugin or restart Standalone."
        )
    };

    Ok(UpdateStatus::ReadyToRestart {
        version: version.to_string(),
        message,
    })
}

// ============================================================================
// Elevated Windows PowerShell Fallback (for Protected Program Files Paths)
// ============================================================================

fn ps_escape_single_quoted(s: &str) -> String {
    s.replace('\'', "''")
}

fn spawn_elevated_updater_script(
    config: &UpdaterConfig,
    version: &str,
    src_dir: &Path,
    cleanse_only: bool,
) -> Result<(), String> {
    let script_parent = src_dir.parent().unwrap_or(src_dir);
    let script_path = script_parent.join("apply_phantom_spire_update.ps1");

    let src_str = ps_escape_single_quoted(&src_dir.to_string_lossy());
    let canon_vst3_str = ps_escape_single_quoted(&config.canonical_vst3_dir.to_string_lossy());
    let canon_exe_str = ps_escape_single_quoted(&config.canonical_exe_path.to_string_lossy());
    let temp_str = ps_escape_single_quoted(&config.temp_dir.to_string_lossy());
    let ver_str = ps_escape_single_quoted(version);
    let cleanse_flag = if cleanse_only { "$true" } else { "$false" };
    let silent_flag = if config.silent_mode { "$true" } else { "$false" };

    let roots_ps = config
        .scan_roots
        .iter()
        .map(|r| format!("'{}'", ps_escape_single_quoted(&r.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(", ");

    let ps_script = format!(
        r#"param([switch]$Elevated)
$ErrorActionPreference = 'Stop'
if (-not {silent_flag}) {{
    Add-Type -AssemblyName System.Windows.Forms
}}

try {{
    $src = '{src_str}'
    $canonicalVst3 = '{canon_vst3_str}'
    $canonicalExe  = '{canon_exe_str}'
    $tempBase = '{temp_str}'
    $ver = '{ver_str}'
    $cleanseOnly = {cleanse_flag}
    $silent = {silent_flag}
    $roots = @({roots_ps}) | Where-Object {{ $_ -and (Test-Path $_) }} | Select-Object -Unique

    if (-not $Elevated) {{
        Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass',
            '-File', ('"{{0}}"' -f $PSCommandPath), '-Elevated'
        )
        exit 0
    }}

    $trashRoot = Join-Path $tempBase ('PhantomSpireTrash-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Force -Path $trashRoot | Out-Null

    function Remove-OrMoveLocked([string]$targetPath) {{
        if (-not (Test-Path $targetPath)) {{ return $false }}
        try {{
            Remove-Item -Recurse -Force $targetPath -ErrorAction Stop
            return $true
        }} catch {{
            $srcRoot = [System.IO.Path]::GetPathRoot($targetPath)
            $tmpRoot = [System.IO.Path]::GetPathRoot($trashRoot)
            $destDir = if ($srcRoot -ieq $tmpRoot) {{ $trashRoot }} else {{
                $alt = Join-Path (Split-Path $targetPath -Parent) ('.PhantomSpireTrash-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
                New-Item -ItemType Directory -Force -Path $alt | Out-Null
                $alt
            }}
            $destName = (Split-Path $targetPath -Leaf) + '.' + [guid]::NewGuid().ToString('N').Substring(0, 6)
            Move-Item -Path $targetPath -Destination (Join-Path $destDir $destName) -Force -ErrorAction Stop
            return $true
        }}
    }}

    if (-not $cleanseOnly) {{
        $srcVst3 = Join-Path $src 'phantom_spire.vst3'
        $srcExe  = Join-Path $src 'phantom_spire.exe'
        if (Test-Path (Join-Path $srcVst3 'Contents\x86_64-win\phantom_spire.vst3')) {{
            $expectedHash = (Get-FileHash (Join-Path $srcVst3 'Contents\x86_64-win\phantom_spire.vst3')).Hash
            $canonParent = Split-Path $canonicalVst3 -Parent
            New-Item -ItemType Directory -Force -Path $canonParent | Out-Null
            if (Test-Path $canonicalVst3) {{ [void](Remove-OrMoveLocked $canonicalVst3) }}
            Copy-Item -Recurse -Force $srcVst3 $canonicalVst3
            $dstDll = Join-Path $canonicalVst3 'Contents\x86_64-win\phantom_spire.vst3'
            if ((Get-FileHash $dstDll).Hash -ne $expectedHash) {{ throw "SHA-256 failed for $dstDll" }}
        }}
        if (Test-Path $srcExe) {{
            $expectedExeHash = (Get-FileHash $srcExe).Hash
            $exeParent = Split-Path $canonicalExe -Parent
            New-Item -ItemType Directory -Force -Path $exeParent | Out-Null
            if (Test-Path $canonicalExe) {{ [void](Remove-OrMoveLocked $canonicalExe) }}
            Copy-Item -Force $srcExe $canonicalExe
            if ((Get-FileHash $canonicalExe).Hash -ne $expectedExeHash) {{ throw "SHA-256 failed for $canonicalExe" }}
        }}
    }}

    $cleansed = 0
    foreach ($root in $roots) {{
        Get-ChildItem -Path $root -Filter 'phantom_spire.vst3' -Directory -Recurse -Depth 5 -ErrorAction SilentlyContinue | ForEach-Object {{
            if (-not $_.FullName.Equals($canonicalVst3, [StringComparison]::OrdinalIgnoreCase)) {{
                if (Remove-OrMoveLocked $_.FullName) {{ $cleansed++ }}
            }} else {{
                $nested = Join-Path $_.FullName 'phantom_spire.vst3'
                if (Test-Path $nested) {{ if (Remove-OrMoveLocked $nested) {{ $cleansed++ }} }}
            }}
        }}
        Get-ChildItem -Path $root -Filter 'phantom_spire.vst3.bak' -Recurse -Depth 5 -ErrorAction SilentlyContinue | ForEach-Object {{
            if (Remove-OrMoveLocked $_.FullName) {{ $cleansed++ }}
        }}
        Get-ChildItem -Path $root -Filter 'phantom_spire.clap' -Recurse -Depth 5 -ErrorAction SilentlyContinue | ForEach-Object {{
            if (Remove-OrMoveLocked $_.FullName) {{ $cleansed++ }}
        }}
        Get-ChildItem -Path $root -Filter 'phantom_spire.exe' -File -Recurse -Depth 5 -ErrorAction SilentlyContinue | ForEach-Object {{
            if (-not $_.FullName.Equals($canonicalExe, [StringComparison]::OrdinalIgnoreCase)) {{
                if (Remove-OrMoveLocked $_.FullName) {{ $cleansed++ }}
            }}
        }}
    }}

    # Clean up all temporary staging/cleanse/trash artifacts
    Get-ChildItem -Path $tempBase -Directory -ErrorAction SilentlyContinue | Where-Object {{
        $_.Name -like 'PhantomSpireUpdate-*' -or $_.Name -like 'PhantomSpireCleanse-*' -or $_.Name -like 'PhantomSpireTrash-*'
    }} | ForEach-Object {{
        if (-not $_.FullName.Equals((Split-Path $PSCommandPath -Parent), [StringComparison]::OrdinalIgnoreCase)) {{
            Remove-Item -Recurse -Force $_.FullName -ErrorAction SilentlyContinue
        }}
    }}

    if (-not $silent) {{
        $headline = if ($cleanseOnly) {{ "Phantom Spire Cleanse Complete!" }} else {{ "Phantom Spire Updated to v$ver!" }}
        [System.Windows.Forms.MessageBox]::Show(
            "$headline`r`n`r`nVST3: $canonicalVst3`r`nStandalone: $canonicalExe`r`nCleansed stale copies: $cleansed",
            "Phantom Spire Updater",
            [System.Windows.Forms.MessageBoxButtons]::OK,
            [System.Windows.Forms.MessageBoxIcon]::Information
        ) | Out-Null
    }}

    # Schedule self-removal of our own script staging folder
    $selfDir = Split-Path $PSCommandPath -Parent
    Start-Process cmd.exe -WindowStyle Hidden -ArgumentList "/c ping -n 2 127.0.0.1 >nul & rmdir /s /q `"$selfDir`""
}} catch {{
    if (-not {silent_flag}) {{
        [System.Windows.Forms.MessageBox]::Show(
            "Phantom Spire updater failed:`n$($_.Exception.Message)",
            "Phantom Spire Update Error",
            [System.Windows.Forms.MessageBoxButtons]::OK,
            [System.Windows.Forms.MessageBoxIcon]::Error
        ) | Out-Null
    }}
    exit 1
}}
"#
    );

    let mut script_bytes = Vec::with_capacity(3 + ps_script.len());
    script_bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    script_bytes.extend_from_slice(ps_script.as_bytes());
    fs::write(&script_path, script_bytes)
        .map_err(|e| format!("Failed to write elevated updater script: {e}"))?;

    let mut cmd = hidden_command("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-WindowStyle",
        "Hidden",
        "-File",
        &script_path.to_string_lossy(),
    ]);
    cmd.spawn()
        .map_err(|e| format!("Failed to launch elevated updater: {e}"))?;
    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_handle_does_not_spawn_or_start_checking() {
        let handle = UpdaterHandle::default();
        assert_eq!(handle.status(), UpdateStatus::Idle);
    }

    #[test]
    fn env_overrides_and_disable_guard_work() {
        let _guard = ENV_LOCK.lock().unwrap();

        let keys = [
            "PHANTOM_SPIRE_DISABLE_AUTO_UPDATE",
            "PHANTOM_SPIRE_MANIFEST_URL",
            "PHANTOM_SPIRE_TRUSTED_PREFIX",
            "PHANTOM_SPIRE_SCAN_ROOTS",
            "PHANTOM_SPIRE_CANONICAL_VST3",
            "PHANTOM_SPIRE_CANONICAL_EXE",
            "PHANTOM_SPIRE_UPDATER_SILENT",
            "PHANTOM_SPIRE_UPDATER_CONFIG",
        ];
        let prev: Vec<Option<String>> = keys.iter().map(|k| std::env::var(k).ok()).collect();

        std::env::remove_var("PHANTOM_SPIRE_UPDATER_CONFIG");
        std::env::set_var("PHANTOM_SPIRE_DISABLE_AUTO_UPDATE", "1");
        std::env::set_var(
            "PHANTOM_SPIRE_MANIFEST_URL",
            "http://127.0.0.1:9999/latest.json",
        );
        std::env::set_var("PHANTOM_SPIRE_TRUSTED_PREFIX", "http://127.0.0.1:9999/");
        std::env::set_var("PHANTOM_SPIRE_SCAN_ROOTS", "C:\\Temp\\RootA;C:\\Temp\\RootB");
        std::env::set_var(
            "PHANTOM_SPIRE_CANONICAL_VST3",
            "C:\\Temp\\Canon\\phantom_spire.vst3",
        );
        std::env::set_var(
            "PHANTOM_SPIRE_CANONICAL_EXE",
            "C:\\Temp\\Canon\\phantom_spire.exe",
        );
        std::env::set_var("PHANTOM_SPIRE_UPDATER_SILENT", "true");

        let cfg = UpdaterConfig::from_env();
        assert!(cfg.auto_update_disabled);
        assert_eq!(cfg.manifest_url, "http://127.0.0.1:9999/latest.json");
        assert_eq!(cfg.trusted_url_prefix, "http://127.0.0.1:9999/");
        assert_eq!(cfg.scan_roots.len(), 2);
        assert_eq!(
            cfg.canonical_vst3_dir,
            PathBuf::from("C:\\Temp\\Canon\\phantom_spire.vst3")
        );
        assert_eq!(
            cfg.canonical_exe_path,
            PathBuf::from("C:\\Temp\\Canon\\phantom_spire.exe")
        );
        assert!(cfg.silent_mode);

        let handle = UpdaterHandle::new(cfg);
        assert!(handle.is_auto_update_disabled());
        handle.trigger_startup_check_if_enabled();
        assert_eq!(handle.status(), UpdateStatus::Idle);

        for (k, v) in keys.iter().zip(prev.into_iter()) {
            match v {
                Some(old) => std::env::set_var(k, old),
                None => std::env::remove_var(k),
            }
        }
    }

    #[test]
    fn config_file_overrides_and_env_precedence_work() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("ps_cfg_test_{}", unique_nonce()));
        fs::create_dir_all(&tmp).unwrap();
        let cfg_file = tmp.join("updater.json");

        // Include UTF-8 BOM to verify BOM tolerance
        let mut content = vec![0xEF, 0xBB, 0xBF];
        content.extend_from_slice(
            br#"{
                "manifest_url": "http://127.0.0.1:1111/latest.json",
                "trusted_url_prefix": "http://127.0.0.1:1111/",
                "http_timeout_secs": 4,
                "silent_mode": true,
                "require_sha256": true
            }"#,
        );
        fs::write(&cfg_file, content).unwrap();

        let cfg = UpdaterConfig::from_file(&cfg_file).unwrap();
        assert_eq!(cfg.manifest_url, "http://127.0.0.1:1111/latest.json");
        assert_eq!(cfg.trusted_url_prefix, "http://127.0.0.1:1111/");
        assert_eq!(cfg.http_timeout_secs, 4);
        assert!(cfg.silent_mode);
        assert!(cfg.require_sha256);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn semver_comparison_handles_prefixes_build_and_prerelease_strictly() {
        assert!(is_newer_version("v0.2.0", "0.1.0"));
        assert!(is_newer_version("0.1.1+build.4", "0.1.0"));
        assert!(is_newer_version("0.2.0", "0.2.0-rc.1"));
        assert!(!is_newer_version("0.2.0-rc.1", "0.2.0"));
        assert!(is_newer_version("0.2.0-rc.2", "0.2.0-rc.1"));
        assert!(is_newer_version("0.2.0-rc.10", "0.2.0-rc.2"));
        assert!(is_newer_version("0.2.0-beta.1", "0.2.0-alpha.9"));
        assert!(is_newer_version("0.2.0-alpha.beta", "0.2.0-alpha.1"));
        assert!(is_newer_version("0.2.0-alpha.1", "0.2.0-alpha"));
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        assert!(!is_newer_version("0.1.0+build.99", "0.1.0"));
        assert!(!is_newer_version("0.0.9", "0.1.0"));
        assert!(!is_newer_version("not-a-version", "0.1.0"));
        assert!(!is_newer_version("0.2.0-", "0.1.0"));
    }

    #[test]
    fn sha256_matches_nist_fips_180_4_test_vectors() {
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_bytes(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million_a = vec![b'a'; 1_000_000];
        assert_eq!(
            sha256_bytes(&million_a),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn current_module_path_returns_existing_file() {
        let p = current_module_path().expect("GetModuleHandleExW should locate current module");
        assert!(p.exists(), "module path should exist: {}", p.display());
    }
}
