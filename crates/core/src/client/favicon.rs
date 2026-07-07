//! Favicon download and caching for the sidebar tab list.

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

/// Higher scores win. SVG is last resort — CEF rasterizes it on an opaque plate.
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
    // Extensionless CDN paths (e.g. `/favicon`) — try before SVG, after raster.
    50
}

/// Pick the best favicon URL to download. Prefer raster formats from the page's
/// declared icons; if only SVG is declared, use `/favicon.ico` on the page host
/// before falling back to SVG (most sites serve a transparent ICO there).
pub(super) fn pick_icon_url(urls: &[String], page_host: &str) -> Option<String> {
    if let Some(url) = urls
        .iter()
        .max_by_key(|url| icon_format_score(url))
        .filter(|url| icon_format_score(url) >= 70)
    {
        return Some(url.clone());
    }

    if let Some(url) = urls
        .iter()
        .max_by_key(|url| icon_format_score(url))
        .filter(|url| icon_format_score(url) > 10)
    {
        return Some(url.clone());
    }

    if !page_host.is_empty() {
        return Some(format!("https://{page_host}/favicon.ico"));
    }

    urls.first().cloned()
}

/// Best favicon URL from CEF's `on_favicon_urlchange` list for `page_host`.
pub(super) fn best_icon_url(list: &mut CefStringList, page_host: &str) -> Option<String> {
    let urls = collect_icon_urls(list);
    pick_icon_url(&urls, page_host)
}

// `cef_id` is the CEF browser identifier of the requesting tab (translated to
// a tab id by `tabs::note_favicon`).
wrap_download_image_callback! {
    pub struct FaviconDownloadCallback {
        cef_id: i32,
        host: String,
    }

    impl DownloadImageCallback {
        fn on_download_image_finished(
            &self,
            _image_url: Option<&CefString>,
            _http_status_code: i32,
            image: Option<&mut Image>,
        ) {
            let Some(image) = image else { return };
            if image.is_empty() != 0 {
                return;
            }
            // Encode CEF's decoded image to PNG at 1x with transparency.
            let Some(png) = image.as_png(1.0, 1, None, None) else {
                return;
            };
            let size = png.size();
            if size == 0 {
                return;
            }
            let mut buf = vec![0u8; size];
            let written = png.data(Some(&mut buf), 0);
            buf.truncate(written);
            if !buf.is_empty() {
                history::store_favicon(&self.host, &buf);
                // Now that the PNG is cached, tell the host to display it.
                tabs::note_favicon(self.cef_id, &self.host);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{icon_format_score, pick_icon_url};

    #[test]
    fn prefers_ico_over_svg() {
        let urls = vec![
            "https://github.githubassets.com/favicons/favicon.svg".into(),
            "https://github.com/favicon.ico".into(),
        ];
        assert_eq!(
            pick_icon_url(&urls, "github.com").as_deref(),
            Some("https://github.com/favicon.ico")
        );
    }

    #[test]
    fn svg_only_falls_back_to_root_ico() {
        let urls = vec!["https://github.githubassets.com/favicons/favicon.svg".into()];
        assert_eq!(
            pick_icon_url(&urls, "github.com").as_deref(),
            Some("https://github.com/favicon.ico")
        );
    }

    #[test]
    fn png_beats_extensionless_before_root_ico() {
        let urls = vec![
            "https://cdn.example.com/favicon".into(),
            "https://cdn.example.com/icon.png".into(),
        ];
        assert_eq!(
            pick_icon_url(&urls, "example.com").as_deref(),
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
}
