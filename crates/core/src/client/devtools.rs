//! DevTools toggling and the page context menu (Inspect Element + open-in-new-tab).

use cef::*;

/// Custom context-menu command id for "Inspect Element". CEF reserves the
/// `MENU_ID_USER_FIRST..=MENU_ID_USER_LAST` range for application-defined items.
pub(super) const MENU_ID_INSPECT_ELEMENT: i32 = sys::cef_menu_id_t::MENU_ID_USER_FIRST as i32;

// Chrome content context-menu command ids (`cef_command_ids.h`). Alloy embed has
// no Chrome TabStripModel, so Chromium's default handlers for these no-op - the
// menu item appears, the click does nothing. We own them and route through
// `tabs::request_open_tab` (same path as `target="_blank"` / middle-click).
const IDC_CONTENT_CONTEXT_OPENLINKNEWTAB: i32 = 50100;
const IDC_CONTENT_CONTEXT_OPENLINKNEWWINDOW: i32 = 50101;
const IDC_CONTENT_CONTEXT_OPENIMAGENEWTAB: i32 = 50123;
const IDC_CONTENT_CONTEXT_OPEN_ORIGINAL_IMAGE_NEW_TAB: i32 = 50125;

/// Whether `command_id` is a Chrome "open in new tab/window" content-menu item
/// that we must handle ourselves under Alloy embed.
fn is_open_in_new_tab_command(command_id: i32) -> bool {
    matches!(
        command_id,
        IDC_CONTENT_CONTEXT_OPENLINKNEWTAB
            | IDC_CONTENT_CONTEXT_OPENLINKNEWWINDOW
            | IDC_CONTENT_CONTEXT_OPENIMAGENEWTAB
            | IDC_CONTENT_CONTEXT_OPEN_ORIGINAL_IMAGE_NEW_TAB
    )
}

/// URL the open-in-new-tab command should load: link URL for link items, image
/// source for image items. Empty when the params have no usable target.
fn open_target_url(params: &ContextMenuParams, command_id: i32) -> String {
    match command_id {
        IDC_CONTENT_CONTEXT_OPENIMAGENEWTAB | IDC_CONTENT_CONTEXT_OPEN_ORIGINAL_IMAGE_NEW_TAB => {
            CefString::from(&params.source_url()).to_string()
        }
        _ => {
            // Prefer the filtered link URL; fall back to the unfiltered form when
            // Chromium stripped the scheme (javascript:, etc. stay empty either way).
            let link = CefString::from(&params.link_url()).to_string();
            if link.is_empty() {
                CefString::from(&params.unfiltered_link_url()).to_string()
            } else {
                link
            }
        }
    }
}

/// Route a context-menu open-in-new-tab to the host tab path. Foreground (takes
/// focus), matching style §4. Private sources open in the same private window.
fn handle_open_in_new_tab(browser: &Browser, params: &ContextMenuParams, command_id: i32) {
    let url = open_target_url(params, command_id);
    if url.is_empty() {
        eprintln!("[client] open-in-new-tab: empty url command_id={command_id}");
        return;
    }
    let source_cef_id = browser.identifier();
    eprintln!("[client] open-in-new-tab command_id={command_id} url={url}");
    if crate::private_window::is_private_cef(source_cef_id) {
        crate::private_window::request_open_tab(source_cef_id, &url, true);
    } else {
        crate::tabs::request_open_tab(&url, true);
    }
}

/// Toggle DevTools for `browser`: close it if already open, otherwise open it in
/// its own window. Backs both the ⌘⌥I menu item (via `embed::toggle_dev_tools`)
/// and the F12 keyboard shortcut, so the two stay consistent.
///
/// DevTools opens as a **separate window**, not docked in-window: under CEF 149's
/// Chrome bootstrap a DevTools browser must be Chrome-style, but hosting one in a
/// child `NSView` forces Alloy style (which it rejects), and its own Chrome-managed
/// window ignores `CefWindowInfo` (so it can't be created hidden for view-adoption
/// either). Inline docking is therefore not feasible for our Alloy-style tabs - see
/// ADR-0021. UI thread only.
pub(crate) fn toggle_dev_tools(browser: &Browser) {
    let Some(host) = browser.host() else { return };
    if host.has_dev_tools() != 0 {
        host.close_dev_tools();
    } else {
        host.show_dev_tools(None, None, None, None);
    }
}

/// Open DevTools focused on the element at the given page coordinates - the
/// context-menu "Inspect Element" path. Unlike [`toggle_dev_tools`] this always
/// opens (or refocuses) DevTools; it never closes it. UI thread only.
pub(super) fn inspect_element_at(browser: &Browser, x: i32, y: i32) {
    if let Some(host) = browser.host() {
        host.show_dev_tools(None, None, None, Some(&Point { x, y }));
    }
}

// Page context menu:
// - Chrome's "Open Link in New Tab" (and image variants) are intercepted - Alloy
//   embed has no TabStrip, so Chromium's defaults no-op.
// - "Inspect Element" is appended (CEF never includes it on its own).
// The DevTools window itself is created with a null client, so it keeps CEF's
// own context menu rather than this one.
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
            // Only separate from existing items - skip a leading separator on the
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
            if command_id == MENU_ID_INSPECT_ELEMENT {
                if let (Some(browser), Some(params)) = (browser, params) {
                    inspect_element_at(browser, params.xcoord(), params.ycoord());
                }
                return 1;
            }
            if is_open_in_new_tab_command(command_id) {
                if let (Some(browser), Some(params)) = (browser, params) {
                    handle_open_in_new_tab(browser, params, command_id);
                }
                return 1;
            }
            0 // not ours - let CEF run its default command
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
                return 1; // handled - don't forward to the renderer
            }
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_new_tab_commands_are_recognized() {
        assert!(is_open_in_new_tab_command(
            IDC_CONTENT_CONTEXT_OPENLINKNEWTAB
        ));
        assert!(is_open_in_new_tab_command(
            IDC_CONTENT_CONTEXT_OPENLINKNEWWINDOW
        ));
        assert!(is_open_in_new_tab_command(
            IDC_CONTENT_CONTEXT_OPENIMAGENEWTAB
        ));
        assert!(is_open_in_new_tab_command(
            IDC_CONTENT_CONTEXT_OPEN_ORIGINAL_IMAGE_NEW_TAB
        ));
        // Inspect Element and unrelated Chrome ids are not open-in-new-tab.
        assert!(!is_open_in_new_tab_command(MENU_ID_INSPECT_ELEMENT));
        assert!(!is_open_in_new_tab_command(50104)); // COPYLINKLOCATION
        assert!(!is_open_in_new_tab_command(0));
    }
}
