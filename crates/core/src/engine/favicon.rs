//! Favicon download and caching for the sidebar tab list.
//!
//! WebKit has no favicon API, so when a page finishes loading we read its
//! declared `<link rel="icon">` URLs with a script in an isolated world, rank
//! them, fetch the best candidate directly, and store PNG bytes NSImage can
//! decode. Pages without declared icons fall back to the well-known paths.
//!
//! ## Fetching runs off the UI thread, behind an SSRF guard
//!
//! The candidate URLs include the page's own `<link rel="icon" href="...">`
//! values, which are attacker-controlled. Two hazards follow, both handled here:
//!
//!   1. **SSRF.** A page can declare an icon at an internal host
//!      (`http://169.254.169.254/...`, `http://127.0.0.1:8080/admin`, a LAN
//!      router). Because this fetch runs in the privileged browser process via
//!      `ureq` (outside WebKit's network stack, so it bypasses content rules), an
//!      unguarded fetch is a blind cross-origin request into the LAN / cloud
//!      metadata / loopback services. [`fetch_url_allowed`] blocks that: only
//!      `http`/`https`, and a cross-origin target must resolve entirely to
//!      publicly-routable addresses. A page fetching *its own* origin (same
//!      host) is always allowed, so a local-dev page on loopback still gets its
//!      favicon. The guard re-runs on every redirect hop.
//!   2. **UI-thread stall.** The fetch is blocking HTTP; running it on the main
//!      thread lets a slow/tarpit server freeze the browser. [`start_favicon_download`]
//!      runs the download on a worker thread with explicit timeouts and marshals
//!      the store + notify back to the main thread via `main_thread::post`.

use std::io::{Cursor, Read};
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::time::Duration;

use image::{ImageEncoder, ImageReader};
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_foundation::{NSArray, NSError, NSString};
use objc2_web_kit::{WKContentWorld, WKWebView};
use resvg::tiny_skia::{self, Pixmap};
use resvg::usvg::{self, Tree};
use url::Url;

use crate::history;
use crate::tabs;

/// Per-request connect timeout for favicon fetches.
const FAVICON_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Overall wall-clock budget for a single favicon fetch (all redirect hops).
const FAVICON_TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
/// Redirect hops we follow before giving up on a candidate.
const FAVICON_MAX_REDIRECTS: usize = 4;
/// Cap on downloaded favicon bytes.
const FAVICON_MAX_BYTES: u64 = 256 * 1024;

/// Higher scores win. SVG is last resort - rasterize when no raster is available.
fn icon_format_score(url: &str) -> u8 {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if path.ends_with(".ico") {
        return 100;
    }
    if path.ends_with(".png") {
        return 90;
    }
    if path.ends_with(".webp") {
        return 80;
    }
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        return 75;
    }
    if path.ends_with(".gif") {
        return 70;
    }
    if path.ends_with(".svg") {
        return 10;
    }
    50
}

fn page_scheme(page_url: &str) -> &str {
    page_url
        .split("://")
        .next()
        .filter(|scheme| *scheme == "http" || *scheme == "https")
        .unwrap_or("https")
}

fn page_authority(page_url: &str, page_host: &str) -> String {
    if let Some(rest) = page_url.split("://").nth(1) {
        let authority = rest.split('/').next().unwrap_or(page_host);
        if !authority.is_empty() {
            return authority.to_string();
        }
    }
    page_host.to_string()
}

fn site_root_url(page_url: &str, page_host: &str, path: &str) -> Option<String> {
    if page_host.is_empty() {
        return None;
    }
    let authority = page_authority(page_url, page_host);
    Some(format!("{}://{}{}", page_scheme(page_url), authority, path))
}

/// Well-known icon paths many sites serve even without `<link rel="icon">`.
fn fallback_icon_urls(page_url: &str, page_host: &str) -> Vec<String> {
    [
        "/favicon.ico",
        "/apple-touch-icon.png",
        "/apple-touch-icon-precomposed.png",
        "/icon.png",
        "/favicon.png",
    ]
    .into_iter()
    .filter_map(|path| site_root_url(page_url, page_host, path))
    .collect()
}

/// Ordered, deduplicated favicon candidates: raster from the page first, then the
/// site-root ICO (matching scheme + port), then SVG.
pub(crate) fn rank_icon_urls(urls: &[String], page_url: &str, page_host: &str) -> Vec<String> {
    let mut ranked: Vec<(u8, String)> = urls
        .iter()
        .map(|url| (icon_format_score(url), url.clone()))
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |url: String| {
        if seen.insert(url.clone()) {
            out.push(url);
        }
    };

    for (_, url) in &ranked {
        if icon_format_score(url) >= 70 {
            push(url.clone());
        }
    }
    for (_, url) in &ranked {
        if icon_format_score(url) == 50 {
            push(url.clone());
        }
    }
    for url in fallback_icon_urls(page_url, page_host) {
        push(url);
    }
    for (_, url) in &ranked {
        if icon_format_score(url) == 10 {
            push(url.clone());
        }
    }
    out
}

/// True if `ip` is a publicly-routable address safe to fetch from a
/// cross-origin favicon URL. Rejects the ranges an SSRF wants to reach:
/// loopback, private, link-local (incl. cloud metadata `169.254.169.254`),
/// CGNAT, unique-local, unspecified, broadcast, multicast.
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            // `::ffff:a.b.c.d` reaches the same v4 host - judge it as v4.
            Some(v4) => is_public_v4(v4),
            None => {
                let first = v6.segments()[0];
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || (first & 0xfe00) == 0xfc00  // unique-local fc00::/7
                    || (first & 0xffc0) == 0xfe80) // link-local fe80::/10
            }
        },
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    !(v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_unspecified()
        || v4.is_multicast()
        || (a == 100 && (b & 0xc0) == 0x40)) // CGNAT 100.64.0.0/10
}

/// Gate a favicon fetch target. Allows only `http`/`https`; a same-host target
/// (the page's own origin) is always allowed so local-dev pages on loopback
/// still get their favicon; any other host must resolve entirely to public IPs.
fn fetch_url_allowed(url: &Url, page_host: &str) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case(page_host) {
        return true;
    }
    let port = url.port_or_known_default().unwrap_or(80);
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    let mut resolved = false;
    for addr in addrs {
        resolved = true;
        if !is_public_ip(addr.ip()) {
            return false;
        }
    }
    resolved
}

/// A dedicated favicon transport: bounded timeouts, and redirects handled by us
/// (`max_redirects(0)`) so [`fetch_icon_bytes`] can re-run the SSRF guard on
/// every hop rather than trusting `ureq` to follow blindly.
fn favicon_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_connect(Some(FAVICON_CONNECT_TIMEOUT))
        .timeout_global(Some(FAVICON_TOTAL_TIMEOUT))
        .build()
        .into()
}

/// Fetch one candidate, following (and re-validating) up to
/// [`FAVICON_MAX_REDIRECTS`] redirects. Returns storable PNG bytes or `None`.
///
/// Note: `to_socket_addrs` resolves for the guard, then `ureq` resolves again to
/// connect, so a DNS-rebinding attacker could in principle race the two. That is
/// a blind GET (no response reaches the page); the load-bearing wins - IP-literal
/// targets and redirect bounces to internal hosts - are fully closed here.
fn fetch_icon_bytes(agent: &ureq::Agent, start_url: &str, page_host: &str) -> Option<Vec<u8>> {
    let mut current = Url::parse(start_url).ok()?;
    for _ in 0..=FAVICON_MAX_REDIRECTS {
        if !fetch_url_allowed(&current, page_host) {
            eprintln!("[favicon] blocked non-public target page_host={page_host} url={current}");
            return None;
        }
        let response = agent.get(current.as_str()).call().ok()?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(ureq::http::header::LOCATION)?
                .to_str()
                .ok()?;
            current = current.join(location).ok()?;
            continue;
        }
        if !status.is_success() {
            return None;
        }
        let mut bytes = Vec::new();
        response
            .into_body()
            .with_config()
            .limit(FAVICON_MAX_BYTES)
            .reader()
            .read_to_end(&mut bytes)
            .ok()?;
        return normalize_icon_bytes(&bytes);
    }
    None
}

/// Keep PNG bytes NSImage understands; rasterize or transcode other formats.
fn normalize_icon_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(bytes.to_vec());
    }
    if bytes.starts_with(b"<svg") || bytes.windows(5).any(|w| w == b"<svg ") {
        return rasterize_svg(bytes);
    }
    if bytes.len() > 4 && bytes[..4] == [0, 0, 1, 0] {
        if let Some(png) = extract_largest_ico_image(bytes) {
            return Some(png);
        }
        return encode_image_to_png(bytes);
    }
    encode_image_to_png(bytes)
}

fn encode_image_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let image = reader.decode().ok()?;
    let rgba = image.to_rgba8();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(out)
}

fn rasterize_svg(svg: &[u8]) -> Option<Vec<u8>> {
    let tree = Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let width = size.width().ceil().max(1.0) as u32;
    let height = size.height().ceil().max(1.0) as u32;
    let (width, height) = (width.min(256), height.min(256));
    let mut pixmap = Pixmap::new(width, height)?;
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

fn extract_largest_ico_image(ico: &[u8]) -> Option<Vec<u8>> {
    if ico.len() < 6 {
        return None;
    }
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut best: Option<Vec<u8>> = None;
    let mut best_area = 0usize;
    for i in 0..count {
        let entry = 6 + i * 16;
        if entry + 16 > ico.len() {
            break;
        }
        let w = ico[entry] as usize;
        let h = ico[entry + 1] as usize;
        let width = if w == 0 { 256 } else { w };
        let height = if h == 0 { 256 } else { h };
        let size = u32::from_le_bytes(ico[entry + 8..entry + 12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(ico[entry + 12..entry + 16].try_into().ok()?) as usize;
        if offset + size > ico.len() {
            continue;
        }
        let slice = &ico[offset..offset + size];
        let area = width * height;
        if area >= best_area {
            let decoded = normalize_icon_bytes(slice).or_else(|| encode_image_to_png(slice));
            if let Some(bytes) = decoded {
                best_area = area;
                best = Some(bytes);
            }
        }
    }
    best
}

fn store_favicon_for_browser(cef_id: i32, host: &str, bytes: &[u8]) {
    if crate::private_window::is_private_cef(cef_id) {
        history::store_session_favicon(host, bytes);
    } else {
        history::store_favicon(host, bytes);
    }
}

/// Persist the captured favicon and notify the tab list. Must run on the UI
/// thread (it emits a `tabs::Event` that the native layer marshals to its main
/// actor).
fn store_and_note(cef_id: i32, host: &str, bytes: &[u8]) {
    store_favicon_for_browser(cef_id, host, bytes);
    if !crate::private_window::note_favicon(cef_id, host) {
        tabs::note_favicon(cef_id, host);
    }
}

/// Hand the captured favicon to the main thread (callers are download workers).
fn deliver_favicon(cef_id: i32, host: String, bytes: Vec<u8>) {
    crate::main_thread::post(move || store_and_note(cef_id, &host, &bytes));
}

/// Download ranked candidates on a worker thread (blocking HTTP must stay off
/// the main thread), stopping at the first that yields storable icon bytes.
pub(crate) fn start_favicon_download(cef_id: i32, host: String, candidates: Vec<String>) {
    std::thread::spawn(move || {
        let agent = favicon_agent();
        for url in &candidates {
            eprintln!("[favicon] fetching host={host} url={url}");
            let Some(bytes) = fetch_icon_bytes(&agent, url, &host) else {
                eprintln!("[favicon] failed host={host} url={url}");
                continue;
            };
            eprintln!(
                "[favicon] captured host={host} url={url} bytes={}",
                bytes.len()
            );
            deliver_favicon(cef_id, host.clone(), bytes);
            return;
        }
        eprintln!("[favicon] exhausted candidates for host={host}");
    });
}

/// Try to capture a favicon from declared page URLs and well-known fallbacks.
pub(crate) fn fetch_favicon(cef_id: i32, page_url: &str, declared_urls: &[String]) {
    let Some(host) = history::host_of(page_url) else {
        return;
    };
    let candidates = rank_icon_urls(declared_urls, page_url, &host);
    if candidates.is_empty() {
        return;
    }
    start_favicon_download(cef_id, host, candidates);
}

/// Script (isolated world) that lists the page's declared icon URLs.
const DECLARED_ICONS_JS: &str = "Array.from(document.querySelectorAll('link[rel~=\\'icon\\'],link[rel=\\'apple-touch-icon\\']')).map(function(l){return l.href}).filter(Boolean)";

/// After a main-frame load: read the declared icons and fetch the best one,
/// unless this host already has a cached favicon.
pub(crate) fn collect_from_page(cef_id: i32, view: &WKWebView) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: main thread (navigation-delegate callback).
    let Some(page_url) = unsafe { view.URL() }
        .and_then(|u| u.absoluteString())
        .map(|s| s.to_string())
    else {
        return;
    };
    let Some(host) = history::host_of(&page_url) else {
        return;
    };
    if history::favicon(&host).is_some() {
        return;
    }
    let done = block2::RcBlock::new(move |result: *mut AnyObject, _err: *mut NSError| {
        let declared: Vec<String> = unsafe { result.as_ref() }
            .and_then(|r| r.downcast_ref::<NSArray>())
            .map(|array| {
                array
                    .iter()
                    .filter_map(|item| item.downcast_ref::<NSString>().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        fetch_favicon(cef_id, &page_url, &declared);
    });
    // SAFETY: main thread; the client world keeps page scripts out.
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(DECLARED_ICONS_JS),
            None,
            &WKContentWorld::defaultClientWorld(mtm),
            Some(&done),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        fallback_icon_urls, fetch_url_allowed, icon_format_score, is_public_ip,
        normalize_icon_bytes, rank_icon_urls,
    };
    use std::net::IpAddr;
    use url::Url;

    fn allowed(url: &str, page_host: &str) -> bool {
        fetch_url_allowed(&Url::parse(url).unwrap(), page_host)
    }

    #[test]
    fn ssrf_guard_blocks_cross_origin_internal_hosts() {
        // Cloud metadata, loopback, LAN, CGNAT, unique-local - all cross-origin.
        assert!(!allowed(
            "http://169.254.169.254/latest/meta-data/",
            "evil.com"
        ));
        assert!(!allowed("http://127.0.0.1:8080/admin", "evil.com"));
        assert!(!allowed("http://10.0.0.1/setup", "evil.com"));
        assert!(!allowed("http://192.168.1.1/", "evil.com"));
        assert!(!allowed("http://172.16.5.4/", "evil.com"));
        assert!(!allowed("http://100.64.0.1/", "evil.com"));
        assert!(!allowed("http://[::1]/", "evil.com"));
        assert!(!allowed("http://[fd00::1]/", "evil.com"));
        assert!(!allowed("http://0.0.0.0/", "evil.com"));
    }

    #[test]
    fn ssrf_guard_rejects_non_http_schemes() {
        assert!(!allowed("ftp://example.com/icon.ico", "evil.com"));
        assert!(!allowed("file:///etc/passwd", "evil.com"));
        assert!(!allowed("data:image/png;base64,AAAA", "evil.com"));
    }

    #[test]
    fn ssrf_guard_allows_same_origin_even_on_loopback() {
        // A user visiting their own local-dev server still gets its favicon.
        assert!(allowed("http://127.0.0.1:8080/favicon.ico", "127.0.0.1"));
        assert!(allowed("http://localhost/favicon.ico", "localhost"));
        assert!(allowed("https://EXAMPLE.com/favicon.ico", "example.com"));
    }

    #[test]
    fn ssrf_guard_allows_cross_origin_public_ip() {
        // Public IP literal (example.com) fetched cross-origin is fine.
        assert!(allowed("https://93.184.216.34/favicon.ico", "example.com"));
    }

    #[test]
    fn public_ip_classification() {
        let pub_v4: IpAddr = "8.8.8.8".parse().unwrap();
        let meta: IpAddr = "169.254.169.254".parse().unwrap();
        let loop_v6: IpAddr = "::1".parse().unwrap();
        let mapped_loopback: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(is_public_ip(pub_v4));
        assert!(!is_public_ip(meta));
        assert!(!is_public_ip(loop_v6));
        assert!(!is_public_ip(mapped_loopback));
    }

    #[test]
    fn prefers_ico_over_svg() {
        let urls = vec![
            "https://github.githubassets.com/favicons/favicon.svg".into(),
            "https://github.com/favicon.ico".into(),
        ];
        let ranked = rank_icon_urls(&urls, "https://github.com/", "github.com");
        assert_eq!(
            ranked.first().map(String::as_str),
            Some("https://github.com/favicon.ico")
        );
    }

    #[test]
    fn svg_only_inserts_root_ico_before_svg() {
        let urls = vec!["https://github.githubassets.com/favicons/favicon.svg".into()];
        let ranked = rank_icon_urls(&urls, "https://github.com/", "github.com");
        assert_eq!(
            ranked.first().map(String::as_str),
            Some("https://github.com/favicon.ico")
        );
        assert!(ranked.iter().any(|u| u.ends_with(".svg")));
    }

    #[test]
    fn empty_declared_urls_still_probe_well_known_paths() {
        let ranked = rank_icon_urls(&[], "https://www.apple.com/", "www.apple.com");
        assert_eq!(
            ranked.first().map(String::as_str),
            Some("https://www.apple.com/favicon.ico")
        );
        assert!(ranked.iter().any(|u| u.contains("apple-touch-icon")));
    }

    #[test]
    fn root_ico_preserves_loopback_port() {
        let urls = fallback_icon_urls("http://127.0.0.1:8080/", "127.0.0.1");
        assert_eq!(
            urls.first().map(String::as_str),
            Some("http://127.0.0.1:8080/favicon.ico")
        );
    }

    #[test]
    fn fallback_urls_include_touch_icon() {
        let urls = fallback_icon_urls("https://www.apple.com/", "www.apple.com");
        assert!(urls.iter().any(|u| u.ends_with("/apple-touch-icon.png")));
    }

    #[test]
    fn png_beats_extensionless_before_root_ico() {
        let urls = vec![
            "https://cdn.example.com/favicon".into(),
            "https://cdn.example.com/icon.png".into(),
        ];
        let ranked = rank_icon_urls(&urls, "https://example.com/", "example.com");
        assert_eq!(
            ranked.first().map(String::as_str),
            Some("https://cdn.example.com/icon.png")
        );
    }

    #[test]
    fn format_scores_order_raster_above_svg() {
        assert!(
            icon_format_score("https://x.com/a.ico") > icon_format_score("https://x.com/a.svg")
        );
        assert!(
            icon_format_score("https://x.com/a.png") > icon_format_score("https://x.com/a.svg")
        );
    }

    #[test]
    fn accepts_png_magic() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\r";
        assert!(normalize_icon_bytes(png).is_some());
    }

    #[test]
    fn rasterizes_minimal_svg() {
        let svg = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">"#,
            r#"<circle cx="8" cy="8" r="6" fill="black"/></svg>"#
        );
        let png = normalize_icon_bytes(svg.as_bytes()).expect("svg should rasterize");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn decodes_apple_favicon_ico() {
        let ico = include_bytes!("../../tests/fixtures/apple-favicon.ico");
        let png = normalize_icon_bytes(ico).expect("apple favicon.ico should decode");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() > 100);
    }
}
