//! DevTools toggling and the page context menu's Inspect Element entry.

use cef::*;

/// Custom context-menu command id for "Inspect Element". CEF reserves the
/// `MENU_ID_USER_FIRST..=MENU_ID_USER_LAST` range for application-defined items.
pub(super) const MENU_ID_INSPECT_ELEMENT: i32 = sys::cef_menu_id_t::MENU_ID_USER_FIRST as i32;

/// Toggle DevTools for `browser`: close it if already open, otherwise open it in
/// its own window. Backs both the ⌘⌥I menu item (via `embed::toggle_dev_tools`)
/// and the F12 keyboard shortcut, so the two stay consistent.
///
/// DevTools opens as a **separate window**, not docked in-window: under CEF 149's
/// Chrome bootstrap a DevTools browser must be Chrome-style, but hosting one in a
/// child `NSView` forces Alloy style (which it rejects), and its own Chrome-managed
/// window ignores `CefWindowInfo` (so it can't be created hidden for view-adoption
/// either). Inline docking is therefore not feasible for our Alloy-style tabs — see
/// ADR-0021. UI thread only.
pub(crate) fn toggle_dev_tools(browser: &Browser) {
    let Some(host) = browser.host() else { return };
    if host.has_dev_tools() != 0 {
        host.close_dev_tools();
    } else {
        host.show_dev_tools(None, None, None, None);
    }
}

/// Open DevTools focused on the element at the given page coordinates — the
/// context-menu "Inspect Element" path. Unlike [`toggle_dev_tools`] this always
/// opens (or refocuses) DevTools; it never closes it. UI thread only.
pub(super) fn inspect_element_at(browser: &Browser, x: i32, y: i32) {
    if let Some(host) = browser.host() {
        host.show_dev_tools(None, None, None, Some(&Point { x, y }));
    }
}

// Adds "Inspect Element" to CEF's default page context menu and routes it to
// DevTools. The default model already carries navigation / editing / view-source
// items; we only append the inspect entry CEF never includes on its own. The
// DevTools window itself is created with a null client, so it keeps CEF's own
// context menu rather than this one.
wrap_context_menu_handler! {
    pub struct BrowserContextMenuHandler;

    impl ContextMenuHandler {
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            let Some(model) = model else { return };
            // Only separate from existing items — skip a leading separator on the
            // rare empty model.
            if model.count() > 0 {
                model.add_separator();
            }
            model.add_item(
                MENU_ID_INSPECT_ELEMENT,
                Some(&CefString::from("Inspect Element")),
            );
        }

        fn on_context_menu_command(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            command_id: i32,
            _event_flags: EventFlags,
        ) -> i32 {
            if command_id != MENU_ID_INSPECT_ELEMENT {
                return 0; // not ours — let CEF run its default command
            }
            if let (Some(browser), Some(params)) = (browser, params) {
                inspect_element_at(browser, params.xcoord(), params.ycoord());
            }
            1
        }
    }
}

// F12 toggles DevTools, matching Chrome. ⌘⌥I is handled by the native menu,
// which consumes the key equivalent before it reaches CEF, so this handler only
// needs the un-menued F12. Stateless.
wrap_keyboard_handler! {
    pub struct BrowserKeyboardHandler;

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: *mut u8,
            _is_keyboard_shortcut: Option<&mut i32>,
        ) -> i32 {
            let Some(event) = event else { return 0 };
            // Act on the raw key-down (once per press) so the page never sees F12.
            // `windows_key_code` is CEF's cross-platform VK code; VK_F12 = 0x7B.
            if event.type_ == KeyEventType::RAWKEYDOWN && event.windows_key_code == 0x7B {
                if let Some(browser) = browser {
                    toggle_dev_tools(browser);
                }
                return 1; // handled — don't forward to the renderer
            }
            0
        }
    }
}
