//! WKWebView spike for ADR-0071 (backlog P0-5). Throwaway measurement code, not product code.
//!
//! Subcommands:
//! - `convert <list.txt> <out-dir>`: compile EasyList-syntax text to WebKit content-rule JSON with
//!   adblock-rust (the Brave iOS path), split into chunks, and print coverage stats. Also writes
//!   `hosts.txt`, the plain `||host^` rules the hostname proxy uses.
//! - `run <tasks.json> <out-dir> [--rules <dir>] [--proxy <hosts.txt>]`: load each task URL in a
//!   WKWebView (fresh non-persistent store), wait, run its probe JS, record the resource URLs the
//!   page loaded, and save a snapshot. Writes `<out-dir>/results.json`.
//! - `leaks <results.json> <list.txt>`: count loaded resources that the CEF engine (same
//!   adblock-rust lists) would have blocked. Those are the WebKit path's misses.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use adblock::lists::{FilterSet, ParseOptions};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapImageFileType,
    NSBitmapImageRep, NSImage, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURLRequest,
    NSURL,
};
use objc2_web_kit::{
    WKAudiovisualMediaTypes, WKContentRuleList, WKContentRuleListStore, WKContentWorld,
    WKSnapshotConfiguration, WKUserScript, WKUserScriptInjectionTime, WKWebView,
    WKWebViewConfiguration, WKWebsiteDataStore,
};
use serde_json::{json, Value};

const CHUNK: usize = 50_000;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("convert") => convert(&args[2], &args[3]),
        Some("run") => run(&args[2..]),
        Some("leaks") => leaks(&args[2], &args[3]),
        _ => eprintln!("usage: convert <list> <out> | run <tasks> <out> [--rules d] [--proxy f] | leaks <results> <list>"),
    }
}

// ---------- convert ----------

fn convert(list_path: &str, out: &str) {
    let text = std::fs::read_to_string(list_path).expect("read list");
    std::fs::create_dir_all(out).unwrap();
    let total_lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('!') && !l.starts_with('['))
        .count();
    let mut set = FilterSet::new(true);
    set.add_filter_list(&text, ParseOptions::default());
    let t = std::time::Instant::now();
    let (rules, used) = set.into_content_blocking().expect("debug filter set");
    let used_set: HashSet<&str> = used.iter().map(String::as_str).collect();

    // Classify what did not convert, by the feature that likely blocked it.
    let mut dropped = std::collections::BTreeMap::<&str, usize>::new();
    for l in text.lines().map(str::trim) {
        if l.is_empty() || l.starts_with('!') || l.starts_with('[') || used_set.contains(l) {
            continue;
        }
        let k = if l.contains("##+js(") || l.contains("#%#") {
            "scriptlet"
        } else if l.contains("#?#") || l.contains(":has(") || l.contains(":-abp-") {
            "procedural cosmetic"
        } else if l.contains("#@#") {
            "cosmetic exception"
        } else if l.contains("##") {
            "cosmetic other"
        } else if l.contains("$removeparam") || l.contains("$redirect") || l.contains("$csp") {
            "request rewrite option"
        } else if l.starts_with('/') && l.ends_with('/') || l.contains("/$") && l.starts_with('/') {
            "regex"
        } else if l.contains('$') {
            "network with options"
        } else {
            "network other"
        };
        *dropped.entry(k).or_default() += 1;
    }

    for (i, chunk) in rules.chunks(CHUNK).enumerate() {
        let path = format!("{out}/rules-{i}.json");
        std::fs::write(&path, serde_json::to_string(chunk).unwrap()).unwrap();
    }

    let mut hosts = Vec::new();
    for l in text.lines().map(str::trim) {
        let l = l.strip_suffix("$third-party").unwrap_or(l);
        if let Some(h) = l.strip_prefix("||").and_then(|r| r.strip_suffix('^')) {
            if !h.is_empty() && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
                hosts.push(h.to_string());
            }
        }
    }
    std::fs::write(format!("{out}/hosts.txt"), hosts.join("\n")).unwrap();

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "filter_lines": total_lines,
            "filters_converted": used.len(),
            "webkit_rules": rules.len(),
            "chunks": rules.len().div_ceil(CHUNK),
            "plain_host_rules": hosts.len(),
            "convert_ms": t.elapsed().as_millis(),
            "not_converted_by_kind": dropped,
        }))
        .unwrap()
    );
}

// ---------- leaks ----------

fn request_type(initiator: &str) -> &'static str {
    match initiator {
        "script" => "script",
        "img" | "image" => "image",
        "css" | "link" => "stylesheet",
        "xmlhttprequest" | "fetch" | "beacon" => "xhr",
        "iframe" | "frame" => "sub_frame",
        "video" | "audio" => "media",
        _ => "other",
    }
}

fn leaks(results_path: &str, list_path: &str) {
    let results: Value = serde_json::from_str(&std::fs::read_to_string(results_path).unwrap()).unwrap();
    let text = std::fs::read_to_string(list_path).unwrap();
    let engine = adblock::Engine::from_rules(text.lines(), ParseOptions::default());
    let mut rows = Vec::new();
    for task in results.as_array().unwrap() {
        let page = task["final_url"].as_str().unwrap_or("");
        let mut leaked = Vec::new();
        let resources = task["probe"]["resources"].as_array().cloned().unwrap_or_default();
        for r in &resources {
            let (Some(u), Some(t)) = (r[0].as_str(), r[1].as_str()) else { continue };
            if let Ok(req) = adblock::request::Request::new(u, page, request_type(t)) {
                if engine.check_network_request(&req).matched {
                    leaked.push(u.to_string());
                }
            }
        }
        rows.push(json!({
            "url": task["url"],
            "resources": resources.len(),
            "would_block": leaked.len(),
            "proxy_blocked": task["proxy_blocked"],
            "sample": leaked.iter().take(5).collect::<Vec<_>>(),
        }));
    }
    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
}

// ---------- hostname proxy ----------

/// Minimal HTTP CONNECT proxy. Refuses tunnels to blocked hosts (suffix match), relays the rest.
/// Sees only the hostname: no TLS interception, by design (ADR-0071).
fn start_proxy(hosts: HashSet<String>, counter: Arc<AtomicUsize>, log: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hosts = Arc::new(hosts);
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let hosts = hosts.clone();
            let counter = counter.clone();
            let log = log.clone();
            std::thread::spawn(move || handle_proxy(client, &hosts, &counter, &log));
        }
    });
    port
}

fn blocked(host: &str, hosts: &HashSet<String>) -> bool {
    let mut h = host;
    loop {
        if hosts.contains(h) {
            return true;
        }
        match h.find('.') {
            Some(i) => h = &h[i + 1..],
            None => return false,
        }
    }
}

fn handle_proxy(mut client: TcpStream, hosts: &HashSet<String>, counter: &AtomicUsize, log: &Mutex<Vec<String>>) {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        match client.read(&mut byte) {
            Ok(1) => buf.push(byte[0]),
            _ => return,
        }
        if buf.len() > 16 * 1024 {
            return;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let mut parts = head.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else { return };
    if method != "CONNECT" {
        let _ = client.write_all(b"HTTP/1.1 501 Not Implemented\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    let host = target.rsplit_once(':').map(|(h, _)| h).unwrap_or(target);
    if blocked(host, hosts) {
        counter.fetch_add(1, Ordering::Relaxed);
        log.lock().unwrap().push(host.to_string());
        let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    let Ok(upstream) = TcpStream::connect(target) else {
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
        return;
    };
    let _ = client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
    let (mut c2, mut u2) = (client.try_clone().unwrap(), upstream.try_clone().unwrap());
    let (mut c1, mut u1) = (client, upstream);
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut c2, &mut u2);
        let _ = u2.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut u1, &mut c1);
    let _ = c1.shutdown(std::net::Shutdown::Write);
}

#[link(name = "Network", kind = "framework")]
extern "C" {
    fn nw_endpoint_create_host(host: *const std::ffi::c_char, port: *const std::ffi::c_char) -> *mut AnyObject;
    fn nw_proxy_config_create_http_connect(endpoint: *mut AnyObject, tls: *mut AnyObject) -> *mut AnyObject;
}

// ---------- run ----------

/// Records every resource the page loaded, plus the task's own probe value.
const COLLECT_JS: &str = r#"JSON.stringify({
  title: document.title,
  ready: document.readyState,
  resources: performance.getEntriesByType('resource').map(e => [e.name, e.initiatorType]),
  probe: window.__probe === undefined ? null : window.__probe,
  pruned: window.__ochrePruned || null,
})"#;

/// Page-world scriptlet in the json-prune style uBlock Origin uses on YouTube: strips ad
/// payloads from the player response before the player reads it, and records what it removed.
const YT_PRUNE_JS: &str = r#"(() => {
  const keys = ['adPlacements', 'playerAds', 'adSlots'];
  window.__ochrePruned = [];
  const prune = (o, tag) => {
    if (o && typeof o === 'object') for (const k of keys) if (k in o) { delete o[k]; window.__ochrePruned.push(tag + k); }
    return o;
  };
  let v;
  Object.defineProperty(window, 'ytInitialPlayerResponse', {
    configurable: true, get() { return v; }, set(x) { v = prune(x, 'init:'); },
  });
  const parse = JSON.parse;
  JSON.parse = function (...a) { return prune(parse.apply(this, a), 'json:'); };
  const rj = Response.prototype.json;
  Response.prototype.json = function () { return rj.call(this).then(o => prune(o, 'fetch:')); };
})();"#;

struct Runner {
    tasks: Vec<Value>,
    idx: usize,
    out: String,
    results: Vec<Value>,
    webview: Retained<WKWebView>,
    proxy_count: Arc<AtomicUsize>,
    proxy_log: Arc<Mutex<Vec<String>>>,
    proxy_start: usize,
}

fn after(secs: f64, f: impl Fn() + 'static) {
    let block = RcBlock::new(move |_: NonNull<NSTimer>| f());
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(secs, false, &block) };
}

fn next_task(r: Rc<RefCell<Runner>>) {
    let (url, wait) = {
        let mut s = r.borrow_mut();
        if s.idx >= s.tasks.len() {
            let path = format!("{}/results.json", s.out);
            std::fs::write(&path, serde_json::to_string_pretty(&s.results).unwrap()).unwrap();
            eprintln!("wrote {path}");
            std::process::exit(0);
        }
        s.proxy_start = s.proxy_count.load(Ordering::Relaxed);
        s.proxy_log.lock().unwrap().clear();
        let t = &s.tasks[s.idx];
        (t["url"].as_str().unwrap().to_string(), t["wait"].as_f64().unwrap_or(8.0))
    };
    eprintln!("[{}] {url}", r.borrow().idx);
    {
        let s = r.borrow();
        let nsurl = NSURL::URLWithString(&NSString::from_str(&url)).unwrap();
        unsafe { s.webview.loadRequest(&NSURLRequest::requestWithURL(&nsurl)) };
    }
    let r2 = r.clone();
    after(wait, move || {
        let pre = r2.borrow().tasks[r2.borrow().idx]["probe_js"].as_str().map(str::to_string);
        let wv = r2.borrow().webview.clone();
        if let Some(js) = pre {
            let log = RcBlock::new(|_: *mut AnyObject, err: *mut NSError| {
                if let Some(e) = unsafe { err.as_ref() } {
                    eprintln!("probe_js error: {:?}", e.userInfo());
                }
            });
            let js = format!("(() => {{ {js} }})(); 0");
            unsafe { wv.evaluateJavaScript_completionHandler(&NSString::from_str(&js), Some(&log)) };
        }
        let r3 = r2.clone();
        // Give an async probe a moment to settle before collecting.
        after(4.0, move || collect(r3.clone()));
    });
}

fn collect(r: Rc<RefCell<Runner>>) {
    let wv = r.borrow().webview.clone();
    let r2 = r.clone();
    let block = RcBlock::new(move |res: *mut AnyObject, err: *mut NSError| {
        let probe: Value = unsafe { res.as_ref() }
            .and_then(|o| o.downcast_ref::<NSString>())
            .and_then(|s| serde_json::from_str(&s.to_string()).ok())
            .unwrap_or_else(|| json!({ "error": unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string()) }));
        let idx = r2.borrow().idx;
        let final_url = unsafe { r2.borrow().webview.URL() }
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let proxy_blocked = {
            let s = r2.borrow();
            let n = s.proxy_count.load(Ordering::Relaxed) - s.proxy_start;
            let log = s.proxy_log.lock().unwrap().clone();
            json!({ "count": n, "hosts": log })
        };
        let shot = format!("{}/{idx:02}.png", r2.borrow().out);
        let url = r2.borrow().tasks[idx]["url"].clone();
        r2.borrow_mut().results.push(json!({
            "url": url, "final_url": final_url, "probe": probe,
            "proxy_blocked": proxy_blocked, "snapshot": shot,
        }));
        snapshot(r2.clone(), shot);
    });
    unsafe { wv.evaluateJavaScript_completionHandler(&NSString::from_str(COLLECT_JS), Some(&block)) };
}

fn snapshot(r: Rc<RefCell<Runner>>, path: String) {
    let mtm = MainThreadMarker::new().unwrap();
    let wv = r.borrow().webview.clone();
    let cfg = unsafe { WKSnapshotConfiguration::new(mtm) };
    let r2 = r.clone();
    let block = RcBlock::new(move |img: *mut NSImage, _err: *mut NSError| {
        if let Some(img) = unsafe { img.as_ref() } {
            if let Some(tiff) = img.TIFFRepresentation() {
                if let Some(rep) = unsafe { NSBitmapImageRep::imageRepWithData(&tiff) } {
                    if let Some(png) = unsafe {
                        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
                    } {
                        let _ = std::fs::write(&path, png.to_vec());
                    }
                }
            }
        }
        r2.borrow_mut().idx += 1;
        next_task(r2.clone());
    });
    unsafe { wv.takeSnapshotWithConfiguration_completionHandler(Some(&cfg), &block) };
}

fn run(args: &[String]) {
    let tasks: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&args[0]).unwrap()).unwrap();
    let out = args[1].clone();
    std::fs::create_dir_all(&out).unwrap();
    let flag = |name: &str| args.iter().position(|a| a == name).map(|i| args[i + 1].clone());
    let rules_dir = flag("--rules");
    let proxy_hosts = flag("--proxy");

    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    let store = unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
    let proxy_count = Arc::new(AtomicUsize::new(0));
    let proxy_log = Arc::new(Mutex::new(Vec::new()));
    if let Some(path) = &proxy_hosts {
        let hosts: HashSet<String> = std::fs::read_to_string(path).unwrap().lines().map(str::to_string).collect();
        let port = start_proxy(hosts, proxy_count.clone(), proxy_log.clone());
        eprintln!("proxy on 127.0.0.1:{port}");
        let host = std::ffi::CString::new("127.0.0.1").unwrap();
        let port = std::ffi::CString::new(port.to_string()).unwrap();
        unsafe {
            let ep = nw_endpoint_create_host(host.as_ptr(), port.as_ptr());
            let pc = nw_proxy_config_create_http_connect(ep, std::ptr::null_mut());
            let pc: Retained<AnyObject> = Retained::from_raw(pc).unwrap();
            let arr = NSArray::from_retained_slice(&[pc]);
            let _: () = msg_send![&*store, setProxyConfigurations: &*arr];
        }
    }
    unsafe {
        config.setWebsiteDataStore(&store);
        config.setMediaTypesRequiringUserActionForPlayback(WKAudiovisualMediaTypes::None);
    }
    let ucc = unsafe { config.userContentController() };
    if tasks.iter().any(|t| t["inject"] == "yt") {
        let script = unsafe {
            WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                mtm.alloc(),
                &NSString::from_str(YT_PRUNE_JS),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
                &WKContentWorld::pageWorld(mtm),
            )
        };
        unsafe { ucc.addUserScript(&script) };
    }

    let frame = NSRect::new(NSPoint::new(80.0, 80.0), NSSize::new(1280.0, 860.0));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            frame,
            NSWindowStyleMask::Titled | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let webview = unsafe { WKWebView::initWithFrame_configuration(mtm.alloc(), frame, &config) };
    window.setContentView(Some(&webview));
    window.orderFront(None);

    let runner = Rc::new(RefCell::new(Runner {
        tasks,
        idx: 0,
        out,
        results: Vec::new(),
        webview,
        proxy_count,
        proxy_log,
        proxy_start: 0,
    }));

    // Compile the rule chunks one at a time, then start the corpus.
    let mut chunks: Vec<String> = Vec::new();
    if let Some(dir) = &rules_dir {
        let mut i = 0;
        while let Ok(s) = std::fs::read_to_string(format!("{dir}/rules-{i}.json")) {
            chunks.push(s);
            i += 1;
        }
    }
    compile_chunks(Rc::new(RefCell::new(chunks)), 0, ucc, runner, std::time::Instant::now());
    app.run();
}

fn compile_chunks(
    chunks: Rc<RefCell<Vec<String>>>,
    i: usize,
    ucc: Retained<objc2_web_kit::WKUserContentController>,
    runner: Rc<RefCell<Runner>>,
    started: std::time::Instant,
) {
    if i >= chunks.borrow().len() {
        if i > 0 {
            eprintln!("compiled {i} rule lists in {} ms", started.elapsed().as_millis());
        }
        next_task(runner);
        return;
    }
    let mtm = MainThreadMarker::new().unwrap();
    let store = unsafe { WKContentRuleListStore::defaultStore(mtm) }.unwrap();
    let json = NSString::from_str(&chunks.borrow()[i]);
    let id = NSString::from_str(&format!("ochre-spike-{i}"));
    let (c2, u2, r2) = (chunks.clone(), ucc.clone(), runner.clone());
    let block = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
        match unsafe { list.as_ref() } {
            Some(list) => unsafe { u2.addContentRuleList(list) },
            None => eprintln!(
                "rule list {i} failed: {:?}",
                unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string())
            ),
        }
        compile_chunks(c2.clone(), i + 1, u2.clone(), r2.clone(), started);
    });
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(&id),
            Some(&json),
            Some(&block),
        )
    };
}
