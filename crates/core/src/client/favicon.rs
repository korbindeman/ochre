//! Favicon download and caching for the sidebar tab list.

use cef::*;

use crate::history;
use crate::tabs;

/// First entry of a CEF string list, read without consuming it (the list is
/// borrowed from CEF and freed after the callback returns). Mirrors the raw
/// `cef_string_list_*` access the `cef` crate uses in its own `Debug` impl.
pub(super) fn first_icon_url(list: &mut CefStringList) -> Option<String> {
    let raw: *mut sys::_cef_string_list_t = list.into();
    if raw.is_null() {
        return None;
    }
    unsafe {
        if sys::cef_string_list_size(raw) == 0 {
            return None;
        }
        let mut value = std::mem::zeroed();
        (sys::cef_string_list_value(raw, 0, &mut value) != 0)
            .then(|| CefString::from(std::ptr::from_ref(&value)).to_string())
    }
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
