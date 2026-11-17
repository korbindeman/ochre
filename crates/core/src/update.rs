//! Auto-update - Zinnia model (ADR-0052).
//!
//! GitHub Releases serve a Tauri-compatible `latest.json` manifest. The archive
//! is minisign-signed; the public key is compiled in. Check on launch (and on a
//! timer / menu), then the host shows an "Update & Restart" card. Download,
//! verify, stage, spawn a replace-and-relaunch helper, and quit.
//!
//! Logic lives in the core so Windows can later share the same manifest +
//! signature scheme (WinSparkle/Squirrel later; same `latest.json` shape).

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Default manifest URL (Tauri-compatible `latest.json` on GitHub Releases).
/// `/releases/latest` ignores prereleases; alpha builds also fall back to the
/// GitHub API when this 404s (see [`fetch_manifest`]).
pub const DEFAULT_ENDPOINT: &str =
    "https://github.com/korbindeman/ochre/releases/latest/download/latest.json";

/// GitHub API list used when `/releases/latest` has no stable release yet
/// (alpha/prerelease channel).
const GITHUB_RELEASES_API: &str = "https://api.github.com/repos/korbindeman/ochre/releases";

/// Minisign public key, base64 of the `.pub` file contents (same encoding as
/// Tauri's `plugins.updater.pubkey`). Private key is CI-only - never ship it.
const PUBLIC_KEY_B64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXkgQzhGNzhERjRBMDE5NURBMgpSV1NpWFJtZzlJMzN5RXJuSDFhZmhxSjVVbGthQzBaRWZDV1RLWWVvbGVhTFZTamxYWDJjeGpmUQo=";

/// Hosts allowed for archive downloads (SSRF guard).
const ALLOWED_DOWNLOAD_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// Cap on archive size (~1 GiB). CEF bundles are ~200 MB today.
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;

/// HTTP timeouts.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// Progress of the current install (bytes downloaded / total if known).
static DOWNLOAD_BYTES: AtomicU64 = AtomicU64::new(0);
static DOWNLOAD_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Last successful check result, for the host to re-read after a background check.
static LAST_AVAILABLE: Mutex<Option<AvailableUpdate>> = Mutex::new(None);

/// Tauri-compatible update manifest (`latest.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pub_date: Option<String>,
    pub platforms: HashMap<String, PlatformEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformEntry {
    pub signature: String,
    pub url: String,
}

/// An update newer than the running build, ready to install.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableUpdate {
    pub version: String,
    pub current_version: String,
    pub notes: String,
    pub pub_date: Option<String>,
    pub url: String,
    pub signature: String,
}

/// Platform key used in `latest.json` (`darwin-aarch64`, …).
pub fn platform_key() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("darwin-aarch64");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some("darwin-x86_64");
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("windows-x86_64");
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("linux-x86_64");
    }
    #[allow(unreachable_code)]
    None
}

/// Bytes downloaded / total for the in-flight install (0 if idle).
pub fn download_progress() -> (u64, u64) {
    (
        DOWNLOAD_BYTES.load(Ordering::Relaxed),
        DOWNLOAD_TOTAL.load(Ordering::Relaxed),
    )
}

/// Last update found by [`check`], if any.
pub fn last_available() -> Option<AvailableUpdate> {
    LAST_AVAILABLE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

/// Clear a dismissed / installed update from the cache.
pub fn clear_last_available() {
    *LAST_AVAILABLE.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

/// Check for a newer version. `current_version` is the running app's marketing
/// version (no leading `v`). Returns `Ok(None)` when up to date or the platform
/// has no package. Network / parse errors return `Err` (host logs and stays quiet).
pub fn check(current_version: &str) -> Result<Option<AvailableUpdate>, String> {
    let endpoint =
        std::env::var("BROWSER_UPDATE_ENDPOINT").unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
    let manifest = fetch_manifest(&endpoint)?;
    let available = select_update(&manifest, current_version)?;
    *LAST_AVAILABLE.lock().unwrap_or_else(|p| p.into_inner()) = available.clone();
    Ok(available)
}

/// Download, verify, stage, and spawn the replace-and-relaunch helper.
///
/// `app_bundle` is the path to the running `.app`. On success the helper is
/// running; the host should quit so the swap can finish.
///
/// Blocks - call off the main thread.
pub fn download_and_install(update: &AvailableUpdate, app_bundle: &Path) -> Result<(), String> {
    DOWNLOAD_BYTES.store(0, Ordering::Relaxed);
    DOWNLOAD_TOTAL.store(0, Ordering::Relaxed);

    let cache = update_cache_dir()?;
    let archive_path = cache.join(format!(
        "{}-{}.tar.gz",
        crate::product::NAME,
        sanitize_version(&update.version)
    ));
    download_file(&update.url, &archive_path)?;
    verify_signature(&archive_path, &update.signature)?;

    let stage = cache.join(format!("stage-{}", sanitize_version(&update.version)));
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| format!("clear stage: {e}"))?;
    }
    fs::create_dir_all(&stage).map_err(|e| format!("create stage: {e}"))?;
    extract_tar_gz(&archive_path, &stage)?;

    let staged_app = find_app_bundle(&stage)
        .ok_or_else(|| "update archive did not contain a .app bundle".to_string())?;

    let app_bundle = app_bundle
        .canonicalize()
        .unwrap_or_else(|_| app_bundle.to_path_buf());
    spawn_relaunch_helper(&staged_app, &app_bundle)?;

    // Keep the archive for a bit so a failed swap can be retried; drop older junk.
    let _ = fs::remove_file(&archive_path);
    clear_last_available();
    Ok(())
}

/// Pure: pick a platform package newer than `current_version`.
pub fn select_update(
    manifest: &Manifest,
    current_version: &str,
) -> Result<Option<AvailableUpdate>, String> {
    let Some(key) = platform_key() else {
        return Ok(None);
    };
    let Some(entry) = manifest.platforms.get(key) else {
        return Ok(None);
    };

    let remote = parse_version(&manifest.version)?;
    let current = parse_version(current_version)?;
    if remote <= current {
        return Ok(None);
    }

    ensure_download_url_allowed(&entry.url)?;

    Ok(Some(AvailableUpdate {
        version: strip_v(&manifest.version).to_string(),
        current_version: strip_v(current_version).to_string(),
        notes: manifest.notes.clone(),
        pub_date: manifest.pub_date.clone(),
        url: entry.url.clone(),
        signature: entry.signature.clone(),
    }))
}

/// Verify a minisign signature (base64 of the `.sig` file contents, Tauri style).
pub fn verify_bytes(data: &[u8], signature_b64: &str) -> Result<(), String> {
    let pub_bytes = base64::engine::general_purpose::STANDARD
        .decode(PUBLIC_KEY_B64)
        .map_err(|e| format!("decode public key: {e}"))?;
    let pub_str = String::from_utf8(pub_bytes).map_err(|e| format!("public key utf-8: {e}"))?;
    let pk = PublicKey::decode(&pub_str).map_err(|e| format!("public key: {e}"))?;

    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .map_err(|e| format!("decode signature: {e}"))?;
    let sig_str = String::from_utf8(sig_bytes).map_err(|e| format!("signature utf-8: {e}"))?;
    let signature = Signature::decode(&sig_str).map_err(|e| format!("signature: {e}"))?;

    pk.verify(data, &signature, false)
        .map_err(|e| format!("signature verification failed: {e}"))
}

fn verify_signature(path: &Path, signature_b64: &str) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| format!("read archive: {e}"))?;
    verify_bytes(&data, signature_b64)
}

fn fetch_manifest(endpoint: &str) -> Result<Manifest, String> {
    match http_get_string(endpoint) {
        Ok(body) => parse_manifest_json(&body),
        Err(err) => {
            // Prerelease alphas never appear under /releases/latest - fall back.
            if endpoint.contains("github.com") {
                eprintln!("[update] endpoint failed ({err}); trying GitHub API for prereleases");
                fetch_manifest_from_github_api()
            } else {
                Err(err)
            }
        }
    }
}

fn fetch_manifest_from_github_api() -> Result<Manifest, String> {
    let body = http_get_string(GITHUB_RELEASES_API)?;
    let releases: Vec<GitHubRelease> =
        serde_json::from_str(&body).map_err(|e| format!("parse GitHub releases: {e}"))?;
    for release in releases {
        if release.draft {
            continue;
        }
        if let Some(asset) = release.assets.iter().find(|a| a.name == "latest.json") {
            let json = http_get_string(&asset.browser_download_url)?;
            return parse_manifest_json(&json);
        }
    }
    Err("no published release with a latest.json asset".into())
}

fn parse_manifest_json(body: &str) -> Result<Manifest, String> {
    serde_json::from_str(body).map_err(|e| format!("parse latest.json: {e}"))
}

fn http_get_string(url: &str) -> Result<String, String> {
    let mut response = http_agent()
        .get(url)
        .header("User-Agent", "OchreBrowser-Updater")
        .header("Accept", "application/json")
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !(200..300).contains(&response.status().as_u16()) {
        return Err(format!("GET {url}: HTTP {}", response.status()));
    }
    response
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| format!("read {url}: {e}"))
}

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    ensure_download_url_allowed(url)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create cache: {e}"))?;
    }
    let partial = dest.with_extension("partial");
    let mut response = http_agent()
        .get(url)
        .header("User-Agent", "OchreBrowser-Updater")
        .call()
        .map_err(|e| format!("download: {e}"))?;
    if !(200..300).contains(&response.status().as_u16()) {
        return Err(format!("download: HTTP {}", response.status()));
    }
    let total = response.body().content_length().unwrap_or(0);
    if total > MAX_ARCHIVE_BYTES {
        return Err(format!("archive too large ({total} bytes)"));
    }
    DOWNLOAD_TOTAL.store(total, Ordering::Relaxed);

    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(&partial).map_err(|e| format!("create partial: {e}"))?;
    let mut buf = [0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download read: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("download write: {e}"))?;
        written += n as u64;
        if written > MAX_ARCHIVE_BYTES {
            let _ = fs::remove_file(&partial);
            return Err("archive exceeded size cap while downloading".into());
        }
        DOWNLOAD_BYTES.store(written, Ordering::Relaxed);
    }
    file.sync_all().map_err(|e| format!("sync archive: {e}"))?;
    drop(file);
    fs::rename(&partial, dest).map_err(|e| format!("finalize archive: {e}"))?;
    Ok(())
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into()
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| format!("open archive: {e}"))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| format!("extract archive: {e}"))?;
    Ok(())
}

fn find_app_bundle(root: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: u32) -> Option<PathBuf> {
        if depth > 4 {
            return None;
        }
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("app") && path.is_dir() {
                return Some(path);
            }
            if path.is_dir() {
                if let Some(found) = walk(&path, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(root, 0)
}

fn spawn_relaunch_helper(staged_app: &Path, install_target: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let cache = update_cache_dir()?;
        let script_path = cache.join("relaunch.sh");
        let pid = std::process::id();
        let script = format!(
            r#"#!/bin/bash
set -euo pipefail
PID="{pid}"
SRC={src}
DST={dst}
while kill -0 "$PID" 2>/dev/null; do sleep 0.2; done
sleep 0.4
# Move the live bundle aside, then put the new one in place.
BACKUP="${{DST}}.updating"
rm -rf "$BACKUP"
if [ -e "$DST" ]; then
  mv "$DST" "$BACKUP"
fi
mv "$SRC" "$DST"
rm -rf "$BACKUP"
# Unsigned alpha builds: clear quarantine so Gatekeeper does not block relaunch.
xattr -cr "$DST" 2>/dev/null || true
open "$DST"
"#,
            pid = pid,
            src = shell_quote(&staged_app.to_string_lossy()),
            dst = shell_quote(&install_target.to_string_lossy()),
        );
        fs::write(&script_path, script).map_err(|e| format!("write relaunch script: {e}"))?;
        // Make executable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path)
                .map_err(|e| format!("stat script: {e}"))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).map_err(|e| format!("chmod script: {e}"))?;
        }

        // Detach so the helper survives our exit (setsid).
        let mut cmd = Command::new("/bin/bash");
        cmd.arg(&script_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        cmd.spawn()
            .map_err(|e| format!("spawn relaunch helper: {e}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (staged_app, install_target);
        Err("auto-update install is only implemented on macOS".into())
    }
}

fn update_cache_dir() -> Result<PathBuf, String> {
    let dir = PathBuf::from(crate::root_cache_dir()).join("updates");
    fs::create_dir_all(&dir).map_err(|e| format!("create update cache: {e}"))?;
    Ok(dir)
}

fn ensure_download_url_allowed(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("bad download url: {e}"))?;
    if parsed.scheme() != "https" {
        return Err("download url must be https".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "download url missing host".to_string())?;
    if !ALLOWED_DOWNLOAD_HOSTS
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
    {
        return Err(format!("download host not allowed: {host}"));
    }
    Ok(())
}

fn parse_version(raw: &str) -> Result<semver::Version, String> {
    let s = strip_v(raw);
    semver::Version::parse(s).map_err(|e| format!("version '{raw}': {e}"))
}

fn strip_v(s: &str) -> &str {
    s.strip_prefix('v').unwrap_or(s)
}

fn sanitize_version(v: &str) -> String {
    strip_v(v)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_manifest(version: &str) -> Manifest {
        let mut platforms = HashMap::new();
        platforms.insert(
            "darwin-aarch64".into(),
            PlatformEntry {
                signature: "dGVzdA==".into(),
                url: format!(
                    "https://github.com/korbindeman/ochre/releases/download/v1.0.0/{}.app.tar.gz",
                    crate::product::NAME
                ),
            },
        );
        Manifest {
            version: version.into(),
            notes: "notes".into(),
            pub_date: Some("2026-07-19T00:00:00Z".into()),
            platforms,
        }
    }

    #[test]
    fn newer_version_is_offered() {
        let m = sample_manifest("0.2.0");
        let got = select_update(&m, "0.1.0").unwrap().expect("update");
        assert_eq!(got.version, "0.2.0");
        assert_eq!(got.current_version, "0.1.0");
    }

    #[test]
    fn same_or_older_is_none() {
        let m = sample_manifest("0.1.0");
        assert!(select_update(&m, "0.1.0").unwrap().is_none());
        assert!(select_update(&m, "0.2.0").unwrap().is_none());
    }

    #[test]
    fn prerelease_ordering() {
        let m = sample_manifest("0.1.0-alpha.2");
        let got = select_update(&m, "0.1.0-alpha.1")
            .unwrap()
            .expect("alpha bump");
        assert_eq!(got.version, "0.1.0-alpha.2");
        // Stable outranks alpha of the same numeric version.
        let stable = sample_manifest("0.1.0");
        assert!(select_update(&stable, "0.1.0-alpha.9").unwrap().is_some());
    }

    #[test]
    fn strips_leading_v() {
        let m = sample_manifest("v0.3.0");
        let got = select_update(&m, "v0.2.0").unwrap().expect("update");
        assert_eq!(got.version, "0.3.0");
        assert_eq!(got.current_version, "0.2.0");
    }

    #[test]
    fn rejects_non_https_or_unknown_host() {
        let mut m = sample_manifest("9.0.0");
        m.platforms.get_mut("darwin-aarch64").unwrap().url =
            "http://evil.example/app.tar.gz".into();
        assert!(select_update(&m, "0.1.0").is_err());

        m.platforms.get_mut("darwin-aarch64").unwrap().url =
            "https://evil.example/app.tar.gz".into();
        assert!(select_update(&m, "0.1.0").is_err());
    }

    #[test]
    fn verify_roundtrip_with_test_vectors() {
        // Fixture signed with the project minisign key (public key is compiled in).
        // Bytes of "hello\n".
        let data = b"hello\n";
        let sig_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test-hello.sig");
        let sig_raw = std::fs::read(&sig_path).expect("tests/test-hello.sig present");
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig_raw);
        verify_bytes(data, &sig_b64).expect("sig verifies");
    }
}
