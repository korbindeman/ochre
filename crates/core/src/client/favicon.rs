//! Favicon download and caching for the sidebar tab list.
//!
//! CEF's `download_image` often yields empty `CefImage` handles for favicon
//! responses (ICO/SVG) even on HTTP 200, so we fetch the ranked candidate URLs
//! directly and store PNG bytes NSImage can decode. Sites without `<link
//! rel="icon">` (e.g. apple.com) never fire `on_favicon_urlchange`; a load-end
//! fallback probes the usual well-known paths.

use std::io::{Cursor, Read};

use cef::*;
use image::{ImageEncoder, ImageReader};
use resvg::tiny_skia::{self, Pixmap};
use resvg::usvg::{self, Tree};

use crate::history;
use crate::tabs;

/// Borrow every icon URL from a CEF string list without consuming it.
fn collect_icon_urls(list: &mut CefStringList) -> Vec<String> {
    let raw: *mut sys::_cef_string_list_t = list.into();
    if raw.is_null() {
        return Vec::new();
    }
    unsafe {
        let size = sys::cef_string_list_size(raw);
        let mut urls = Vec::with_capacity(size);
        for i in 0..size {
            let mut value = std::mem::zeroed();
            if sys::cef_string_list_value(raw, i, &mut value) != 0 {
                urls.push(CefString::from(std::ptr::from_ref(&value)).to_string());
            }
        }
        urls
    }
}

/// Higher scores win. SVG is last resort — rasterize when no raster is available.
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
pub(super) fn rank_icon_urls(urls: &[String], page_url: &str, page_host: &str) -> Vec<String> {
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

/// Ranked favicon candidates from CEF's `on_favicon_urlchange` list.
pub(super) fn rank_icon_urls_from_list(
    list: &mut CefStringList,
    page_url: &str,
    page_host: &str,
) -> Vec<String> {
    rank_icon_urls(&collect_icon_urls(list), page_url, page_host)
}

fn fetch_icon_bytes(url: &str) -> Option<Vec<u8>> {
    let response = ureq::get(url).call().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut bytes = Vec::new();
    response
        .into_body()
        .with_config()
        .limit(256 * 1024)
        .reader()
        .read_to_end(&mut bytes)
        .ok()?;
    normalize_icon_bytes(&bytes)
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

/// Fetch ranked candidates until one yields storable icon bytes.
pub(super) fn start_favicon_download(cef_id: i32, host: String, candidates: Vec<String>) {
    for url in candidates {
        eprintln!("[favicon] fetching host={host} url={url}");
        let Some(bytes) = fetch_icon_bytes(&url) else {
            eprintln!("[favicon] failed host={host} url={url}");
            continue;
        };
        eprintln!(
            "[favicon] captured host={host} url={url} bytes={}",
            bytes.len()
        );
        store_favicon_for_browser(cef_id, &host, &bytes);
        if !crate::private_window::note_favicon(cef_id, &host) {
            tabs::note_favicon(cef_id, &host);
        }
        return;
    }
    eprintln!("[favicon] exhausted candidates for host={host}");
}

/// Try to capture a favicon from declared page URLs and well-known fallbacks.
pub(super) fn fetch_favicon(cef_id: i32, page_url: &str, declared_urls: &[String]) {
    let Some(host) = history::host_of(page_url) else {
        return;
    };
    let candidates = rank_icon_urls(declared_urls, page_url, &host);
    if candidates.is_empty() {
        return;
    }
    start_favicon_download(cef_id, host, candidates);
}

/// Load-end fallback: skip when a favicon is already cached for this host.
pub(super) fn fetch_favicon_if_missing(cef_id: i32, page_url: &str) {
    let Some(host) = history::host_of(page_url) else {
        return;
    };
    if history::favicon(&host).is_some() {
        return;
    }
    fetch_favicon(cef_id, page_url, &[]);
}

#[cfg(test)]
mod tests {
    use super::{fallback_icon_urls, icon_format_score, normalize_icon_bytes, rank_icon_urls};

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
