//! Local crash payloads for post-mortem debugging.
//!
//! On Rust panics and fatal signals the core writes a structured JSON document to
//! `{root_cache_dir}/crashes/` (plus a `latest.json` pointer) so an agent can ask
//! "why did it crash?" and read one file. Production Crashpad upload is a later
//! milestone (roadmap); this is dev-first, always-on local dumps.
//!
//! Stderr is teed into `{root_cache_dir}/diagnostics.log` at install time so the
//! payload can include recent log lines without touching every `eprintln!` callsite.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::panic;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::root_cache_dir;

const MAX_NOTES: usize = 200;
const LOG_TAIL_BYTES: usize = 64 * 1024;

static INSTALLED: Once = Once::new();
static WRITING: AtomicBool = AtomicBool::new(false);
static NOTES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum CrashKind {
    Panic,
    Signal,
    SwiftException,
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
    backtrace: String,
    process: ProcessInfo,
    core_version: &'static str,
    structure: Option<Value>,
    recent_logs: Vec<String>,
    notes: Vec<String>,
    crashes_dir: String,
}

/// Install crash hooks once per process: panic handler, fatal-signal handlers,
/// and stderr tee into `diagnostics.log`.
pub fn install() {
    INSTALLED.call_once(|| {
        let _ = fs::create_dir_all(crashes_dir());
        install_stderr_tee(&diagnostics_log_path());
        install_panic_hook();
        install_signal_handlers();
        note("crash_report installed");
    });
}

/// Append a short breadcrumb included in the next crash payload.
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
    write_payload(CrashKind::SwiftException, message);
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
        write_payload(CrashKind::Panic, format!("{message} (at {location})"));
        default_hook(info);
    }));
}

fn install_signal_handlers() {
    // SAFETY: handlers only run on fatal signals; we accept best-effort I/O.
    unsafe {
        register_signal(libc::SIGABRT);
        register_signal(libc::SIGBUS);
        register_signal(libc::SIGFPE);
        register_signal(libc::SIGILL);
        register_signal(libc::SIGSEGV);
        register_signal(libc::SIGTRAP);
    }
}

unsafe fn register_signal(sig: libc::c_int) {
    let mut action: libc::sigaction = std::mem::zeroed();
    action.sa_sigaction = handle_fatal_signal as *const () as usize;
    action.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND;
    libc::sigemptyset(&mut action.sa_mask as *mut _);
    libc::sigaction(sig, &action, std::ptr::null_mut());
}

extern "C" fn handle_fatal_signal(
    sig: libc::c_int,
    _info: *mut libc::siginfo_t,
    _ctx: *mut libc::c_void,
) {
    let message = signal_name(sig);
    write_payload(CrashKind::Signal, message);
    unsafe {
        libc::_exit(128 + sig);
    }
}

fn signal_name(sig: libc::c_int) -> String {
    match sig {
        libc::SIGABRT => "SIGABRT".to_string(),
        libc::SIGBUS => "SIGBUS".to_string(),
        libc::SIGFPE => "SIGFPE".to_string(),
        libc::SIGILL => "SIGILL".to_string(),
        libc::SIGSEGV => "SIGSEGV".to_string(),
        libc::SIGTRAP => "SIGTRAP".to_string(),
        other => format!("signal {other}"),
    }
}

fn write_payload(kind: CrashKind, message: String) {
    if WRITING.swap(true, Ordering::SeqCst) {
        return;
    }

    let payload = CrashPayload {
        schema_version: 1,
        recorded_at_unix_ms: unix_ms_now(),
        kind,
        message,
        backtrace: format!("{:?}", std::backtrace::Backtrace::force_capture()),
        process: process_info(),
        core_version: crate::CORE_VERSION,
        structure: capture_structure(),
        recent_logs: tail_lines(&diagnostics_log_path(), LOG_TAIL_BYTES),
        notes: NOTES
            .lock()
            .map(|notes| notes.iter().cloned().collect())
            .unwrap_or_default(),
        crashes_dir: crashes_dir().display().to_string(),
    };

    let Ok(json) = serde_json::to_string_pretty(&payload) else {
        return;
    };

    let _ = fs::create_dir_all(crashes_dir());
    let stamp = payload.recorded_at_unix_ms;
    let kind_tag = match payload.kind {
        CrashKind::Panic => "panic",
        CrashKind::Signal => "signal",
        CrashKind::SwiftException => "swift",
    };
    let dated = crashes_dir().join(format!("{stamp}-{kind_tag}.json"));
    if write_atomic(&dated, &json).is_ok() {
        let _ = fs::write(latest_path(), &json);
        // Best-effort stderr in case the terminal is still attached.
        let _ = writeln!(
            io::stderr(),
            "[crash] wrote payload to {}",
            latest_path().display()
        );
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
            schema_version: 1,
            recorded_at_unix_ms: 1,
            kind: CrashKind::Panic,
            message: "test".to_string(),
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
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(json.contains("schema_version"));
        assert!(json.contains("recent_logs"));
    }
}
