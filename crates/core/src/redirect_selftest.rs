//! Subresource-redirect regression self-test (ADR-0028 guard).
//!
//! CEF/Chromium **149** stopped following HTTP redirects for subresources - the
//! request issued after a 3xx died with `net::ERR_INVALID_ARGUMENT` (it broke the
//! signed-in Google avatar, served via a `rd-ogw` redirect). We pinned CEF to the
//! 148 line to dodge it. Nothing in the unit suite exercises a real redirect
//! through CEF's network stack, so `just verify` passed on *both* the broken and
//! the fixed engine. This is the standing guard that fails loudly if a future CEF
//! bump reintroduces the regression - the network analogue of the focus self-test
//! (ADR-0018). **Re-run `just redirect-selftest` after any CEF version change.**
//!
//! Hermetic: an in-process HTTP server (loopback, ephemeral port) serves a page
//! whose `<img>` loads through a **same-origin 302**, then beacons the outcome
//! back. No external network. On a good engine the image follows the redirect and
//! `onload` fires (PASS); on a regressed engine the subresource errors and
//! `onerror` fires (FAIL). The beacon itself is a direct (non-redirect) request,
//! so it reaches us on either engine.

use std::ffi::c_void;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use objc2::rc::Retained;
use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::{embed, mac};

const W: f64 = 640.0;
const H: f64 = 480.0;
const TIMEOUT: Duration = Duration::from_secs(15);

/// The test page: an `<img>` whose src redirects (302) to a same-origin image,
/// beaconing the load outcome back. The beacon is a direct request (no redirect),
/// so it is delivered regardless of whether the redirect itself works.
const PAGE: &str = "<!doctype html><meta charset=utf-8><title>redirect-selftest</title>\
<body><img src=\"/redirect\" \
onload=\"fetch('/result?ok')\" onerror=\"fetch('/result?fail')\"></body>";

/// A 1×1 transparent PNG - a *decodable* image, so `onload` fires on a good
/// engine (garbage bytes would fail to decode and fire `onerror`, a false FAIL).
const PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

/// Verdict shared between the HTTP thread (producer) and the watchdog (consumer).
/// `Some(true)` = image followed the redirect and loaded; `Some(false)` = errored.
type Verdict = Arc<(Mutex<Option<bool>>, Condvar)>;

/// Run the self-test. Never returns: the watchdog exits the process with 0 (PASS)
/// or 1 (FAIL) once the verdict is in or the timeout elapses.
pub fn run() -> ! {
    let mtm = match MainThreadMarker::new() {
        Some(mtm) => mtm,
        None => fail("must run on the main thread"),
    };

    // Bind first so the URL carries the real port, and the server is listening
    // before the browser can request the page.
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) => fail(&format!("could not bind loopback server: {err}")),
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let url = format!("http://127.0.0.1:{port}/");

    let verdict: Verdict = Arc::new((Mutex::new(None), Condvar::new()));

    {
        let verdict = verdict.clone();
        thread::spawn(move || serve(listener, verdict));
    }
    {
        let verdict = verdict.clone();
        thread::spawn(move || watchdog(verdict));
    }

    if !embed::initialize() {
        fail("CEF failed to initialize");
    }

    // A minimal host window for the embedded browser. No custom container/delegate:
    // the process exits via the watchdog, not the window's close button. The window
    // is shown so CEF fully activates the browser; resource loading isn't gated on
    // it, but this matches the proven native-shell path.
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(W, H));
    let style =
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;
    let window: Retained<NSWindow> = unsafe {
        msg_send![
            NSWindow::alloc(mtm),
            initWithContentRect: rect,
            styleMask: style,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("redirect-selftest"));

    let view: Retained<NSView> = unsafe { msg_send![NSView::alloc(mtm), initWithFrame: rect] };
    window.setContentView(Some(&view));
    window.makeKeyAndOrderFront(None);
    mac::platform_show_window(None);

    let created = unsafe {
        embed::create_browser(
            Retained::as_ptr(&view) as *mut c_void,
            W as i32,
            H as i32,
            &url,
        )
    };
    if !created {
        fail("failed to create the embedded browser");
    }

    embed::run_message_loop();
    // The watchdog exits the process on a verdict/timeout; reaching here means the
    // loop ended first (e.g. the window was closed) - treat as a failure.
    fail("message loop ended before a verdict");
}

/// Block until the verdict arrives or the timeout elapses, then report and exit.
fn watchdog(verdict: Verdict) -> ! {
    let (lock, cvar) = &*verdict;
    let mut guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    while guard.is_none() {
        let (g, timed_out) = cvar
            .wait_timeout(guard, TIMEOUT)
            .unwrap_or_else(|p| p.into_inner());
        guard = g;
        if timed_out.timed_out() {
            break;
        }
    }
    match *guard {
        Some(true) => {
            println!("[redirect-selftest] result=PASS (subresource followed the 302 and loaded)");
            std::process::exit(0);
        }
        Some(false) => {
            println!(
                "[redirect-selftest] result=FAIL (redirected <img> errored - this CEF build drops \
                 subresource redirects; see ADR-0028)"
            );
            std::process::exit(1);
        }
        None => {
            println!("[redirect-selftest] result=FAIL (timeout - no load outcome within 15s)");
            std::process::exit(1);
        }
    }
}

/// Accept connections and answer each on its own short-lived thread (avoids any
/// head-of-line blocking between the page, the redirect, the image, the beacon).
fn serve(listener: TcpListener, verdict: Verdict) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let verdict = verdict.clone();
        thread::spawn(move || handle(stream, verdict));
    }
}

fn handle(mut stream: TcpStream, verdict: Verdict) {
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.split_whitespace().nth(1).unwrap_or("/");

    if path.starts_with("/result") {
        let ok = path.contains("ok");
        let (lock, cvar) = &*verdict;
        *lock.lock().unwrap_or_else(|p| p.into_inner()) = Some(ok);
        cvar.notify_all();
        let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
    } else if path.starts_with("/redirect") {
        // The subresource redirect under test: same-origin 302 to the image.
        let _ = stream.write_all(
            b"HTTP/1.1 302 Found\r\nLocation: /image.png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    } else if path.starts_with("/image.png") {
        let png = B64.decode(PNG_B64).unwrap_or_default();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            png.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&png);
    } else {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            PAGE.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(PAGE.as_bytes());
    }
}

/// Report a setup failure and exit non-zero (the test can't run ⇒ not a PASS).
fn fail(reason: &str) -> ! {
    println!("[redirect-selftest] result=FAIL ({reason})");
    std::process::exit(1);
}
