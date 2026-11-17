use phantom_spire::updater::{
    check_with_config, cleanse_with_config, count_stale_copies, is_newer_version,
    run_update_pipeline, sha256_bytes, sha256_file, LatestManifest, UpdateStatus, UpdaterConfig,
    UpdaterHandle,
};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ============================================================================
// Pure-Rust Minimal Store ZIP Builder (CRC-32 + PKZIP Local/Central Headers)
// ============================================================================

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn build_zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();

    for &(name, data) in entries {
        let name_bytes = name.as_bytes();
        let offset = out.len() as u32;
        let crc = crc32(data);
        let len = data.len() as u32;

        // Local file header
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: store
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        // Central directory header
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method: store
        central.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central.extend_from_slice(&0u16.to_le_bytes()); // mod date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra len
        central.extend_from_slice(&0u16.to_le_bytes()); // comment len
        central.extend_from_slice(&0u16.to_le_bytes()); // disk start
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }

    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);

    // End of central directory record
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // disk number
    out.extend_from_slice(&0u16.to_le_bytes()); // central disk
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len

    out
}

// ============================================================================
// In-Process Mock GitHub Releases HTTP Server
// ============================================================================

#[derive(Clone)]
struct MockResponse {
    status_code: u16,
    reason: String,
    content_type: String,
    body: Vec<u8>,
    delay: Duration,
}

impl MockResponse {
    fn ok_json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status_code: 200,
            reason: "OK".to_string(),
            content_type: "application/json".to_string(),
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    fn ok_bytes(body: Vec<u8>) -> Self {
        Self {
            status_code: 200,
            reason: "OK".to_string(),
            content_type: "application/zip".to_string(),
            body,
            delay: Duration::ZERO,
        }
    }

    fn status(code: u16, reason: &str, body: &str) -> Self {
        Self {
            status_code: code,
            reason: reason.to_string(),
            content_type: "text/plain".to_string(),
            body: body.as_bytes().to_vec(),
            delay: Duration::ZERO,
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

struct MockGitHubServer {
    base_url: String,
    routes: Arc<Mutex<HashMap<String, MockResponse>>>,
    seen_requests: Arc<Mutex<Vec<String>>>,
    request_count: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockGitHubServer {
    fn start(initial_routes: HashMap<String, MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral mock port");
        let port = listener.local_addr().unwrap().port();
        listener
            .set_nonblocking(true)
            .expect("set nonblocking mock listener");

        let routes = Arc::new(Mutex::new(initial_routes));
        let seen_requests = Arc::new(Mutex::new(Vec::new()));
        let request_count = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        let routes_bg = routes.clone();
        let seen_bg = seen_requests.clone();
        let count_bg = request_count.clone();
        let stop_bg = stop.clone();

        let handle = thread::spawn(move || {
            while !stop_bg.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let routes_req = routes_bg.clone();
                        let seen_req = seen_bg.clone();
                        let count_req = count_bg.clone();
                        thread::spawn(move || {
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let mut buf = [0u8; 4096];
                            let n = stream.read(&mut buf).unwrap_or(0);
                            if n == 0 {
                                return;
                            }
                            let req_text = String::from_utf8_lossy(&buf[..n]).into_owned();
                            count_req.fetch_add(1, Ordering::SeqCst);
                            if let Ok(mut g) = seen_req.lock() {
                                g.push(req_text.clone());
                            }

                            let first_line = req_text.lines().next().unwrap_or("");
                            let raw_target = first_line.split_whitespace().nth(1).unwrap_or("/");
                            let path_only = raw_target.split('?').next().unwrap_or("/");

                            let resp = routes_req
                                .lock()
                                .ok()
                                .and_then(|map| map.get(path_only).cloned())
                                .unwrap_or_else(|| {
                                    MockResponse::status(404, "Not Found", "Not Found")
                                });

                            if !resp.delay.is_zero() {
                                thread::sleep(resp.delay);
                            }

                            let header = format!(
                                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                resp.status_code,
                                resp.reason,
                                resp.content_type,
                                resp.body.len()
                            );
                            let _ = stream.write_all(header.as_bytes());
                            let _ = stream.write_all(&resp.body);
                            let _ = stream.flush();
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            routes,
            seen_requests,
            request_count,
            stop,
            handle: Some(handle),
        }
    }

    fn set_route(&self, path: &str, resp: MockResponse) {
        self.routes.lock().unwrap().insert(path.to_string(), resp);
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn requests(&self) -> Vec<String> {
        self.seen_requests.lock().unwrap().clone()
    }

    fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }
}

impl Drop for MockGitHubServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ============================================================================
// Isolated Filesystem Test Sandbox
// ============================================================================

static SANDBOX_SEQ: AtomicUsize = AtomicUsize::new(1);

struct TestSandbox {
    root: PathBuf,
    canon_vst3_dir: PathBuf,
    canon_exe_path: PathBuf,
    scan_root_a: PathBuf,
    scan_root_b: PathBuf,
    temp_dir: PathBuf,
}

impl TestSandbox {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SANDBOX_SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ps_updater_it_{name}_{}_{}_{}",
            std::process::id(),
            nanos,
            seq
        ));
        let canon_vst3_dir = root.join("canon_vst3").join("phantom_spire.vst3");
        let canon_exe_path = root.join("canon_exe").join("phantom_spire.exe");
        let scan_root_a = root.join("scan_a");
        let scan_root_b = root.join("scan_b");
        let temp_dir = root.join("temp");

        fs::create_dir_all(canon_vst3_dir.parent().unwrap()).unwrap();
        fs::create_dir_all(canon_exe_path.parent().unwrap()).unwrap();
        fs::create_dir_all(&scan_root_a).unwrap();
        fs::create_dir_all(&scan_root_b).unwrap();
        fs::create_dir_all(&temp_dir).unwrap();

        Self {
            root,
            canon_vst3_dir,
            canon_exe_path,
            scan_root_a,
            scan_root_b,
            temp_dir,
        }
    }

    fn config(&self, base_url: &str, current_version: &str) -> UpdaterConfig {
        UpdaterConfig {
            current_version: current_version.to_string(),
            manifest_url: format!("{base_url}/latest.json"),
            trusted_url_prefix: format!("{base_url}/"),
            scan_roots: vec![
                self.canon_vst3_dir.parent().unwrap().to_path_buf(),
                self.canon_exe_path.parent().unwrap().to_path_buf(),
                self.scan_root_a.clone(),
                self.scan_root_b.clone(),
            ],
            canonical_vst3_dir: self.canon_vst3_dir.clone(),
            canonical_exe_path: self.canon_exe_path.clone(),
            temp_dir: self.temp_dir.clone(),
            silent_mode: true,
            auto_update_disabled: false,
            http_timeout_secs: 5,
            require_sha256: false,
        }
    }

    fn install_dummy_canonical(&self, vst3_bytes: &[u8], exe_bytes: &[u8]) {
        let dll = self
            .canon_vst3_dir
            .join("Contents")
            .join("x86_64-win")
            .join("phantom_spire.vst3");
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, vst3_bytes).unwrap();
        fs::write(&self.canon_exe_path, exe_bytes).unwrap();
    }

    fn canonical_vst3_dll(&self) -> PathBuf {
        self.canon_vst3_dir
            .join("Contents")
            .join("x86_64-win")
            .join("phantom_spire.vst3")
    }

    fn managed_temp_entries(&self) -> Vec<String> {
        fs::read_dir(&self.temp_dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| {
                let l = n.to_ascii_lowercase();
                l.starts_with("phantomspireupdate-")
                    || l.starts_with("phantomspirecleanse-")
                    || l.starts_with("phantomspiretrash-")
            })
            .collect()
    }
}

impl Drop for TestSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn make_release_zip(vst3_payload: &[u8], exe_payload: &[u8]) -> (Vec<u8>, String, String, String) {
    let zip_bytes = build_zip_bytes(&[
        (
            "phantom_spire.vst3/Contents/x86_64-win/phantom_spire.vst3",
            vst3_payload,
        ),
        ("phantom_spire.exe", exe_payload),
    ]);
    let zip_sha = sha256_bytes(&zip_bytes);
    let vst3_sha = sha256_bytes(vst3_payload);
    let exe_sha = sha256_bytes(exe_payload);
    (zip_bytes, zip_sha, vst3_sha, exe_sha)
}

fn wait_until<F>(timeout: Duration, mut pred: F) -> bool
where
    F: FnMut() -> bool,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        if pred() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    pred()
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_manifest_check_newer_version_cache_bust_and_utf8_bom() {
    let sb = TestSandbox::new("bom_check");
    let server = MockGitHubServer::start(HashMap::new());
    let zip_url = server.url("/releases/download/v0.2.0/PhantomSpire-win64.zip");

    let mut bom_json = vec![0xEF, 0xBB, 0xBF];
    bom_json.extend_from_slice(
        serde_json::to_vec(&LatestManifest {
            version: "0.2.0".to_string(),
            zip_url: zip_url.clone(),
            sha256: Some("a".repeat(64)),
            vst3_sha256: None,
            exe_sha256: None,
            published_at: None,
            release_notes: None,
        })
        .unwrap()
        .as_slice(),
    );
    server.set_route("/latest.json", MockResponse::ok_json(bom_json));

    let cfg = sb.config(&server.base_url, "0.1.0");
    let status = check_with_config(&cfg);
    assert_eq!(
        status,
        UpdateStatus::Available {
            version: "0.2.0".to_string(),
            zip_url,
            stale_copies: 0,
        }
    );

    let reqs = server.requests();
    assert_eq!(reqs.len(), 1);
    assert!(
        reqs[0].contains("GET /latest.json?t="),
        "Expected cache-busted query string, got: {}",
        reqs[0]
    );
    assert!(
        reqs[0].to_ascii_lowercase().contains("cache-control: no-cache"),
        "Expected Cache-Control: no-cache header, got: {}",
        reqs[0]
    );
}

#[test]
fn test_version_comparison_newer_equal_older_and_prerelease() {
    let sb = TestSandbox::new("semver");
    let server = MockGitHubServer::start(HashMap::new());
    let zip_url = server.url("/releases/download/v0.2.0/PhantomSpire-win64.zip");

    let cases = [
        ("0.1.0", "0.2.0", true),
        ("0.1.0", "0.1.0", false),
        ("0.2.0", "0.1.9", false),
        ("0.2.0", "0.2.0-rc.1", false),
        ("0.2.0-rc.1", "0.2.0", true),
        ("0.2.0-rc.2", "0.2.0-rc.10", true),
        ("0.2.0-alpha.1", "0.2.0-alpha.beta", true),
        ("0.1.0", "0.1.0+build.42", false),
        ("0.1.0", "v0.1.1", true),
    ];

    for (current, remote, expect_available) in cases {
        assert_eq!(is_newer_version(remote, current), expect_available);
        let body = serde_json::to_vec(&LatestManifest {
            version: remote.to_string(),
            zip_url: zip_url.clone(),
            sha256: None,
            vst3_sha256: None,
            exe_sha256: None,
            published_at: None,
            release_notes: None,
        })
        .unwrap();
        server.set_route("/latest.json", MockResponse::ok_json(body));

        let cfg = sb.config(&server.base_url, current);
        let st = check_with_config(&cfg);
        if expect_available {
            assert!(
                matches!(st, UpdateStatus::Available { .. }),
                "expected Available for current={current}, remote={remote}, got {st:?}"
            );
        } else {
            assert!(
                matches!(st, UpdateStatus::UpToDate { .. }),
                "expected UpToDate for current={current}, remote={remote}, got {st:?}"
            );
        }
    }
}

#[test]
fn test_full_download_sha256_verification_and_dual_vst3_exe_install() {
    let sb = TestSandbox::new("dual_install");
    sb.install_dummy_canonical(b"OLD_VST3_V010", b"OLD_EXE_V010");

    // Also plant a stale copy to ensure update automatically cleanses it
    let stale_vst3 = sb.scan_root_a.join("phantom_spire.vst3");
    fs::create_dir_all(stale_vst3.join("Contents").join("x86_64-win")).unwrap();
    fs::write(
        stale_vst3
            .join("Contents")
            .join("x86_64-win")
            .join("phantom_spire.vst3"),
        b"STALE_VST3",
    )
    .unwrap();

    let new_vst3 = b"NEW_VST3_BINARY_PAYLOAD_V020_VERIFIED";
    let new_exe = b"NEW_STANDALONE_EXE_PAYLOAD_V020_VERIFIED";
    let (zip_bytes, zip_sha, vst3_sha, exe_sha) = make_release_zip(new_vst3, new_exe);

    let server = MockGitHubServer::start(HashMap::new());
    let zip_path_route = "/releases/download/v0.2.0/PhantomSpire-win64.zip";
    let zip_url = server.url(zip_path_route);

    let manifest = LatestManifest {
        version: "0.2.0".to_string(),
        zip_url: zip_url.clone(),
        sha256: Some(zip_sha),
        vst3_sha256: Some(vst3_sha.clone()),
        exe_sha256: Some(exe_sha.clone()),
        published_at: Some("2026-11-17T00:00:00Z".to_string()),
        release_notes: Some("Release 0.2.0".to_string()),
    };
    server.set_route(
        "/latest.json",
        MockResponse::ok_json(serde_json::to_vec(&manifest).unwrap()),
    );
    server.set_route(zip_path_route, MockResponse::ok_bytes(zip_bytes));

    let mut cfg = sb.config(&server.base_url, "0.1.0");
    cfg.require_sha256 = true;

    let handle = UpdaterHandle::new(cfg.clone());
    assert_eq!(handle.status(), UpdateStatus::Idle);

    // Non-blocking async check
    handle.check_async();
    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            handle.status(),
            UpdateStatus::Available { .. }
        )),
        "expected Available, got {:?}",
        handle.status()
    );
    assert_eq!(
        handle.status(),
        UpdateStatus::Available {
            version: "0.2.0".to_string(),
            zip_url: zip_url.clone(),
            stale_copies: 1,
        }
    );

    // Non-blocking async install (uses cached manifest SHA-256 automatically)
    handle.trigger_update_from_available_async();
    assert!(
        wait_until(Duration::from_secs(10), || matches!(
            handle.status(),
            UpdateStatus::ReadyToRestart { .. } | UpdateStatus::Error(_)
        )),
        "timed out waiting for install, status={:?}",
        handle.status()
    );
    match handle.status() {
        UpdateStatus::ReadyToRestart { version, .. } => assert_eq!(version, "0.2.0"),
        other => panic!("Expected ReadyToRestart, got {other:?}"),
    }

    // Verify BOTH canonical VST3 and canonical EXE were updated and match exact SHA-256
    assert_eq!(fs::read(sb.canonical_vst3_dll()).unwrap(), new_vst3);
    assert_eq!(sha256_file(&sb.canonical_vst3_dll()).unwrap(), vst3_sha);
    assert_eq!(fs::read(&sb.canon_exe_path).unwrap(), new_exe);
    assert_eq!(sha256_file(&sb.canon_exe_path).unwrap(), exe_sha);

    // Verify stale copy was cleansed
    assert!(!stale_vst3.exists(), "stale VST3 should be removed");
    assert_eq!(count_stale_copies(&cfg), 0);

    // Verify ZERO temp staging/cleanse/trash artifacts left behind
    assert!(
        sb.managed_temp_entries().is_empty(),
        "Expected zero leftover temp dirs, found: {:?}",
        sb.managed_temp_entries()
    );
}

#[cfg(target_os = "windows")]
#[test]
fn test_locked_file_ntfs_hot_swap_for_vst3_and_exe() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;

    let sb = TestSandbox::new("hotswap");
    sb.install_dummy_canonical(b"LOCKED_OLD_VST3_V010", b"LOCKED_OLD_EXE_V010");

    let new_vst3 = b"HOTSWAPPED_NEW_VST3_V030";
    let new_exe = b"HOTSWAPPED_NEW_EXE_V030";
    let (zip_bytes, zip_sha, vst3_sha, exe_sha) = make_release_zip(new_vst3, new_exe);

    let server = MockGitHubServer::start(HashMap::new());
    let zip_route = "/releases/download/v0.3.0/PhantomSpire-win64.zip";
    server.set_route(zip_route, MockResponse::ok_bytes(zip_bytes));

    let manifest = LatestManifest {
        version: "0.3.0".to_string(),
        zip_url: server.url(zip_route),
        sha256: Some(zip_sha),
        vst3_sha256: Some(vst3_sha),
        exe_sha256: Some(exe_sha),
        published_at: None,
        release_notes: None,
    };

    let cfg = sb.config(&server.base_url, "0.1.0");

    // Simulate DAW holding loaded VST3 DLL and running Standalone EXE in memory
    let locked_vst3_handle = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .open(sb.canonical_vst3_dll())
        .expect("open locked VST3 handle");
    let locked_exe_handle = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .open(&sb.canon_exe_path)
        .expect("open locked EXE handle");

    let result = UpdaterHandle::run_update_from_manifest(&cfg, &manifest)
        .expect("hot-swap update must succeed while old binaries are locked");

    match result {
        UpdateStatus::ReadyToRestart { version, message } => {
            assert_eq!(version, "0.3.0");
            assert!(
                message.to_ascii_lowercase().contains("hot-swapped"),
                "expected hot-swapped mention in message: {message}"
            );
        }
        other => panic!("unexpected status: {other:?}"),
    }

    // Even WHILE old handles are still open, canonical paths contain the brand new binaries!
    assert_eq!(fs::read(sb.canonical_vst3_dll()).unwrap(), new_vst3);
    assert_eq!(fs::read(&sb.canon_exe_path).unwrap(), new_exe);

    // Release simulated DAW/process handles, then run cleanse -> ALL trash dirs disappear!
    drop(locked_vst3_handle);
    drop(locked_exe_handle);

    let report = cleanse_with_config(&cfg).expect("post-unlock cleanse should succeed");
    assert_eq!(report.deferred_locked_trash, 0);
    assert!(
        sb.managed_temp_entries().is_empty(),
        "Expected all trash dirs removed after handles released, found: {:?}",
        sb.managed_temp_entries()
    );
}

#[test]
fn test_aggressive_cleansing_of_stale_binaries_nested_bundles_and_temp_artifacts() {
    let sb = TestSandbox::new("cleanse");
    sb.install_dummy_canonical(b"CANON_VST3_KEEP", b"CANON_EXE_KEEP");

    // 1. Stale VST3 bundle dir in scan_root_a
    let stale_vst3_dir = sb.scan_root_a.join("old_folder").join("phantom_spire.vst3");
    fs::create_dir_all(stale_vst3_dir.join("Contents").join("x86_64-win")).unwrap();
    fs::write(
        stale_vst3_dir
            .join("Contents")
            .join("x86_64-win")
            .join("phantom_spire.vst3"),
        b"STALE_BUNDLE",
    )
    .unwrap();

    // 2. Stale .vst3.bak directory
    let stale_bak = sb.scan_root_a.join("phantom_spire.vst3.bak");
    fs::create_dir_all(&stale_bak).unwrap();
    fs::write(stale_bak.join("old.bin"), b"BAK").unwrap();

    // 3. Stale .clap file
    let stale_clap = sb.scan_root_a.join("CLAP").join("phantom_spire.clap");
    fs::create_dir_all(stale_clap.parent().unwrap()).unwrap();
    fs::write(&stale_clap, b"OLD_CLAP").unwrap();

    // 4. Stale standalone .exe in Downloads-like root
    let stale_exe = sb.scan_root_b.join("Downloads").join("phantom_spire.exe");
    fs::create_dir_all(stale_exe.parent().unwrap()).unwrap();
    fs::write(&stale_exe, b"OLD_EXE").unwrap();

    // 5. Stray single-file phantom_spire.vst3 outside canonical bundle
    let stray_vst3_file = sb.scan_root_b.join("stray").join("phantom_spire.vst3");
    fs::create_dir_all(stray_vst3_file.parent().unwrap()).unwrap();
    fs::write(&stray_vst3_file, b"STRAY_FILE").unwrap();

    // 6. Legacy nested bundle inside canonical_vst3_dir/phantom_spire.vst3
    let nested_bundle = sb.canon_vst3_dir.join("phantom_spire.vst3");
    fs::create_dir_all(&nested_bundle).unwrap();
    fs::write(nested_bundle.join("legacy.txt"), b"NESTED").unwrap();

    // 7. Temporary update/cleanse/trash dirs + an unrelated dir that MUST survive
    let tmp_update = sb.temp_dir.join("PhantomSpireUpdate-111-222");
    let tmp_cleanse = sb.temp_dir.join("PhantomSpireCleanse-333-444");
    let tmp_trash = sb.temp_dir.join("PhantomSpireTrash-555-666");
    let unrelated_tmp = sb.temp_dir.join("UnrelatedAppTemp-999");
    for d in [&tmp_update, &tmp_cleanse, &tmp_trash, &unrelated_tmp] {
        fs::create_dir_all(d).unwrap();
        fs::write(d.join("file.tmp"), b"temp").unwrap();
    }

    let cfg = sb.config("http://127.0.0.1:1", "0.1.0");
    assert_eq!(count_stale_copies(&cfg), 6);

    let report = cleanse_with_config(&cfg).expect("cleanse should succeed");
    assert_eq!(report.stale_removed, 6);
    assert_eq!(report.temp_artifacts_removed, 3);
    assert_eq!(report.deferred_locked_trash, 0);

    // All 6 stale copies & all 3 temp dirs are gone
    assert!(!stale_vst3_dir.exists());
    assert!(!stale_bak.exists());
    assert!(!stale_clap.exists());
    assert!(!stale_exe.exists());
    assert!(!stray_vst3_file.exists());
    assert!(!nested_bundle.exists());
    assert!(!tmp_update.exists());
    assert!(!tmp_cleanse.exists());
    assert!(!tmp_trash.exists());
    assert!(sb.managed_temp_entries().is_empty());

    // Canonical binaries & unrelated temp dir are 100% intact
    assert_eq!(fs::read(sb.canonical_vst3_dll()).unwrap(), b"CANON_VST3_KEEP");
    assert_eq!(fs::read(&sb.canon_exe_path).unwrap(), b"CANON_EXE_KEEP");
    assert!(unrelated_tmp.exists());
    assert_eq!(count_stale_copies(&cfg), 0);
}

#[test]
fn test_rollback_and_intact_binaries_on_corrupt_zip_and_sha256_mismatch() {
    let sb = TestSandbox::new("rollback");
    sb.install_dummy_canonical(b"ORIGINAL_VST3_SAFE", b"ORIGINAL_EXE_SAFE");

    let (valid_zip, _good_zip_sha, _good_vst3_sha, good_exe_sha) =
        make_release_zip(b"NEW_VST3", b"NEW_EXE");
    let empty_payload_zip = build_zip_bytes(&[("README.txt", b"no binaries here")]);
    let corrupt_zip = b"NOT A REAL ZIP FILE CORRUPTED PAYLOAD 1234567890".to_vec();

    let server = MockGitHubServer::start(HashMap::new());
    server.set_route("/good.zip", MockResponse::ok_bytes(valid_zip));
    server.set_route("/corrupt.zip", MockResponse::ok_bytes(corrupt_zip));
    server.set_route("/empty.zip", MockResponse::ok_bytes(empty_payload_zip));

    let cfg = sb.config(&server.base_url, "0.1.0");

    // 1. Corrupt ZIP SHA-256 mismatch
    let bad_zip_manifest = LatestManifest {
        version: "0.2.0".to_string(),
        zip_url: server.url("/good.zip"),
        sha256: Some("0".repeat(64)),
        vst3_sha256: None,
        exe_sha256: None,
        published_at: None,
        release_notes: None,
    };
    let err = UpdaterHandle::run_update_from_manifest(&cfg, &bad_zip_manifest).unwrap_err();
    assert!(
        err.to_ascii_lowercase().contains("sha-256"),
        "expected SHA-256 error, got: {err}"
    );
    assert_eq!(fs::read(sb.canonical_vst3_dll()).unwrap(), b"ORIGINAL_VST3_SAFE");
    assert_eq!(fs::read(&sb.canon_exe_path).unwrap(), b"ORIGINAL_EXE_SAFE");
    assert!(sb.managed_temp_entries().is_empty());

    // 2. Per-binary VST3 SHA-256 mismatch
    let bad_vst3_manifest = LatestManifest {
        version: "0.2.0".to_string(),
        zip_url: server.url("/good.zip"),
        sha256: None,
        vst3_sha256: Some("f".repeat(64)),
        exe_sha256: Some(good_exe_sha),
        published_at: None,
        release_notes: None,
    };
    let err = UpdaterHandle::run_update_from_manifest(&cfg, &bad_vst3_manifest).unwrap_err();
    assert!(err.to_ascii_lowercase().contains("sha-256"), "got: {err}");
    assert_eq!(fs::read(sb.canonical_vst3_dll()).unwrap(), b"ORIGINAL_VST3_SAFE");
    assert_eq!(fs::read(&sb.canon_exe_path).unwrap(), b"ORIGINAL_EXE_SAFE");
    assert!(sb.managed_temp_entries().is_empty());

    // 3. Corrupt non-ZIP download
    let err = run_update_pipeline(&cfg, "0.2.0", &server.url("/corrupt.zip")).unwrap_err();
    assert!(
        err.to_ascii_lowercase().contains("zip") || err.to_ascii_lowercase().contains("corrupt"),
        "expected corrupt ZIP error, got: {err}"
    );
    assert_eq!(fs::read(sb.canonical_vst3_dll()).unwrap(), b"ORIGINAL_VST3_SAFE");
    assert_eq!(fs::read(&sb.canon_exe_path).unwrap(), b"ORIGINAL_EXE_SAFE");
    assert!(sb.managed_temp_entries().is_empty());

    // 4. Valid ZIP missing plugin payloads
    let err = run_update_pipeline(&cfg, "0.2.0", &server.url("/empty.zip")).unwrap_err();
    assert!(err.contains("phantom_spire.vst3"), "got: {err}");
    assert_eq!(fs::read(sb.canonical_vst3_dll()).unwrap(), b"ORIGINAL_VST3_SAFE");
    assert_eq!(fs::read(&sb.canon_exe_path).unwrap(), b"ORIGINAL_EXE_SAFE");
    assert!(sb.managed_temp_entries().is_empty());
}

#[test]
fn test_failure_modes_offline_404_500_malformed_untrusted_and_slow_timeout() {
    let sb = TestSandbox::new("failures");

    // 1. Unreachable localhost port (bind & drop immediately)
    let dead_port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut dead_cfg = sb.config(&format!("http://127.0.0.1:{dead_port}"), "0.1.0");
    dead_cfg.http_timeout_secs = 2;
    let st = check_with_config(&dead_cfg);
    assert!(
        matches!(st, UpdateStatus::Error(_)),
        "expected Error on unreachable port, got {st:?}"
    );

    let server = MockGitHubServer::start(HashMap::new());
    let mut cfg = sb.config(&server.base_url, "0.1.0");
    cfg.http_timeout_secs = 2;

    // 2. HTTP 404
    server.set_route(
        "/latest.json",
        MockResponse::status(404, "Not Found", "No release yet"),
    );
    assert!(matches!(check_with_config(&cfg), UpdateStatus::Error(_)));

    // 3. HTTP 500
    server.set_route(
        "/latest.json",
        MockResponse::status(500, "Internal Server Error", "GitHub down"),
    );
    assert!(matches!(check_with_config(&cfg), UpdateStatus::Error(_)));

    // 4. Malformed JSON
    server.set_route(
        "/latest.json",
        MockResponse::ok_json(b"{ this is not valid json !!!".to_vec()),
    );
    match check_with_config(&cfg) {
        UpdateStatus::Error(msg) => assert!(msg.contains("JSON"), "got: {msg}"),
        other => panic!("expected Error on malformed JSON, got {other:?}"),
    }

    // 5. Untrusted download URL (rejected at both check AND pipeline stages)
    let untrusted_manifest = LatestManifest {
        version: "9.9.9".to_string(),
        zip_url: "https://evil.example.com/malware.zip".to_string(),
        sha256: None,
        vst3_sha256: None,
        exe_sha256: None,
        published_at: None,
        release_notes: None,
    };
    server.set_route(
        "/latest.json",
        MockResponse::ok_json(serde_json::to_vec(&untrusted_manifest).unwrap()),
    );
    match check_with_config(&cfg) {
        UpdateStatus::Error(msg) => assert!(msg.contains("Untrusted"), "got: {msg}"),
        other => panic!("expected Untrusted error in check, got {other:?}"),
    }
    let pipe_err = run_update_pipeline(&cfg, "9.9.9", "https://evil.example.com/malware.zip")
        .unwrap_err();
    assert!(pipe_err.contains("Untrusted"), "got: {pipe_err}");

    // 6. Slow / hanging server respects short timeout AND check_async is strictly non-blocking
    server.set_route(
        "/latest.json",
        MockResponse::ok_json(b"{}".to_vec()).with_delay(Duration::from_secs(8)),
    );
    cfg.http_timeout_secs = 1;

    let t0 = Instant::now();
    let handle = UpdaterHandle::new(cfg.clone());
    handle.check_async();
    let async_return_elapsed = t0.elapsed();
    assert!(
        async_return_elapsed < Duration::from_millis(250),
        "check_async must never block caller; took {async_return_elapsed:?}"
    );

    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            handle.status(),
            UpdateStatus::Error(_)
        )),
        "slow server should time out into Error within 5s, got {:?}",
        handle.status()
    );
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "slow server timed out too slowly: {:?}",
        t0.elapsed()
    );
}

#[test]
fn test_auto_update_disabled_never_contacts_network_and_cleanse_async_works() {
    let sb = TestSandbox::new("disabled_and_async_cleanse");
    sb.install_dummy_canonical(b"CANON_VST3", b"CANON_EXE");

    let stale_clap = sb.scan_root_a.join("phantom_spire.clap");
    fs::write(&stale_clap, b"OLD_CLAP").unwrap();
    let stale_tmp = sb.temp_dir.join("PhantomSpireUpdate-999-888");
    fs::create_dir_all(&stale_tmp).unwrap();

    let server = MockGitHubServer::start(HashMap::new());
    let mut cfg = sb.config(&server.base_url, "0.1.0");
    cfg.auto_update_disabled = true;

    let handle = UpdaterHandle::new(cfg.clone());
    handle.trigger_startup_check_if_enabled();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(handle.status(), UpdateStatus::Idle);
    assert_eq!(
        server.request_count(),
        0,
        "Disabled auto-update must make zero network requests"
    );

    // Async cleanse runs in background and clears both stale binary & temp dir
    handle.cleanse_async();
    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            handle.status(),
            UpdateStatus::UpToDate { stale_copies: 0 }
        )),
        "expected UpToDate {{ stale_copies: 0 }}, got {:?}",
        handle.status()
    );
    assert!(!stale_clap.exists());
    assert!(!stale_tmp.exists());
    assert_eq!(fs::read(sb.canonical_vst3_dll()).unwrap(), b"CANON_VST3");
    assert_eq!(fs::read(&sb.canon_exe_path).unwrap(), b"CANON_EXE");
}

#[test]
fn test_release_script_and_github_workflow_contracts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ps1 = fs::read_to_string(root.join("publish-release.ps1"))
        .expect("publish-release.ps1 must exist");
    let yml = fs::read_to_string(root.join(".github").join("workflows").join("release.yml"))
        .expect(".github/workflows/release.yml must exist");

    for (name, text) in [("publish-release.ps1", &ps1), ("release.yml", &yml)] {
        assert!(
            text.contains("phantom_spire.vst3"),
            "{name} must package phantom_spire.vst3"
        );
        assert!(
            text.contains("phantom_spire.exe"),
            "{name} must package phantom_spire.exe"
        );
        assert!(
            text.contains("PhantomSpire-win64.zip"),
            "{name} must produce PhantomSpire-win64.zip"
        );
        assert!(
            text.contains("latest.json"),
            "{name} must generate latest.json"
        );
        assert!(
            text.contains("SHA256") && text.contains("vst3_sha256") && text.contains("exe_sha256"),
            "{name} must compute and publish SHA-256 hashes"
        );
    }
    assert!(yml.contains("stable-x86_64-pc-windows-gnu"));
    assert!(yml.contains("softprops/action-gh-release"));
}

#[test]
fn test_apply_update_from_zip_for_test_helper() {
    let sb = TestSandbox::new("direct_zip_helper");
    sb.install_dummy_canonical(b"OLD_V", b"OLD_E");

    let (zip_bytes, _, _, _) = make_release_zip(b"DIRECT_VST3", b"DIRECT_EXE");
    let zip_file = sb.root.join("local_release.zip");
    File::create(&zip_file)
        .unwrap()
        .write_all(&zip_bytes)
        .unwrap();

    let cfg = sb.config("http://127.0.0.1:1", "0.2.0");
    let st = UpdaterHandle::apply_update_from_zip_for_test(&cfg, &zip_file)
        .expect("direct zip apply should succeed");
    assert!(matches!(st, UpdateStatus::ReadyToRestart { .. }));
    assert_eq!(fs::read(sb.canonical_vst3_dll()).unwrap(), b"DIRECT_VST3");
    assert_eq!(fs::read(&sb.canon_exe_path).unwrap(), b"DIRECT_EXE");
    assert!(sb.managed_temp_entries().is_empty());
}

#[test]
fn test_cleanse_seeds_missing_canonical_before_removing_stale() {
    let sb = TestSandbox::new("cleanse_seeds_canon");
    // Do NOT create canonical VST3/EXE first — simulate a user running from Downloads only
    let dl_vst3 = sb.scan_root_a.join("phantom_spire.vst3");
    let dl_dll = dl_vst3
        .join("Contents")
        .join("x86_64-win")
        .join("phantom_spire.vst3");
    fs::create_dir_all(dl_dll.parent().unwrap()).unwrap();
    fs::write(&dl_dll, b"SEED_VST3_BYTES").unwrap();

    let dl_exe = sb.scan_root_a.join("phantom_spire.exe");
    fs::write(&dl_exe, b"SEED_EXE_BYTES").unwrap();

    let cfg = sb.config("http://127.0.0.1:1", "0.2.0");
    let report = cleanse_with_config(&cfg).expect("cleanse should succeed and seed canonical");
    assert!(report.stale_removed >= 2);

    assert!(!dl_vst3.exists(), "Downloads VST3 copy should be cleansed");
    assert!(!dl_exe.exists(), "Downloads EXE copy should be cleansed");
    assert!(
        fs::read(sb.canonical_vst3_dll()).unwrap() == b"SEED_VST3_BYTES",
        "canonical VST3 must be seeded before removing the stale copy"
    );
    assert!(
        fs::read(&sb.canon_exe_path).unwrap() == b"SEED_EXE_BYTES",
        "canonical EXE must be seeded before removing the stale copy"
    );
    assert_eq!(count_stale_copies(&cfg), 0);
}

#[test]
fn test_check_async_sweeps_stale_trash_without_touching_active_staging() {
    let sb = TestSandbox::new("check_async_sweeps_trash");
    let leftover_trash = sb.temp_dir.join("PhantomSpireTrash-old123");
    let active_stage = sb.temp_dir.join("PhantomSpireUpdate-inflight");
    fs::create_dir_all(&leftover_trash).unwrap();
    fs::create_dir_all(&active_stage).unwrap();
    fs::write(leftover_trash.join("old.dll"), b"stale").unwrap();
    fs::write(active_stage.join("stage.zip"), b"active").unwrap();

    let mut routes = HashMap::new();
    routes.insert(
        "/latest.json".to_string(),
        MockResponse::ok_json(r#"{"version":"0.2.0","zip_url":"http://127.0.0.1:1/x.zip"}"#),
    );
    let server = MockGitHubServer::start(routes);

    let mut cfg = sb.config(&server.url("/latest.json"), "0.2.0");
    cfg.trusted_url_prefix = "http://127.0.0.1:1/".to_string();
    let handle = UpdaterHandle::new(cfg);
    handle.check_async();

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !matches!(handle.status(), UpdateStatus::Idle | UpdateStatus::Checking) {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    assert!(
        !leftover_trash.exists(),
        "check_async must sweep unlocked PhantomSpireTrash-* directories"
    );
    assert!(
        active_stage.exists(),
        "check_async must not disturb PhantomSpireUpdate-* staging directories"
    );
}
