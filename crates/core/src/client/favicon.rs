//! Favicon download and caching for the sidebar tab list.
//!
//! CEF's `download_image` often yields empty `CefImage` handles for favicon
//! responses (ICO/SVG) even on HTTP 200, so we fetch the ranked candidate URLs
//! directly and store bytes NSImage can decode (PNG or ICO).

use std::io::Read;

use cef::*;

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

/// Higher scores win. SVG is last resort — rasterize poorly via CEF anyway.
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

fn root_favicon_url(page_url: &str, page_host: &str) -> Option<String> {
    if page_host.is_empty() {
        return None;
    }
    let authority = page_authority(page_url, page_host);
    Some(format!(
        "{}://{}/favicon.ico",
        page_scheme(page_url),
        authority
    ))
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
    if let Some(root) = root_favicon_url(page_url, page_host) {
        push(root);
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

/// Keep PNG/ICO bytes NSImage understands; skip SVG and other formats.
fn normalize_icon_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(bytes.to_vec());
    }
    if bytes.len() > 4 && bytes[..4] == [0, 0, 1, 0] {
        return extract_largest_ico_image(bytes);
    }
    None
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
            if let Some(bytes) = normalize_icon_bytes(slice) {
                best_area = area;
                best = Some(bytes);
            }
        }
    }
    best
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
        if !crate::private_window::is_private_cef(cef_id) {
            history::store_favicon(&host, &bytes);
        }
        if !crate::private_window::note_favicon(cef_id, &host) {
            tabs::note_favicon(cef_id, &host);
        }
        return;
    }
    eprintln!("[favicon] exhausted candidates for host={host}");
}

#[cfg(test)]
mod tests {
    use super::{icon_format_score, normalize_icon_bytes, rank_icon_urls, root_favicon_url};

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
            ranked,
            vec![
                "https://github.com/favicon.ico".to_string(),
                "https://github.githubassets.com/favicons/favicon.svg".to_string(),
            ]
        );
    }

    #[test]
    fn root_ico_preserves_loopback_port() {
        assert_eq!(
            root_favicon_url("http://127.0.0.1:8080/", "127.0.0.1"),
            Some("http://127.0.0.1:8080/favicon.ico".into())
        );
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
}
