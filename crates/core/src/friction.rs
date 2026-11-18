//! Friction capture (P1-44): one user action saves the evidence an agent needs
//! to work on "that felt wrong" - the tab structure, the recent diagnostics
//! log, what had focus, and the user's note - into
//! `{root_cache_dir}/friction/<unix-ms>/`.
//!
//! Core-owned so every frontend writes the same bundle. The frontend supplies
//! only what it alone knows (the focused surface) and adds `window.png` to the
//! returned directory itself. Everything stays on this machine; nothing is
//! uploaded.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::root_cache_dir;

/// How much of the diagnostics log tail to keep: enough for the minutes
/// before the report, small enough to read in one pass.
const LOG_TAIL_BYTES: u64 = 256 * 1024;

#[derive(Serialize)]
struct Report<'a> {
    unix_ms: u128,
    core_version: &'static str,
    note: &'a str,
    focused: &'a str,
    visible_url: Option<String>,
}

/// Write a friction bundle and return its directory, or `None` if the
/// directory could not be created. Main thread only (reads the live URL).
pub fn capture(note: &str, focused: &str) -> Option<PathBuf> {
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = Path::new(&root_cache_dir())
        .join("friction")
        .join(unix_ms.to_string());
    let report = Report {
        unix_ms,
        core_version: crate::CORE_VERSION,
        note,
        focused,
        visible_url: crate::embed::current_url(),
    };
    write_bundle(&dir, &report, &crate::crash_report::diagnostics_log_path())?;
    eprintln!("[friction] saved {}", dir.display());
    Some(dir)
}

fn write_bundle(dir: &Path, report: &Report<'_>, log: &Path) -> Option<()> {
    fs::create_dir_all(dir).ok()?;
    // Best effort past this point: a partial bundle is still useful evidence.
    if let Ok(json) = serde_json::to_string_pretty(report) {
        let _ = fs::write(dir.join("report.json"), json);
    }
    if let Ok(json) = serde_json::to_string_pretty(&crate::tabs::snapshot()) {
        let _ = fs::write(dir.join("structure.json"), json);
    }
    let _ = fs::write(dir.join("diagnostics.log"), log_tail(log, LOG_TAIL_BYTES));
    Some(())
}

/// The last `max` bytes of `path`, starting at a line boundary.
fn log_tail(path: &Path, max: u64) -> Vec<u8> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    let start = bytes.len().saturating_sub(max as usize);
    if start == 0 {
        return bytes;
    }
    let tail = &bytes[start..];
    let line_start = tail.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1);
    tail[line_start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_holds_report_structure_and_log_tail() {
        let root = std::env::temp_dir().join(format!("friction-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let log = root.join("diagnostics.log");
        fs::write(&log, "old line\nrecent line\n").unwrap();
        let dir = root.join("bundle");
        let report = Report {
            unix_ms: 1,
            core_version: "test",
            note: "favorite opened in the wrong space",
            focused: "sidebar",
            visible_url: None,
        };

        write_bundle(&dir, &report, &log).unwrap();

        let saved = fs::read_to_string(dir.join("report.json")).unwrap();
        assert!(saved.contains("favorite opened in the wrong space"));
        assert!(dir.join("structure.json").exists());
        assert_eq!(
            fs::read(dir.join("diagnostics.log")).unwrap(),
            fs::read(&log).unwrap()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn log_tail_starts_at_a_line_boundary() {
        let root = std::env::temp_dir().join(format!("friction-tail-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("log");
        fs::write(&log, "aaaa\nbbbb\ncccc\n").unwrap();
        assert_eq!(log_tail(&log, 7), b"cccc\n");
        let _ = fs::remove_dir_all(&root);
    }
}
