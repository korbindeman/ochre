//! Local crash payloads for post-mortem debugging.
//!
//! On Rust panics, fatal signals, uncaught ObjC exceptions, and (on next launch)
//! newly arrived macOS `.ips` reports, the core writes structured JSON under
//! `{root_cache_dir}/crashes/` (plus a `latest.json` pointer) so an agent can
//! ask "why did it crash?" and read one place. Production Crashpad upload is a
//! later milestone (roadmap); this is dev-first, always-on local dumps.
//!
//! ## Why signals need special care
//!
//! Chromium/CEF installs its own handlers during `cef_initialize`, so we
//! re-install ours *after* that. The signal path itself must stay close to
//! async-signal-safe: no heap, no mutex, no `Backtrace` / `serde` in the
//! handler. A full JSON payload (logs, notes, structure) is written only for
//! panics and Swift exceptions. Signals write a minimal JSON with fault
//! address, then re-raise so Apple's Crash Reporter still produces an `.ips`.
//!
//! Stderr is teed into `{root_cache_dir}/diagnostics.log` at install time so
//! panic/swift payloads can include recent log lines without touching every
//! `eprintln!` callsite.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::panic;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::root_cache_dir;

const MAX_NOTES: usize = 200;
const LOG_TAIL_BYTES: usize = 64 * 1024;
const SCHEMA_VERSION: u32 = 2;
/// Alternate stack for signal handlers (stack-overflow SIGSEGV cannot run on
/// the overflowing stack).
const ALT_STACK_SIZE: usize = 64 * 1024;
/// Max bytes written from a signal handler (stack buffer, no heap).
const SIGNAL_DUMP_CAP: usize = 2048;

static INSTALLED: Once = Once::new();
static WRITING: AtomicBool = AtomicBool::new(false);
static NOTES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// Pre-opened O_APPEND fd into `crashes/signal-live.jsonl` for the signal path.
static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);
/// NUL-terminated absolute path to the next signal dump file (dated).
static SIGNAL_PATH: OnceLock<Vec<u8>> = OnceLock::new();
static ALT_STACK: OnceLock<Vec<u8>> = OnceLock::new();

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum CrashKind {
    Panic,
    /// Written by the async-signal-safe handler as raw JSON (not via
    /// `write_payload`); kept so schema docs and kind tags stay complete.
    #[allow(dead_code)]
    Signal,
    SwiftException,
    /// Copied from `~/Library/Logs/DiagnosticReports` on a later launch.
    SystemReport,
}

#[derive(Debug, Serialize)]
struct ProcessInfo {
    pid: u32,
    role: String,
    args: Vec<String>,
    executable: String,
    build_profile: &'static str,
}

#[derive(Debug, Serialize)]
struct CrashPayload {
    schema_version: u32,
    recorded_at_unix_ms: u128,
    kind: CrashKind,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fault_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signal: Option<i32>,
    backtrace: String,
    process: ProcessInfo,
    core_version: &'static str,
    structure: Option<Value>,
    recent_logs: Vec<String>,
    notes: Vec<String>,
    crashes_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_report_path: Option<String>,
}

/// Install crash hooks once per process: panic handler, fatal-signal handlers,
/// stderr tee, and a one-shot harvest of any Apple `.ips` reports we missed.
pub fn install() {
    INSTALLED.call_once(|| {
        let _ = fs::create_dir_all(crashes_dir());
        let _ = fs::create_dir_all(crashes_dir().join("ips"));
        prepare_signal_path();
        prepare_signal_fd();
        install_alt_stack();
        install_stderr_tee(&diagnostics_log_path());
        install_panic_hook();
        install_signal_handlers();
        // Pull in any `.ips` from a previous session that never wrote JSON
        // (handlers replaced by CEF, stack overflow, SIGKILL, etc.).
        harvest_system_crash_reports();
        note("crash_report installed");
    });
}

/// Re-install fatal signal handlers after CEF init.
///
/// Chromium installs its own handlers inside `cef_initialize`. Without this
/// call, in-process SIGSEGV often never reaches our dump path (as with the
/// AppKit fullscreen-exit crash that left only an Apple `.ips`).
pub fn reinstall_after_cef() {
    install();
    install_signal_handlers();
    note("crash_report signal handlers reinstalled after CEF");
}

/// Append a short breadcrumb included in the next panic/swift crash payload.
pub fn note(message: impl Into<String>) {
    if let Ok(mut notes) = NOTES.lock() {
        notes.push_back(message.into());
        while notes.len() > MAX_NOTES {
            notes.pop_front();
        }
    }
}

/// Directory where crash JSON payloads are written.
pub fn crashes_dir() -> PathBuf {
    PathBuf::from(root_cache_dir()).join("crashes")
}

/// Path to the stderr/diagnostics log tee.
pub fn diagnostics_log_path() -> PathBuf {
    PathBuf::from(root_cache_dir()).join("diagnostics.log")
}

/// Path to the most recently written crash payload.
pub fn latest_path() -> PathBuf {
    crashes_dir().join("latest.json")
}

/// Write a crash payload from the Swift uncaught-exception handler.
pub fn report_swift_exception(name: &str, reason: &str) {
    let message = if reason.is_empty() {
        name.to_string()
    } else {
        format!("{name}: {reason}")
    };
    write_payload(
        CrashKind::SwiftException,
        message,
        None,
        None,
        String::new(),
        None,
    );
}

/// Copy new Apple crash reports into `crashes/ips/` and write a JSON summary
/// as `latest.json` when newer than the current pointer.
///
/// Safe to call multiple times; tracks harvested basenames in
/// `crashes/harvested-ips.txt`.
pub fn harvest_system_crash_reports() {
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let roots = [
        PathBuf::from(format!("{home}/Library/Logs/DiagnosticReports")),
        PathBuf::from(format!("{home}/Library/Logs/DiagnosticReports/Retired")),
    ];
    let harvested_list = crashes_dir().join("harvested-ips.txt");
    let mut known = read_lines_set(&harvested_list);
    let mut newest: Option<(std::time::SystemTime, PathBuf, String)> = None;

    for root in &roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !is_product_ips_name(&name) {
                continue;
            }
            if known.contains(&name) {
                continue;
            }
            let dest = crashes_dir().join("ips").join(&name);
            if fs::copy(&path, &dest).is_err() {
                continue;
            }
            known.insert(name.clone());
            let summary = summarize_ips(&dest)
                .unwrap_or_else(|| format!("system crash report (Apple .ips): {name}"));
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let stamp = mtime
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let payload = CrashPayload {
                schema_version: SCHEMA_VERSION,
                recorded_at_unix_ms: stamp,
                kind: CrashKind::SystemReport,
                message: summary,
                fault_address: None,
                signal: None,
                backtrace: String::new(),
                process: process_info(),
                core_version: crate::CORE_VERSION,
                structure: None,
                recent_logs: Vec::new(),
                notes: vec![format!("harvested from {}", path.display())],
                crashes_dir: crashes_dir().display().to_string(),
                system_report_path: Some(dest.display().to_string()),
            };
            if let Ok(json) = serde_json::to_string_pretty(&payload) {
                let dated = crashes_dir().join(format!("{stamp}-system.json"));
                let _ = write_atomic(&dated, &json);
                if newest.as_ref().map(|(t, _, _)| mtime > *t).unwrap_or(true) {
                    newest = Some((mtime, dated, json));
                }
            }
        }
    }

    {
        let mut names: Vec<_> = known.into_iter().collect();
        names.sort();
        let mut body = names.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        let _ = fs::write(&harvested_list, body);
    }

    if let Some((mtime, _, json)) = newest {
        // Only replace latest.json when this harvest is newer than what is there.
        let replace = match fs::metadata(latest_path()).and_then(|m| m.modified()) {
            Ok(existing) => mtime >= existing,
            Err(_) => true,
        };
        if replace {
            let _ = fs::write(latest_path(), json);
            let _ = writeln!(
                io::stderr(),
                "[crash] harvested system report(s); latest → {}",
                latest_path().display()
            );
        }
    }
}

fn is_product_ips_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(".ips") {
        return false;
    }
    let mut prefixes = vec![
        crate::product::NAME.to_ascii_lowercase(),
        crate::product::HELPER_STEM.to_ascii_lowercase(),
    ];
    for legacy in crate::product::LEGACY_NAMES {
        let legacy = legacy.to_ascii_lowercase();
        if !prefixes.contains(&legacy) {
            prefixes.push(legacy);
        }
    }
    for prefix in &prefixes {
        if lower.starts_with(&format!("{prefix}-")) || lower.starts_with(&format!("{prefix} ")) {
            return true;
        }
    }
    let ids = [
        crate::product::BUNDLE_ID.to_ascii_lowercase(),
        crate::product::BUNDLE_ID_DEV.to_ascii_lowercase(),
    ];
    ids.iter().any(|id| lower.contains(id.as_str()))
}

fn read_lines_set(path: &Path) -> std::collections::HashSet<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return std::collections::HashSet::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Best-effort extract of exception + top frames from an Apple `.ips` file.
fn summarize_ips(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    // IPS is often a one-line header JSON object, then a second full report.
    let report = ips_main_json(&text)?;
    let exc = report.get("exception");
    let exc_type = exc
        .and_then(|e| e.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let signal = exc
        .and_then(|e| e.get("signal"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let subtype = exc
        .and_then(|e| e.get("subtype"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut frames = Vec::new();
    if let Some(threads) = report.get("threads").and_then(|t| t.as_array()) {
        let triggered = threads
            .iter()
            .find(|t| t.get("triggered").and_then(|v| v.as_bool()) == Some(true))
            .or_else(|| threads.first());
        if let Some(thread) = triggered {
            if let Some(fs) = thread.get("frames").and_then(|f| f.as_array()) {
                for frame in fs.iter().take(8) {
                    let sym = frame
                        .get("symbol")
                        .and_then(|s| s.as_str())
                        .unwrap_or("???");
                    let img = frame
                        .get("imageIndex")
                        .and_then(|i| i.as_u64())
                        .map(|i| i.to_string())
                        .unwrap_or_else(|| "?".into());
                    frames.push(format!("{sym} [img {img}]"));
                }
            }
        }
    }
    let mut msg = format!("Apple .ips: {exc_type}");
    if !signal.is_empty() {
        msg.push_str(" / ");
        msg.push_str(signal);
    }
    if !subtype.is_empty() {
        msg.push_str(" (");
        msg.push_str(subtype);
        msg.push(')');
    }
    if !frames.is_empty() {
        msg.push_str(" | ");
        msg.push_str(&frames.join(" ← "));
    }
    Some(msg)
}

fn ips_main_json(text: &str) -> Option<Value> {
    // Prefer the object with the most keys (full report vs one-line header).
    let mut best: Option<Value> = None;
    let mut best_keys = 0usize;
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        rest = &rest[start..];
        let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
        match stream.next() {
            Some(Ok(val)) => {
                let keys = val.as_object().map(|o| o.len()).unwrap_or(0);
                if keys >= best_keys {
                    best_keys = keys;
                    best = Some(val);
                }
                let consumed = stream.byte_offset().max(1);
                rest = &rest[consumed.min(rest.len())..];
            }
            _ => {
                rest = &rest[1..];
            }
        }
    }
    best
}

fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let message = match info.payload().downcast_ref::<&str>() {
            Some(s) => (*s).to_string(),
            None => info
                .payload()
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "panic".to_string()),
        };
        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "unknown".to_string());
        write_payload(
            CrashKind::Panic,
            format!("{message} (at {location})"),
            None,
            None,
            format!("{:?}", std::backtrace::Backtrace::force_capture()),
            None,
        );
        default_hook(info);
    }));
}

fn install_signal_handlers() {
    // SAFETY: handlers only run on fatal signals; dump path is async-signal-safe.
    unsafe {
        register_signal(libc::SIGABRT);
        register_signal(libc::SIGBUS);
        register_signal(libc::SIGFPE);
        register_signal(libc::SIGILL);
        register_signal(libc::SIGSEGV);
        register_signal(libc::SIGTRAP);
    }
}

fn install_alt_stack() {
    let stack = ALT_STACK.get_or_init(|| vec![0u8; ALT_STACK_SIZE]);
    let ss = libc::stack_t {
        ss_sp: stack.as_ptr() as *mut libc::c_void,
        ss_size: stack.len(),
        ss_flags: 0,
    };
    // SAFETY: stack lives for process lifetime in ALT_STACK.
    unsafe {
        let _ = libc::sigaltstack(&ss as *const _, std::ptr::null_mut());
    }
}

unsafe fn register_signal(sig: libc::c_int) {
    let mut action: libc::sigaction = std::mem::zeroed();
    action.sa_sigaction = handle_fatal_signal as *const () as usize;
    // SA_NODEFER avoided; SA_RESETHAND so a fault in the handler re-raises default.
    // SA_ONSTACK uses the alternate stack (stack overflow).
    action.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND | libc::SA_ONSTACK;
    libc::sigemptyset(&mut action.sa_mask as *mut _);
    libc::sigaction(sig, &action, std::ptr::null_mut());
}

extern "C" fn handle_fatal_signal(
    sig: libc::c_int,
    info: *mut libc::siginfo_t,
    _ctx: *mut libc::c_void,
) {
    // Async-signal-safe dump only. No heap, locks, or Rust formatting macros.
    if WRITING.swap(true, Ordering::SeqCst) {
        unsafe {
            re_raise(sig);
        }
        return;
    }

    let fault: usize = if info.is_null() {
        0
    } else {
        // SAFETY: kernel-provided siginfo for this signal delivery.
        unsafe { (*info).si_addr as usize }
    };
    let code: i32 = if info.is_null() {
        0
    } else {
        unsafe { (*info).si_code }
    };
    let pid = process::id();

    let mut buf = [0u8; SIGNAL_DUMP_CAP];
    let n = write_signal_json(&mut buf, sig, code, fault, pid);
    if n > 0 {
        // Prefer the pre-opened append log (always works if fd is live).
        let fd = SIGNAL_FD.load(Ordering::SeqCst);
        if fd >= 0 {
            unsafe {
                let _ = libc::write(fd, buf.as_ptr() as *const libc::c_void, n);
                let nl = b"\n";
                let _ = libc::write(fd, nl.as_ptr() as *const libc::c_void, 1);
            }
        }
        // Also write a dated file + latest.json via open/write/close (AS-safe).
        unsafe {
            write_signal_files(&buf[..n]);
        }
    }

    unsafe {
        re_raise(sig);
    }
}

/// Build a compact JSON document into `out` without heap allocation.
/// Returns bytes written (0 if the buffer was too small for a useful dump).
fn write_signal_json(out: &mut [u8], sig: i32, code: i32, fault: usize, pid: u32) -> usize {
    // Fixed skeleton; insert decimal/hex with only stack helpers.
    // {"schema_version":2,"kind":"signal","message":"SIGSEGV","signal":11,
    //  "fault_address":"0x...","si_code":1,"process":{"pid":123},"core_version":"..."}
    let mut w = SignalWriter { buf: out, pos: 0 };
    let name = signal_name_static(sig);
    w.push_str("{\"schema_version\":2,\"kind\":\"signal\",\"message\":\"");
    w.push_str(name);
    w.push_str("\",\"signal\":");
    w.push_i64(i64::from(sig));
    w.push_str(",\"si_code\":");
    w.push_i64(i64::from(code));
    w.push_str(",\"fault_address\":\"0x");
    w.push_hex_usize(fault);
    w.push_str("\",\"process\":{\"pid\":");
    w.push_i64(i64::from(pid));
    w.push_str("},\"core_version\":\"");
    w.push_str(crate::CORE_VERSION);
    w.push_str("\",\"note\":\"minimal signal dump; see Apple .ips for native frames\"}");
    w.pos
}

struct SignalWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl SignalWriter<'_> {
    fn push_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let space = self.buf.len().saturating_sub(self.pos);
        let n = bytes.len().min(space);
        self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
        self.pos += n;
    }

    fn push_i64(&mut self, mut v: i64) {
        if v < 0 {
            self.push_str("-");
            // Saturate; si_code / signal are small.
            v = v.saturating_neg();
        }
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        if v == 0 {
            self.push_str("0");
            return;
        }
        while v > 0 && i > 0 {
            i -= 1;
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        let s = std::str::from_utf8(&tmp[i..]).unwrap_or("0");
        self.push_str(s);
    }

    fn push_hex_usize(&mut self, mut v: usize) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut tmp = [0u8; 16];
        let mut i = tmp.len();
        if v == 0 {
            self.push_str("0");
            return;
        }
        while v > 0 && i > 0 {
            i -= 1;
            tmp[i] = HEX[v & 0xf];
            v >>= 4;
        }
        let s = std::str::from_utf8(&tmp[i..]).unwrap_or("0");
        self.push_str(s);
    }
}

fn signal_name_static(sig: libc::c_int) -> &'static str {
    match sig {
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        libc::SIGFPE => "SIGFPE",
        libc::SIGILL => "SIGILL",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGTRAP => "SIGTRAP",
        _ => "SIGNAL",
    }
}

/// open/write/close dated signal dump + latest.json (async-signal-safe syscalls).
unsafe fn write_signal_files(json: &[u8]) {
    // Path template prepared at install: ".../crashes/signal-pending.json"
    // We overwrite that well-known path and latest.json so agents always find it.
    let Some(path) = SIGNAL_PATH.get() else {
        return;
    };
    // path includes trailing NUL from CString.
    let fd = libc::open(
        path.as_ptr() as *const libc::c_char,
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
        0o644,
    );
    if fd >= 0 {
        let _ = libc::write(fd, json.as_ptr() as *const libc::c_void, json.len());
        libc::close(fd);
    }

    // latest.json next to signal-pending: replace "signal-pending.json" suffix.
    // Build latest path by rewriting the known suffix in a stack buffer.
    let mut latest = [0u8; 512];
    let plen = path.len().saturating_sub(1); // exclude NUL
    if plen + 1 > latest.len() {
        return;
    }
    latest[..plen].copy_from_slice(&path[..plen]);
    // Find last '/' and write "latest.json" after it.
    let mut slash = 0usize;
    for (i, b) in path[..plen].iter().enumerate() {
        if *b == b'/' {
            slash = i;
        }
    }
    let name = b"latest.json";
    let start = slash + 1;
    if start + name.len() + 1 > latest.len() {
        return;
    }
    latest[start..start + name.len()].copy_from_slice(name);
    latest[start + name.len()] = 0;
    let fd = libc::open(
        latest.as_ptr() as *const libc::c_char,
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
        0o644,
    );
    if fd >= 0 {
        let _ = libc::write(fd, json.as_ptr() as *const libc::c_void, json.len());
        libc::close(fd);
    }
}

unsafe fn re_raise(sig: libc::c_int) {
    // Restore default and re-raise so Crash Reporter still writes an .ips.
    libc::signal(sig, libc::SIG_DFL);
    libc::raise(sig);
    // If raise returns (shouldn't), hard-exit.
    libc::_exit(128 + sig);
}

fn prepare_signal_path() {
    let path = crashes_dir().join("signal-pending.json");
    let mut bytes = path.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    let _ = SIGNAL_PATH.set(bytes);
}

fn prepare_signal_fd() {
    let path = crashes_dir().join("signal-live.jsonl");
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        // Leak the fd for the process lifetime (handler must not close it).
        let fd: RawFd = file.into_raw_fd();
        SIGNAL_FD.store(fd, Ordering::SeqCst);
    }
}

fn write_payload(
    kind: CrashKind,
    message: String,
    fault_address: Option<String>,
    signal: Option<i32>,
    backtrace: String,
    system_report_path: Option<String>,
) {
    if WRITING.swap(true, Ordering::SeqCst) {
        return;
    }

    let payload = CrashPayload {
        schema_version: SCHEMA_VERSION,
        recorded_at_unix_ms: unix_ms_now(),
        kind,
        message,
        fault_address,
        signal,
        backtrace,
        process: process_info(),
        core_version: crate::CORE_VERSION,
        structure: capture_structure(),
        recent_logs: tail_lines(&diagnostics_log_path(), LOG_TAIL_BYTES),
        notes: NOTES
            .lock()
            .map(|notes| notes.iter().cloned().collect())
            .unwrap_or_default(),
        crashes_dir: crashes_dir().display().to_string(),
        system_report_path,
    };

    let Ok(json) = serde_json::to_string_pretty(&payload) else {
        WRITING.store(false, Ordering::SeqCst);
        return;
    };

    let _ = fs::create_dir_all(crashes_dir());
    let stamp = payload.recorded_at_unix_ms;
    let kind_tag = match payload.kind {
        CrashKind::Panic => "panic",
        CrashKind::Signal => "signal",
        CrashKind::SwiftException => "swift",
        CrashKind::SystemReport => "system",
    };
    let dated = crashes_dir().join(format!("{stamp}-{kind_tag}.json"));
    if write_atomic(&dated, &json).is_ok() {
        let _ = fs::write(latest_path(), &json);
        let _ = writeln!(
            io::stderr(),
            "[crash] wrote payload to {}",
            latest_path().display()
        );
    }
    // Allow a later harvest or secondary report in the same process only for
    // non-fatal paths; panics/signals process is dying anyway.
    if matches!(kind, CrashKind::SystemReport) {
        WRITING.store(false, Ordering::SeqCst);
    }
}

fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn process_info() -> ProcessInfo {
    let role = std::env::args()
        .find(|arg| arg.starts_with("--type="))
        .unwrap_or_else(|| "browser".to_string());
    ProcessInfo {
        pid: process::id(),
        role,
        args: std::env::args().collect(),
        executable: std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string()),
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    }
}

fn capture_structure() -> Option<Value> {
    let snapshot = panic::catch_unwind(crate::tabs::snapshot).ok()?;
    serde_json::to_value(snapshot).ok()
}

fn tail_lines(path: &Path, max_bytes: usize) -> Vec<String> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
    if len > max_bytes {
        let _ = file.seek(SeekFrom::End(-(max_bytes as i64)));
    }
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return Vec::new();
    }
    buf.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn unix_ms_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn install_stderr_tee(log_path: &Path) {
    let Ok(log_file) = OpenOptions::new().create(true).append(true).open(log_path) else {
        return;
    };

    let mut pipe_fds = [0i32; 2];
    // SAFETY: `pipe` fills two valid fds on success.
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        return;
    }
    let read_fd = pipe_fds[0];
    let write_fd = pipe_fds[1];

    let stderr_fd = io::stderr().as_raw_fd();
    // SAFETY: dup/dup2 on valid fds.
    let saved_stderr = unsafe { libc::dup(stderr_fd) };
    if saved_stderr < 0 {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return;
    }
    // SAFETY: redirect process stderr through the pipe write end.
    unsafe {
        if libc::dup2(write_fd, stderr_fd) < 0 {
            libc::close(saved_stderr);
            libc::close(read_fd);
            libc::close(write_fd);
            return;
        }
        libc::close(write_fd);
    }

    thread::spawn(move || {
        // SAFETY: read_fd was returned by pipe(); saved_stderr by dup().
        let mut pipe_reader = unsafe { fs::File::from_raw_fd(read_fd) };
        let mut terminal = unsafe { fs::File::from_raw_fd(saved_stderr) };
        let mut log = log_file;
        let mut buf = [0u8; 4096];
        loop {
            match pipe_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = terminal.write_all(&buf[..n]);
                    let _ = log.write_all(&buf[..n]);
                    let _ = terminal.flush();
                    let _ = log.flush();
                }
                Err(_) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_serializes() {
        let payload = CrashPayload {
            schema_version: SCHEMA_VERSION,
            recorded_at_unix_ms: 1,
            kind: CrashKind::Panic,
            message: "test".to_string(),
            fault_address: None,
            signal: None,
            backtrace: String::new(),
            process: ProcessInfo {
                pid: 42,
                role: "browser".to_string(),
                args: vec!["browser".to_string()],
                executable: "/tmp/browser".to_string(),
                build_profile: "debug",
            },
            core_version: "0.1.0",
            structure: None,
            recent_logs: vec!["line".to_string()],
            notes: vec!["note".to_string()],
            crashes_dir: "/tmp/crashes".to_string(),
            system_report_path: None,
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(json.contains("schema_version"));
        assert!(json.contains("recent_logs"));
    }

    #[test]
    fn signal_json_contains_fault_address() {
        let mut buf = [0u8; SIGNAL_DUMP_CAP];
        let n = write_signal_json(&mut buf, libc::SIGSEGV, 1, 0x6306_0000_0010, 7264);
        assert!(n > 0);
        let s = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(s.contains("SIGSEGV"));
        assert!(s.contains("0x630600000010"));
        assert!(s.contains("\"pid\":7264"));
        assert!(serde_json::from_str::<Value>(s).is_ok());
    }

    #[test]
    fn browser_ips_name_filter() {
        assert!(is_product_ips_name("Browser-2026-07-20-041420.ips"));
        assert!(is_product_ips_name(&format!(
            "{}-2026-08-14-120000.ips",
            crate::product::NAME
        )));
        assert!(is_product_ips_name("browser-helper-1.ips"));
        assert!(!is_product_ips_name("Safari-2026.ips"));
        assert!(!is_product_ips_name("notes.txt"));
    }

    #[test]
    fn summarize_ips_extracts_exception() {
        let sample = r#"{"app_name":"Ochre"}
{
  "exception" : {"type":"EXC_BAD_ACCESS","signal":"SIGSEGV","subtype":"KERN_INVALID_ADDRESS"},
  "threads" : [{
    "triggered" : true,
    "frames" : [
      {"symbol":"objc_msgSend","imageIndex":5},
      {"symbol":"-[_NSExitFullScreenTransitionController _doSucceededToExitFullScreen]","imageIndex":6}
    ]
  }]
}
"#;
        let dir = std::env::temp_dir().join(format!("ochre-ips-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("sample.ips");
        fs::write(&path, sample).unwrap();
        let summary = summarize_ips(&path).expect("summary");
        assert!(summary.contains("EXC_BAD_ACCESS"));
        assert!(summary.contains("SIGSEGV"));
        assert!(summary.contains("objc_msgSend"));
        assert!(summary.contains("_NSExitFullScreenTransitionController"));
        let _ = fs::remove_dir_all(&dir);
    }
}
