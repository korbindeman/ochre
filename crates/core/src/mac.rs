//! macOS platform integration for CEF.
//!
//! CEF on macOS has two hard platform requirements that this module satisfies:
//!
//! 1. **Framework loading.** The `Chromium Embedded Framework` is not linked;
//!    it is loaded at startup via `cef_load_library` from inside the `.app`
//!    bundle (`Contents/Frameworks`). [`load_cef_framework`] does this and
//!    returns a guard that unloads on drop.
//!
//! 2. **A CEF-conformant `NSApplication`.** Chromium brackets its own event
//!    dispatch with `-setHandlingSendEvent:` / `-isHandlingSendEvent` and sends
//!    those selectors to `NSApp`. A stock `NSApplication` does not implement
//!    them, so it would raise "unrecognized selector" and crash the moment an
//!    event is processed. [`setup_simple_application`] installs an
//!    `NSApplication` subclass ([`BrowserApplication`]) that conforms to CEF's
//!    `CefAppProtocol` (`CrAppProtocol` + `CrAppControlProtocol`) and overrides
//!    `-sendEvent:` to track the flag - the Rust equivalent of Chromium's
//!    `CefScopedSendingEvent`.
//!
//! These are invoked from [`crate::run`] before CEF is initialized.

use std::sync::atomic::{AtomicBool, Ordering};

use cef::application_mac::{CefAppProtocol, CrAppControlProtocol, CrAppProtocol};
use cef::{Browser, CefString};

use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::{define_class, msg_send, ClassType, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSEvent};
use objc2_foundation::NSString;

pub use cef::library_loader::LibraryLoader;

/// Whether `NSApp` is currently inside its own `-sendEvent:`.
///
/// `NSApp` is a process-wide singleton confined to the main thread, so a single
/// flag is the correct backing store. Using a module static (rather than an
/// Objective-C ivar) is also deliberate: the shared application is created by
/// `+sharedApplication`, which allocates and initializes the instance itself
/// and therefore bypasses objc2's ivar initialization - reading a custom ivar
/// off such an instance would be undefined behavior.
static HANDLING_SEND_EVENT: AtomicBool = AtomicBool::new(false);

/// Load the CEF framework from the app bundle and return the loader guard.
///
/// The guard unloads the framework on drop, so the caller must keep it alive
/// for the lifetime of the process.
pub fn load_cef_framework() -> LibraryLoader {
    // Helper (renderer/GPU/utility) processes are always launched with a
    // `--type=<role>` switch and live one bundle level deeper than the main
    // executable, so the loader resolves a different relative framework path
    // for them.
    let is_helper = std::env::args().any(|arg| arg.starts_with("--type="));
    let exe = std::env::current_exe().expect("failed to resolve current executable path");
    let loader = LibraryLoader::new(&exe, is_helper);
    assert!(
        loader.load(),
        "failed to load the Chromium Embedded Framework"
    );
    loader
}

define_class!(
    // SAFETY:
    // - `NSApplication` is the documented base for a custom principal class and
    //   imposes no subclassing requirements beyond main-thread confinement,
    //   which is inherited here.
    // - `BrowserApplication` has no ivars and does not implement `Drop`.
    #[unsafe(super(NSApplication))]
    #[name = "BrowserApplication"]
    struct BrowserApplication;

    impl BrowserApplication {
        #[unsafe(method(sendEvent:))]
        fn send_event(&self, event: &NSEvent) {
            let previous = HANDLING_SEND_EVENT.swap(true, Ordering::SeqCst);
            let _: () = unsafe { msg_send![super(self), sendEvent: event] };
            HANDLING_SEND_EVENT.store(previous, Ordering::SeqCst);
        }
    }

    unsafe impl CrAppProtocol for BrowserApplication {
        #[unsafe(method(isHandlingSendEvent))]
        fn is_handling_send_event(&self) -> Bool {
            Bool::new(HANDLING_SEND_EVENT.load(Ordering::SeqCst))
        }
    }

    unsafe impl CrAppControlProtocol for BrowserApplication {
        #[unsafe(method(setHandlingSendEvent:))]
        fn set_handling_send_event(&self, handling: Bool) {
            HANDLING_SEND_EVENT.store(handling.as_bool(), Ordering::SeqCst);
        }
    }

    unsafe impl CefAppProtocol for BrowserApplication {}
);

/// Install the CEF-conformant shared `NSApplication` (`NSApp`).
///
/// Must be called on the main thread, before CEF is initialized and before any
/// other code touches `NSApp`. `+sharedApplication` creates the singleton as an
/// instance of the receiver class, so sending it to [`BrowserApplication`]
/// makes `NSApp` conform to `CefAppProtocol`.
pub fn setup_simple_application() {
    let _mtm = MainThreadMarker::new()
        .expect("setup_simple_application must be called on the main thread");
    let _app: Retained<NSApplication> =
        unsafe { msg_send![BrowserApplication::class(), sharedApplication] };
}

/// Finish application setup after CEF initialization: bring the app to the
/// foreground so the newly created CEF window becomes key and visible.
///
/// A full `NSApplicationDelegate` (menu bar, `MainMenu.nib`, window-restoration
/// policy) is not required to render a window - CEF drives the run loop and
/// window lifecycle itself - so wiring one up is deferred to a later milestone.
pub fn setup_app_delegate() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}

/// Bring the browser's platform window to the front.
///
/// Only reached in the ALLOY + `--use-native` popup path; the default Views
/// path shows its `cef::Window` directly (see
/// [`BrowserHandler::show_main_window`](crate::client)). CEF orders its native
/// window front on creation, so here we only need to activate the app.
pub fn platform_show_window(_browser: Option<&mut Browser>) {
    if let Some(mtm) = MainThreadMarker::new() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

/// Set the title of the browser's platform window.
///
/// Only reached in the ALLOY + `--use-native` popup path. In single-window
/// native mode the CEF-created window is the application's main window.
pub fn platform_title_change(_browser: Option<&mut Browser>, title: Option<&CefString>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = NSApplication::sharedApplication(mtm).mainWindow() else {
        return;
    };
    let title = title.map(CefString::to_string).unwrap_or_default();
    window.setTitle(&NSString::from_str(&title));
}

/// True when this process's code signature carries a boolean entitlement.
///
/// Used at CEF command-line setup to decide whether Blink WebAuthentication
/// is safe to leave on (ADR-0061 / CRASH-0003). Ad-hoc signatures and
/// Developer ID signatures that lack Apple's managed web-browser passkey
/// capability both return false. `SecTaskCreateFromSelf` reads the *current*
/// executable, so callers must not ask this of a helper process when the
/// answer is about the host app.
pub fn has_bool_entitlement(name: &str) -> bool {
    let Ok(c_name) = std::ffi::CString::new(name) else {
        return false;
    };
    unsafe { task_has_bool_entitlement(c_name.as_ptr()) }
}

/// `com.apple.developer.web-browser.public-key-credential` - Apple's managed
/// capability that lets a browser use iCloud Keychain passkeys for any RP.
pub fn has_web_browser_passkey_entitlement() -> bool {
    has_bool_entitlement("com.apple.developer.web-browser.public-key-credential")
}

// Security.framework + CoreFoundation: read one entitlement off our own
// signature. Matches Chromium's `SecTaskCopyValueForEntitlement` probe.
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        alloc: *const std::ffi::c_void,
        c_str: *const std::ffi::c_char,
        encoding: u32,
    ) -> *const std::ffi::c_void;
    fn CFRelease(cf: *const std::ffi::c_void);
    fn CFGetTypeID(cf: *const std::ffi::c_void) -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(boolean: *const std::ffi::c_void) -> u8;
}

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecTaskCreateFromSelf(allocator: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn SecTaskCopyValueForEntitlement(
        task: *mut std::ffi::c_void,
        entitlement: *const std::ffi::c_void,
        error: *mut *const std::ffi::c_void,
    ) -> *const std::ffi::c_void;
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

// SAFETY: `c_name` is a valid NUL-terminated C string. All CF objects we
// create are released before return. Null returns from Security are treated
// as "entitlement absent," not as a panic.
unsafe fn task_has_bool_entitlement(c_name: *const std::ffi::c_char) -> bool {
    let task = unsafe { SecTaskCreateFromSelf(std::ptr::null()) };
    if task.is_null() {
        return false;
    }
    let key =
        unsafe { CFStringCreateWithCString(std::ptr::null(), c_name, K_CF_STRING_ENCODING_UTF8) };
    if key.is_null() {
        unsafe { CFRelease(task.cast()) };
        return false;
    }
    let mut err: *const std::ffi::c_void = std::ptr::null();
    let value = unsafe { SecTaskCopyValueForEntitlement(task, key, &mut err) };
    unsafe { CFRelease(key) };
    unsafe { CFRelease(task.cast()) };
    if !err.is_null() {
        unsafe { CFRelease(err) };
    }
    if value.is_null() {
        return false;
    }
    let is_true =
        unsafe { CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value) != 0 };
    unsafe { CFRelease(value) };
    is_true
}

#[cfg(test)]
mod tests {
    #[test]
    fn cargo_test_binary_lacks_web_browser_passkey_entitlement() {
        assert!(!super::has_web_browser_passkey_entitlement());
    }
}
