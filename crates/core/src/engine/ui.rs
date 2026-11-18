//! Native UI that WebKit asks its host to supply: JavaScript dialogs, the file
//! picker, the load-error page, and downloads (saved to ~/Downloads, like
//! Safari's default, with the Dock's Downloads stack bouncing on finish).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAlert, NSModalResponse, NSOpenPanel, NSTextField, NSWorkspace};
use objc2_foundation::{
    NSArray, NSData, NSError, NSPoint, NSRect, NSSize, NSString, NSURLRequest, NSURLResponse, NSURL,
};
use objc2_web_kit::{WKDownload, WKDownloadDelegate, WKOpenPanelParameters, WKWebView};

const ALERT_FIRST_BUTTON: NSModalResponse = 1000;
const MODAL_OK: NSModalResponse = 1;

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn new_alert(mtm: MainThreadMarker, host: &str, message: &str) -> Retained<NSAlert> {
    let alert = NSAlert::new(mtm);
    let title = if host.is_empty() {
        "This page says".to_string()
    } else {
        format!("{host} says")
    };
    alert.setMessageText(&ns(&title));
    alert.setInformativeText(&ns(message));
    alert
}

/// Show `alert` as a sheet on the view's window (or modally without one) and
/// hand the button response to `done`.
fn present(view: &WKWebView, alert: &NSAlert, done: impl Fn(NSModalResponse) + 'static) {
    match view.window() {
        Some(window) => {
            let block = RcBlock::new(done);
            alert.beginSheetModalForWindow_completionHandler(&window, Some(&block));
        }
        None => done(alert.runModal()),
    }
}

pub(super) fn alert(view: &WKWebView, host: &str, message: &str, handler: &DynBlock<dyn Fn()>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return handler.call(());
    };
    let alert = new_alert(mtm, host, message);
    alert.addButtonWithTitle(&ns("OK"));
    let handler = handler.copy();
    present(view, &alert, move |_| handler.call(()));
}

pub(super) fn confirm(
    view: &WKWebView,
    host: &str,
    message: &str,
    handler: &DynBlock<dyn Fn(Bool)>,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        return handler.call((Bool::NO,));
    };
    let alert = new_alert(mtm, host, message);
    alert.addButtonWithTitle(&ns("OK"));
    alert.addButtonWithTitle(&ns("Cancel"));
    let handler = handler.copy();
    present(view, &alert, move |response| {
        handler.call((Bool::new(response == ALERT_FIRST_BUTTON),))
    });
}

pub(super) fn prompt(
    view: &WKWebView,
    host: &str,
    message: &str,
    default_text: &str,
    handler: &DynBlock<dyn Fn(*mut NSString)>,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        return handler.call((std::ptr::null_mut(),));
    };
    let alert = new_alert(mtm, host, message);
    alert.addButtonWithTitle(&ns("OK"));
    alert.addButtonWithTitle(&ns("Cancel"));
    let field = NSTextField::textFieldWithString(&ns(default_text), mtm);
    field.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(260.0, 24.0),
    ));
    alert.setAccessoryView(Some(&field));
    alert.window().setInitialFirstResponder(Some(&field));
    let handler = handler.copy();
    present(view, &alert, move |response| {
        if response == ALERT_FIRST_BUTTON {
            let value = field.stringValue();
            handler.call((Retained::as_ptr(&value).cast_mut(),));
        } else {
            handler.call((std::ptr::null_mut(),));
        }
    });
}

pub(super) fn open_panel(
    view: &WKWebView,
    parameters: &WKOpenPanelParameters,
    handler: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        return handler.call((std::ptr::null_mut(),));
    };
    let panel = NSOpenPanel::openPanel(mtm);
    // SAFETY: property reads on the parameters, main thread.
    unsafe {
        panel.setAllowsMultipleSelection(parameters.allowsMultipleSelection());
        panel.setCanChooseDirectories(parameters.allowsDirectories());
    }
    panel.setCanChooseFiles(true);
    let handler = handler.copy();
    let chosen = panel.clone();
    let done = RcBlock::new(move |response: NSModalResponse| {
        if response == MODAL_OK {
            let urls = chosen.URLs();
            handler.call((Retained::as_ptr(&urls).cast_mut(),));
        } else {
            handler.call((std::ptr::null_mut(),));
        }
    });
    match view.window() {
        Some(window) => panel.beginSheetModalForWindow_completionHandler(&window, &done),
        None => panel.beginWithCompletionHandler(&done),
    }
}

/// Hand a URL WebKit cannot show (mailto:, tel:, app links) to its system handler.
pub(super) fn open_externally(url: &str) {
    if let Some(url) = NSURL::URLWithString(&ns(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

// ─── Load errors ─────────────────────────────────────────────────────────────

/// Replace a failed main-frame load with a small error page, keeping the
/// failed URL in the address bar. Cancellations and policy hand-offs (a link
/// that became a download or a new tab) are not errors.
pub(super) fn show_error_page(view: &WKWebView, error: &NSError) {
    let domain = error.domain().to_string();
    let code = error.code();
    let benign = (domain == "NSURLErrorDomain" && code == -999)
        || (domain == "WebKitErrorDomain" && matches!(code, 102 | 204));
    if benign {
        return;
    }
    let info = error.userInfo();
    let failed: Option<Retained<NSURL>> =
        unsafe { msg_send![&*info, objectForKey: &*ns("NSErrorFailingURLKey")] };
    let Some(failed) = failed else { return };
    let url = failed
        .absoluteString()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let host = failed
        .host()
        .map(|s| s.to_string())
        .unwrap_or_else(|| url.clone());
    let html = error_page_html(&host, &error.localizedDescription().to_string(), &url);
    // SAFETY: main thread (delegate callback); macOS 12+ API.
    unsafe {
        view.loadSimulatedRequest_responseHTMLString(
            &NSURLRequest::requestWithURL(&failed),
            &ns(&html),
        );
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn error_page_html(host: &str, reason: &str, url: &str) -> String {
    let url_js = serde_json::to_string(url).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>{host}</title>
<meta name="color-scheme" content="light dark">
<style>
body{{font:13px -apple-system,system-ui;margin:0;height:100vh;display:flex;align-items:center;justify-content:center;color:CanvasText;background:Canvas}}
main{{max-width:420px;padding:24px}}
h1{{font-size:20px;font-weight:600;margin:0 0 8px}}
p{{margin:0 0 16px;opacity:.7;line-height:1.45}}
button{{font:inherit;padding:5px 14px;border-radius:6px;border:1px solid color-mix(in srgb,CanvasText 25%,transparent);background:transparent;color:inherit}}
</style></head><body><main>
<h1>Can't open {host_e}</h1><p>{reason_e}</p>
<button onclick="location.href={url_js}">Try Again</button>
</main></body></html>"#,
        host = html_escape(host),
        host_e = html_escape(host),
        reason_e = html_escape(reason),
    )
}

// ─── Downloads ───────────────────────────────────────────────────────────────

thread_local! {
    static DOWNLOAD_DELEGATE: RefCell<Option<Retained<DownloadDelegate>>> = const { RefCell::new(None) };
    /// Destination per in-flight download, keyed by the WKDownload pointer.
    static DESTINATIONS: RefCell<HashMap<usize, PathBuf>> = RefCell::new(HashMap::new());
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OchreDownloadDelegate"]
    struct DownloadDelegate;

    unsafe impl NSObjectProtocol for DownloadDelegate {}

    unsafe impl WKDownloadDelegate for DownloadDelegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn decide_destination(
            &self,
            download: &WKDownload,
            _response: &NSURLResponse,
            suggested: &NSString,
            handler: &DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            let path = unique_destination(&downloads_dir(), &suggested.to_string());
            DESTINATIONS.with(|d| {
                d.borrow_mut()
                    .insert(download as *const _ as usize, path.clone())
            });
            let url = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
            handler.call((Retained::as_ptr(&url).cast_mut(),));
        }

        #[unsafe(method(downloadDidFinish:))]
        fn did_finish(&self, download: &WKDownload) {
            if let Some(path) =
                DESTINATIONS.with(|d| d.borrow_mut().remove(&(download as *const _ as usize)))
            {
                eprintln!("[download] saved {}", path.display());
                bounce_downloads_stack(&path);
            }
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn did_fail(&self, download: &WKDownload, error: &NSError, _resume: Option<&NSData>) {
            DESTINATIONS.with(|d| d.borrow_mut().remove(&(download as *const _ as usize)));
            eprintln!("[download] failed: {}", error.localizedDescription());
        }
    }
);

pub(super) fn adopt_download(download: &WKDownload) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let delegate = DOWNLOAD_DELEGATE.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| unsafe { msg_send![DownloadDelegate::alloc(mtm), init] })
            .clone()
    });
    // SAFETY: main thread; the delegate lives in a thread-local for the process.
    unsafe { download.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
}

fn downloads_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new(&home).join("Downloads")
}

/// `name.ext`, or `name (2).ext` etc. when taken. Strips path separators.
pub(crate) fn unique_destination(dir: &Path, suggested: &str) -> PathBuf {
    let clean: String = suggested
        .chars()
        .map(|c| if c == '/' || c == ':' { '-' } else { c })
        .collect();
    let clean = clean.trim_start_matches('.').trim();
    let clean = if clean.is_empty() { "download" } else { clean };
    let candidate = dir.join(clean);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match clean.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (clean.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

fn bounce_downloads_stack(path: &Path) {
    let center: Retained<NSObject> = unsafe {
        msg_send![
            objc2::class!(NSDistributedNotificationCenter),
            defaultCenter
        ]
    };
    let name = ns("com.apple.DownloadFileFinished");
    let object = ns(&path.to_string_lossy());
    let _: () = unsafe { msg_send![&*center, postNotificationName: &*name, object: &*object] };
}

#[cfg(test)]
mod tests {
    use super::unique_destination;

    #[test]
    fn unique_destination_numbers_taken_names_and_strips_separators() {
        let dir = std::env::temp_dir().join(format!("ochre-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_destination(&dir, "report.pdf");
        assert_eq!(first, dir.join("report.pdf"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(
            unique_destination(&dir, "report.pdf"),
            dir.join("report (2).pdf")
        );
        assert_eq!(
            unique_destination(&dir, "../evil/x.txt"),
            dir.join("-evil-x.txt")
        );
        assert_eq!(unique_destination(&dir, ""), dir.join("download"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
