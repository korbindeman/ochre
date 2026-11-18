//! macOS platform integration.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

/// Bring the app to the foreground at launch so its first window becomes key.
pub fn setup_app_delegate() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}
