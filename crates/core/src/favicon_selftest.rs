//! Favicon-capture regression self-test.
//!
//! Hermetic loopback server serves a page whose declared icons mirror GitHub
//! (SVG first, ICO second). The engine must capture a transparent raster favicon
//! into the history store - not a white-plated SVG rasterization. Re-run after
//! any favicon-pipeline or CEF change.

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

use crate::{embed, history, mac};

const W: f64 = 640.0;
const H: f64 = 480.0;
const TIMEOUT: Duration = Duration::from_secs(20);

/// 1×1 transparent PNG - decodable raster with alpha.
const PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>favicon-selftest</title>\
<link rel=\"icon\" type=\"image/svg+xml\" href=\"/icon.svg\">\
<link rel=\"icon\" href=\"/icon.ico\" sizes=\"any\">";

const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 16 16\"><rect width=\"16\" height=\"16\" fill=\"white\"/><circle cx=\"8\" cy=\"8\" r=\"6\" fill=\"#24292E\"/></svg>";

type Verdict = Arc<(Mutex<Option<bool>>, Condvar)>;

pub fn run() -> ! {
    let mtm = match MainThreadMarker::new() {
        Some(mtm) => mtm,
        None => fail("must run on the main thread"),
    };

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
    window.setTitle(&NSString::from_str("favicon-selftest"));

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
    fail("message loop ended before a verdict");
}

fn watchdog(verdict: Verdict) -> ! {
    let host = "127.0.0.1";
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        if let Some(png) = history::favicon(host) {
            let pass = png_has_alpha(&png);
            let (lock, cvar) = &*verdict;
            *lock.lock().unwrap_or_else(|p| p.into_inner()) = Some(pass);
            cvar.notify_all();
            if pass {
                println!(
                    "[favicon-selftest] result=PASS (captured {} bytes with alpha for {host})",
                    png.len()
                );
                std::process::exit(0);
            }
            fail("favicon PNG was stored but has no alpha channel");
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    fail(&format!(
        "timeout - no favicon stored for {host} within {}s",
        TIMEOUT.as_secs()
    ));
}

fn png_has_alpha(png: &[u8]) -> bool {
    // IHDR color type: byte 25 in a normal PNG (8-byte signature + len/type/name).
    png.len() > 25 && matches!(png[25], 4 | 6)
}

fn serve(listener: TcpListener, verdict: Verdict) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let verdict = verdict.clone();
        thread::spawn(move || handle(stream, verdict));
    }
}

fn handle(mut stream: TcpStream, _verdict: Verdict) {
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.split_whitespace().nth(1).unwrap_or("/");

    if path.starts_with("/icon.svg") {
        let body = SVG.as_bytes();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
    } else if path.starts_with("/icon.ico") || path.starts_with("/favicon.ico") {
        let png = B64.decode(PNG_B64).unwrap_or_default();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/x-icon\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

fn fail(reason: &str) -> ! {
    println!("[favicon-selftest] result=FAIL ({reason})");
    std::process::exit(1);
}
