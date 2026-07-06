//! Pure-Rust native shell — an AppKit `NSWindow` hosting the embedded browser.
//!
//! A spike proving the embedded ("Option B") hosting model without Swift: this
//! process owns the `NSWindow` (built directly with objc2/AppKit), hands its
//! content view to [`crate::embed::create_browser`], and lets CEF pump the run
//! loop. It drives the exact same lifecycle the SwiftUI app drives over FFI
//! (`initialize` → `create_browser` → `run_message_loop` → `shutdown`), minus
//! the FFI — so it isolates window-embedding issues from Xcode/Swift/bridging
//! ones.
//!
//! Run with `just run --native-shell [--url=...]`.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use crate::{embed, mac};

const DEFAULT_URL: &str = "https://www.google.com";
const CONTENT_WIDTH: f64 = 1200.0;
const CONTENT_HEIGHT: f64 = 800.0;

define_class!(
    // SAFETY:
    // - `NSView` is a documented subclassing target with no requirements
    //   beyond main-thread confinement, which is inherited.
    // - `ContainerView` has no ivars and does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "BrowserContainerView"]
    struct ContainerView;

    impl ContainerView {
        // CEF inserts its browser `NSView` as our subview at a fixed frame;
        // keep it glued to our bounds when the window resizes. AppKit calls
        // this on every size change (`autoresizesSubviews` is on by default).
        #[unsafe(method(resizeSubviewsWithOldSize:))]
        fn resize_subviews_with_old_size(&self, _old_size: NSSize) {
            let bounds = self.bounds();
            for subview in self.subviews() {
                subview.setFrame(bounds);
            }
        }
    }
);

define_class!(
    // SAFETY: plain `NSObject` subclass with no ivars and no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ShellWindowDelegate"]
    struct ShellWindowDelegate;

    unsafe impl NSObjectProtocol for ShellWindowDelegate {}

    unsafe impl NSWindowDelegate for ShellWindowDelegate {
        // Route the close button through CEF instead of letting AppKit tear
        // the window (and the browser's view) down under a live browser:
        // ask the handler to close every browser and veto the close. When the
        // last browser is gone, `BrowserHandler::on_before_close` quits the
        // message loop and the process exits, taking the window with it.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> Bool {
            Bool::new(!embed::request_close_all())
        }

        // CEF on macOS completes browser destruction when its host `NSView`
        // deallocates (the view's dealloc drives `WindowDestroyed` →
        // `on_before_close`). Closing the window does not do that by itself:
        // `run` retains the container view (and thus CEF's subview) past the
        // close. Detach the subviews here so the browser teardown finishes and
        // `on_before_close` can quit the message loop.
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, notification: &NSNotification) {
            let Some(object) = notification.object() else {
                return;
            };
            let Some(window) = object.downcast_ref::<NSWindow>() else {
                return;
            };
            let Some(content_view) = window.contentView() else {
                return;
            };
            for subview in content_view.subviews() {
                subview.removeFromSuperview();
            }
        }
    }
);

/// Initialize CEF, open a native window, embed a browser in it, and block on
/// the CEF message loop until the window is closed.
///
/// Must be called on the main thread, as the process entry point (before
/// anything else touches `NSApp`).
pub fn run() -> Result<(), &'static str> {
    let mtm = MainThreadMarker::new().ok_or("native shell must be started on the main thread")?;

    let url = std::env::args()
        .find_map(|arg| arg.strip_prefix("--url=").map(str::to_owned))
        .unwrap_or_else(|| DEFAULT_URL.to_owned());

    // Loads the CEF framework, installs the CEF-conformant NSApplication, and
    // initializes CEF — all before we create the window, matching the order
    // the SwiftUI host uses.
    if !embed::initialize() {
        return Err("failed to initialize CEF");
    }

    let content_rect = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(CONTENT_WIDTH, CONTENT_HEIGHT),
    );
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable;
    let window: Retained<NSWindow> = unsafe {
        msg_send![
            NSWindow::alloc(mtm),
            initWithContentRect: content_rect,
            styleMask: style,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };
    // We own the window via `Retained`; the AppKit default of also releasing
    // it on close would over-release.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("Browser (native shell)"));

    let container: Retained<ContainerView> =
        unsafe { msg_send![ContainerView::alloc(mtm), initWithFrame: content_rect] };
    window.setContentView(Some(&container));

    // `setDelegate` holds a weak reference, so keep the delegate alive on the
    // stack for the lifetime of the loop below.
    let delegate: Retained<ShellWindowDelegate> =
        unsafe { msg_send![ShellWindowDelegate::alloc(mtm), init] };
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    window.center();
    window.makeKeyAndOrderFront(None);
    mac::platform_show_window(None);

    let created = unsafe {
        embed::create_browser(
            Retained::as_ptr(&container) as *mut c_void,
            CONTENT_WIDTH as i32,
            CONTENT_HEIGHT as i32,
            &url,
        )
    };
    if !created {
        return Err("failed to create the embedded browser");
    }

    embed::run_message_loop();

    window.setDelegate(None);
    embed::shutdown();
    Ok(())
}
