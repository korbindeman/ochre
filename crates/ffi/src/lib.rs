//! C ABI bridge between the Rust core and native UI hosts (the macOS app).
//!
//! Built as a `cdylib` (`libbrowser_ffi.dylib`) that the SwiftUI app links
//! against and calls through a bridging header. The functions here are a thin,
//! `extern "C"` surface over [`browser_core::embed`] - the embedded-hosting
//! lifecycle where the native app owns the window and hands CEF a child NSView.
//!
//! Architecture spec §17/§18: FFI is the single bridge from core to native UIs.

use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;

/// Re-export core so Rust FFI consumers reach it through a single crate.
pub use browser_core;

/// Surface the core version to native callers.
pub fn core_version() -> &'static str {
    browser_core::CORE_VERSION
}

/// Initialize CEF for embedded hosting. Returns `true` on success.
///
/// Call once, on the main thread, before creating any window.
#[no_mangle]
pub extern "C" fn browser_initialize() -> bool {
    browser_core::embed::initialize()
}

/// Write a crash payload for an uncaught Swift/ObjC exception. Call from
/// `NSSetUncaughtExceptionHandler` before the process terminates.
///
/// # Safety
/// `name` and `reason` must be null or valid, NUL-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn browser_crash_report_swift_exception(
    name: *const c_char,
    reason: *const c_char,
) {
    let name = if name.is_null() {
        "NSException"
    } else {
        CStr::from_ptr(name).to_str().unwrap_or("NSException")
    };
    let reason = if reason.is_null() {
        ""
    } else {
        CStr::from_ptr(reason).to_str().unwrap_or("")
    };
    browser_core::crash_report::report_swift_exception(name, reason);
}

// ─── Tabs: lifecycle commands + the event stream ─────────────────────────

/// Register (or clear, with null) the tab/space event callback. It is invoked
/// on the main thread with `(event_type, id, payload)`; `event_type` selects the
/// event, `id` names the tab (or space), and `payload` is a nullable UTF-8 C
/// string whose meaning is per-event (see the bridging header). The core owns
/// `payload` - copy out during the call, don't retain it.
#[no_mangle]
pub extern "C" fn browser_set_event_callback(callback: Option<browser_core::tabs::EventCallback>) {
    browser_core::embed::set_event_callback(callback);
}

/// Create a new tab rendering into `parent_view` (an `NSView*`) at the given
/// size, loading `url`. Returns the new tab's id (`>= 0`), or a negative value
/// on failure. The tab becomes active; `TabCreated` + `TabSelected` are emitted.
///
/// # Safety
/// `parent_view` must be a valid `NSView*` that outlives the tab; `url` must be
/// a valid, NUL-terminated UTF-8 C string (or null for a blank page).
#[no_mangle]
pub unsafe extern "C" fn browser_tab_create(
    parent_view: *mut c_void,
    width: i32,
    height: i32,
    url: *const c_char,
) -> i32 {
    let url = if url.is_null() {
        ""
    } else {
        CStr::from_ptr(url).to_str().unwrap_or("")
    };
    browser_core::embed::create_tab(parent_view, width, height, url)
}

/// Hydrate a suspended tab restored from the persisted session (ADR-0013):
/// create its CEF browser as a child of `parent_view`, loading the tab's saved
/// URL. Returns `false` when `id` is not a suspended tab or creation fails.
/// Call on the main thread.
///
/// # Safety
/// `parent_view` must be a valid `NSView*` that outlives the tab.
#[no_mangle]
pub unsafe extern "C" fn browser_tab_restore(
    id: i32,
    parent_view: *mut c_void,
    width: i32,
    height: i32,
) -> bool {
    browser_core::embed::restore_tab(id, parent_view, width, height)
}

/// Make tab `id` the active tab (emits `TabSelected`). Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_tab_select(id: i32) {
    browser_core::embed::select_tab(id);
}

/// Hide an active saved tab (minus affordance) without removing it from the
/// saved section. Emits `StructureChanged` and `TabSuspended` when a browser
/// was loaded. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_tab_hide_saved(id: i32) {
    browser_core::embed::hide_saved_tab(id);
}

/// Permanently delete tab `id` (context-menu path). Emits `StructureChanged`.
/// Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_tab_delete(id: i32) {
    browser_core::embed::delete_tab(id);
}

/// Close tab `id`. Emits `StructureChanged`. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_tab_close(id: i32) {
    browser_core::embed::close_tab(id);
}

/// Reopen the most recently closed tab (⌘⇧T), restoring it into its original
/// space, folder, pinned state, and slot. Returns the reopened tab's id, or a
/// negative value when the closed-tab stack is empty. Emits `StructureChanged`
/// (the host hydrates the tab as the new active tab). Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_tab_reopen_closed() -> i32 {
    browser_core::embed::reopen_closed_tab()
}

/// Whether a closed tab is available to reopen - for the "Reopen Closed Tab"
/// menu item's enabled state. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_tab_can_reopen_closed() -> bool {
    browser_core::embed::can_reopen_closed_tab()
}

/// Read a nullable C string into an owned `String` ("" for null / invalid).
///
/// # Safety
/// `ptr` must be null or a valid, NUL-terminated UTF-8 C string.
unsafe fn cstr(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        CStr::from_ptr(ptr).to_str().unwrap_or("").to_string()
    }
}

/// Like [`cstr`], but for a secret (the master password): the copy is wrapped in
/// `Zeroizing` so its plaintext is wiped from heap on drop rather than left in
/// freed memory. Bind it to a `let` (so it lives across the call) and pass
/// `&value`, which derefs to `&str`.
///
/// # Safety
/// `ptr` must be null or a valid, NUL-terminated UTF-8 C string.
unsafe fn cstr_secret(ptr: *const c_char) -> zeroize::Zeroizing<String> {
    zeroize::Zeroizing::new(cstr(ptr))
}

/// The current structure as a JSON snapshot (ADR-0005) - the host rebuilds its
/// tree from this on `StructureChanged`. Shape:
/// `{"active_space":i,"active_tab":i|null,"spaces":[{"id","name"}],
///   "folders":[{"id","space","name","parent":i|null,"expanded"}],
///   "tabs":[{"id","space","folder":i|null,"pinned","favorite",
///             "saved_url":s|null,"url","title","loading"}]}`.
/// Heap-allocated by Rust - free with [`browser_string_free`].
#[no_mangle]
pub extern "C" fn browser_structure_json() -> *mut c_char {
    let json = serde_json::to_string(&browser_core::tabs::snapshot()).unwrap_or_default();
    CString::new(json)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Pin (persist) or unpin tab `id`. Unpinning returns it to "today" and clears
/// its folder. Emits `StructureChanged`. Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_set_pinned(id: i32, pinned: bool) {
    browser_core::embed::set_tab_pinned(id, pinned);
}

/// Pin or unpin tab `id` in the Arc-style icon strip (ADR-0032). Emits
/// `StructureChanged`. Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_set_favorite(id: i32, favorite: bool) {
    browser_core::embed::set_tab_favorite(id, favorite);
}

/// Expand or collapse a Space's Pinned section. Emits `StructureChanged`.
/// Main thread only.
#[no_mangle]
pub extern "C" fn browser_space_set_pinned_expanded(id: i32, expanded: bool) {
    browser_core::embed::set_space_pinned_expanded(id, expanded);
}

/// Replace a saved tab/favorite's pinned URL with its current live URL. This is
/// the only operation that makes temporary navigation durable. Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_replace_pinned_url(id: i32) {
    browser_core::embed::replace_tab_pinned_url(id);
}

/// Move tab `id` into folder `folder` (negative ⇒ out of any folder). Moving
/// into a folder auto-pins the tab. Emits `StructureChanged`. Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_move_to_folder(id: i32, folder: i32) {
    browser_core::embed::move_tab_to_folder(id, folder);
}

/// Relocate tab `id` for drag-and-drop: set its section (`favorite`, `pinned`,
/// and `folder` - negative ⇒ no folder) and position it just before tab `before`
/// (negative ⇒ the section's end). A folder destination auto-pins. Emits
/// `StructureChanged`. Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_move(
    id: i32,
    favorite: bool,
    pinned: bool,
    folder: i32,
    before: i32,
) {
    browser_core::embed::move_tab(id, favorite, pinned, folder, before);
}

/// Create an empty folder (named `name`) in the active space; returns its id.
///
/// # Safety
/// `name` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_create_folder(name: *const c_char) -> i32 {
    browser_core::embed::create_folder(&cstr(name))
}

/// Create a folder in the active space and move `tab_ids` (length `count`) into
/// it (auto-pins). Unknown ids are skipped. Returns the new folder id.
///
/// # Safety
/// `name` must be null or a valid, NUL-terminated UTF-8 C string. When
/// `count > 0`, `tab_ids` must point at `count` valid `i32`s.
#[no_mangle]
pub unsafe extern "C" fn browser_create_folder_with_tabs(
    name: *const c_char,
    tab_ids: *const i32,
    count: i32,
) -> i32 {
    let ids = if count <= 0 || tab_ids.is_null() {
        &[][..]
    } else {
        std::slice::from_raw_parts(tab_ids, count as usize)
    };
    browser_core::embed::create_folder_with_tabs(&cstr(name), ids)
}

/// Close ephemeral ("today") tabs in the active space except those listed in
/// `keep_ids` (length `count`). When `count` is 0, the active tab is kept.
/// Bulk close - not reopenable. Main thread only.
///
/// # Safety
/// When `count > 0`, `keep_ids` must point at `count` valid `i32`s.
#[no_mangle]
pub unsafe extern "C" fn browser_clear_ephemeral_tabs(keep_ids: *const i32, count: i32) {
    let keep = if count <= 0 || keep_ids.is_null() {
        &[][..]
    } else {
        std::slice::from_raw_parts(keep_ids, count as usize)
    };
    browser_core::embed::clear_ephemeral_tabs(keep)
}

/// Delete folder `id`, keeping its tabs (they drop to loose pinned). Main thread.
#[no_mangle]
pub extern "C" fn browser_delete_folder(id: i32) {
    browser_core::embed::delete_folder(id);
}

/// Rename folder `id`.
///
/// # Safety
/// `name` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_rename_folder(id: i32, name: *const c_char) {
    browser_core::embed::rename_folder(id, &cstr(name));
}

/// Set folder `id`'s sidebar expand/collapse state - persisted, so it survives
/// relaunch and space switches (ADR-0039). Emits `StructureChanged`. Main
/// thread only.
#[no_mangle]
pub extern "C" fn browser_set_folder_expanded(id: i32, expanded: bool) {
    browser_core::embed::set_folder_expanded(id, expanded);
}

/// Create a new empty space (named `name`, with emoji `icon`) and switch to it;
/// returns its id.
///
/// # Safety
/// `name` and `icon` must each be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_create_space(name: *const c_char, icon: *const c_char) -> i32 {
    browser_core::embed::create_space(&cstr(name), &cstr(icon))
}

/// Switch to space `id` (restores its remembered active tab). Main thread only.
#[no_mangle]
pub extern "C" fn browser_select_space(id: i32) {
    browser_core::embed::select_space(id);
}

/// Select a tab by 1-based sidebar index (`1`…`8`, `9` = last) in the active
/// space. Returns whether a tab was selected. Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_select_by_index(index: i32) -> bool {
    if !(1..=9).contains(&index) {
        return false;
    }
    browser_core::embed::select_tab_by_index(index as u8)
}

/// Select the next (`delta = 1`) or previous (`delta = -1`) tab in sidebar
/// order. Returns whether a tab was selected. Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_select_adjacent(delta: i32) -> bool {
    browser_core::embed::select_adjacent_tab(delta)
}

/// Select the next (`delta = 1`) or previous (`delta = -1`) space. Returns
/// whether a space was selected. Main thread only.
#[no_mangle]
pub extern "C" fn browser_space_select_adjacent(delta: i32) -> bool {
    browser_core::embed::select_adjacent_space(delta)
}

/// Move the visible tab up (`delta = -1`) or down (`delta = 1`) in sidebar
/// order. Returns whether the tab moved. Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_move_visible(delta: i32) -> bool {
    browser_core::embed::move_visible_tab(delta)
}

/// Rename space `id`.
///
/// # Safety
/// `name` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_rename_space(id: i32, name: *const c_char) {
    browser_core::embed::rename_space(id, &cstr(name));
}

/// Set space `id`'s emoji icon (shown in the space switcher).
///
/// # Safety
/// `icon` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_set_space_icon(id: i32, icon: *const c_char) {
    browser_core::embed::set_space_icon(id, &cstr(icon));
}

/// Delete space `id` and close its tabs (refused for the last space). Main thread.
#[no_mangle]
pub extern "C" fn browser_delete_space(id: i32) {
    browser_core::embed::delete_space(id);
}

/// Promote folder `id` to its own space (its tabs move with it); returns the new
/// space id. Main thread only.
#[no_mangle]
pub extern "C" fn browser_folder_to_space(id: i32) -> i32 {
    browser_core::embed::folder_to_space(id)
}

/// Demote space `id` to a folder in the active space (its pinned tabs move in,
/// its ephemeral tabs are closed); returns the new folder id. Main thread only.
#[no_mangle]
pub extern "C" fn browser_space_to_folder(id: i32) -> i32 {
    browser_core::embed::space_to_folder(id)
}

/// Default Arc `StorableSidebar.json` path on macOS. Free with
/// [`browser_string_free`]; null when unavailable.
#[no_mangle]
pub extern "C" fn browser_import_arc_default_path() -> *mut c_char {
    match browser_core::embed::arc_sidebar_default_path() {
        Some(path) => string_to_c(path),
        None => std::ptr::null_mut(),
    }
}

/// Preview an Arc sidebar import. JSON envelope - free with [`browser_string_free`].
/// Main thread only.
///
/// # Safety
/// `path` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_import_arc_preview(path: *const c_char) -> *mut c_char {
    import_json_envelope(
        browser_core::embed::import_arc_preview(&cstr(path)).map(|preview| {
            serde_json::json!({
                "spaces": preview.spaces,
                "folders": preview.folders,
                "tabs": preview.tabs,
                "favorites": preview.favorites,
            })
        }),
    )
}

/// Import an Arc sidebar (`StorableSidebar.json`). `replace` clears the existing
/// session first; otherwise the import merges. JSON envelope - free with
/// [`browser_string_free`]. Main thread only.
///
/// # Safety
/// `path` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_import_arc(path: *const c_char, replace: bool) -> *mut c_char {
    import_json_envelope(
        browser_core::embed::import_arc(&cstr(path), replace).map(|result| {
            serde_json::json!({
                "spaces": result.spaces,
                "folders": result.folders,
                "tabs": result.tabs,
                "favorites": result.favorites,
                "skipped_duplicates": result.skipped_duplicates,
            })
        }),
    )
}

fn import_json_envelope(result: Result<serde_json::Value, String>) -> *mut c_char {
    let value = match result {
        Ok(payload) => serde_json::json!({ "ok": true, "payload": payload }),
        Err(error) => serde_json::json!({ "ok": false, "error": error }),
    };
    string_to_c(value.to_string())
}

fn string_to_c(s: String) -> *mut c_char {
    CString::new(s)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Run the CEF message loop. Blocks until [`browser_quit`] / all browsers close.
#[no_mangle]
pub extern "C" fn browser_run_message_loop() {
    browser_core::embed::run_message_loop();
}

/// Ask CEF to exit its message loop.
#[no_mangle]
pub extern "C" fn browser_quit() {
    browser_core::embed::quit();
}

/// Load `url` in the primary browser's main frame. Call on the main thread.
///
/// # Safety
/// `url` must be a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_navigate(url: *const c_char) {
    if url.is_null() {
        return;
    }
    if let Ok(url) = CStr::from_ptr(url).to_str() {
        browser_core::embed::navigate(url);
    }
}

/// Navigate back one history entry. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_go_back() {
    browser_core::embed::go_back();
}

/// Navigate forward one history entry. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_go_forward() {
    browser_core::embed::go_forward();
}

/// Reload the current page. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_reload() {
    browser_core::embed::reload();
}

/// Return keyboard focus to the active tab's web view (no-op if none). The host
/// calls this when a transient overlay like the command bar closes, so the page
/// deterministically regains focus rather than relying on AppKit's first-
/// responder restoration. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_focus_active_tab() {
    browser_core::embed::focus_active_tab();
}

/// Reload ignoring the cache (hard reload). Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_reload_ignore_cache() {
    browser_core::embed::reload_ignore_cache();
}

/// Stop the current page's in-flight load (the reload button's cancel state).
/// Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_stop_load() {
    browser_core::embed::stop_load();
}

/// Whether the primary browser can navigate back / forward. Used by the host to
/// enable/disable its Back/Forward menu items. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_can_go_back() -> bool {
    browser_core::embed::can_go_back()
}

#[no_mangle]
pub extern "C" fn browser_can_go_forward() -> bool {
    browser_core::embed::can_go_forward()
}

/// The current page URL as a newly-allocated, NUL-terminated UTF-8 C string, or
/// null if unavailable. The caller **must** free it with [`browser_string_free`].
#[no_mangle]
pub extern "C" fn browser_get_url() -> *mut c_char {
    match browser_core::embed::current_url() {
        Some(url) => std::ffi::CString::new(url)
            .map(std::ffi::CString::into_raw)
            .unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

/// Free a string returned by [`browser_get_url`] or
/// [`browser_query_suggestions`].
///
/// # Safety
/// `ptr` must be null or a pointer previously returned by one of this library's
/// string-returning functions, and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn browser_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(std::ffi::CString::from_raw(ptr));
    }
}

/// Exit HTML5 / browser content fullscreen on the active tab. Call when the
/// host window leaves OS fullscreen while content is still fullscreen (Alloy).
/// Main thread only.
#[no_mangle]
pub extern "C" fn browser_exit_fullscreen() {
    browser_core::embed::exit_fullscreen();
}

/// Zoom the current page in / out / back to actual size. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_zoom_in() {
    browser_core::embed::zoom_in();
}

#[no_mangle]
pub extern "C" fn browser_zoom_out() {
    browser_core::embed::zoom_out();
}

#[no_mangle]
pub extern "C" fn browser_zoom_reset() {
    browser_core::embed::zoom_reset();
}

/// Toggle DevTools for the current page: open it in its own window, or close it
/// if already open. Backs the ⌘⌥I menu item (F12 is handled inside the core's
/// keyboard handler). DevTools opens as a separate window; inline docking isn't
/// feasible under CEF 149's Chrome bootstrap (ADR-0021). Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_toggle_dev_tools() {
    browser_core::embed::toggle_dev_tools();
}

/// Find `text` in the current page. `find_next` is false for a fresh search and
/// true when stepping to the next/previous match of the same term; `forward`
/// picks direction. Results arrive via the callback set with
/// [`browser_set_find_result_callback`]. Call on the main thread.
///
/// # Safety
/// `text` must be a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_find(
    text: *const c_char,
    forward: bool,
    match_case: bool,
    find_next: bool,
) {
    if text.is_null() {
        return;
    }
    if let Ok(text) = CStr::from_ptr(text).to_str() {
        browser_core::embed::find(text, forward, match_case, find_next);
    }
}

/// End the current find-in-page session. Call on the main thread.
#[no_mangle]
pub extern "C" fn browser_stop_finding(clear_selection: bool) {
    browser_core::embed::stop_finding(clear_selection);
}

/// Register (or clear, with null) the find-in-page result callback. It is
/// invoked on the main thread with the total match count, the 1-based active
/// match ordinal, and whether this is the final update for the search.
#[no_mangle]
pub extern "C" fn browser_set_find_result_callback(
    callback: Option<browser_core::embed::FindResultCallback>,
) {
    browser_core::embed::set_find_result_callback(callback);
}

/// Route a host window close through CEF's browser-close flow.
///
/// Returns `true` if a close was initiated - veto the window close and wait
/// for CEF to re-request it; the message loop quits once the last browser is
/// gone. Returns `false` if closing is already underway - let the window
/// close, and release the parent view so browser teardown can complete.
#[no_mangle]
pub extern "C" fn browser_request_close_all() -> bool {
    browser_core::embed::request_close_all()
}

/// Close one window's browsers. `window_id` `0` = normal, `>= 1` = private.
#[no_mangle]
pub extern "C" fn browser_request_close_window(window_id: i32) -> bool {
    browser_core::embed::request_close_window(window_id)
}

/// Tell the core which window owns nav/keyboard commands.
#[no_mangle]
pub extern "C" fn browser_set_focused_window(window_id: i32) {
    browser_core::embed::set_focused_window(window_id);
}

/// Allocate a new private window profile. Returns its id (`>= 1`).
#[no_mangle]
pub extern "C" fn browser_private_window_create() -> i32 {
    browser_core::embed::create_private_window()
}

/// JSON snapshot of a private window's tabs. Free with [`browser_string_free`].
#[no_mangle]
pub extern "C" fn browser_private_structure_json(window_id: i32) -> *mut c_char {
    let json = browser_core::embed::private_structure_json(window_id);
    CString::new(json)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Create a tab in private window `window_id`. Returns tab id or negative.
///
/// # Safety
/// `parent_view` and `url` must be valid per [`browser_tab_create`].
#[no_mangle]
pub unsafe extern "C" fn browser_private_tab_create(
    window_id: i32,
    parent_view: *mut c_void,
    width: i32,
    height: i32,
    url: *const c_char,
) -> i32 {
    let url = if url.is_null() {
        ""
    } else {
        CStr::from_ptr(url).to_str().unwrap_or("")
    };
    browser_core::embed::create_private_tab(window_id, parent_view, width, height, url)
}

#[no_mangle]
pub extern "C" fn browser_private_tab_select(window_id: i32, id: i32) {
    browser_core::embed::select_private_tab(window_id, id);
}

/// Select a tab by 1-based index (`1`…`8`, `9` = last) in a private window.
#[no_mangle]
pub extern "C" fn browser_private_tab_select_by_index(window_id: i32, index: i32) -> bool {
    if !(1..=9).contains(&index) {
        return false;
    }
    browser_core::embed::select_private_tab_by_index(window_id, index as u8)
}

/// Select the next (`delta = 1`) or previous (`delta = -1`) tab in a private
/// window.
#[no_mangle]
pub extern "C" fn browser_private_tab_select_adjacent(window_id: i32, delta: i32) -> bool {
    browser_core::embed::select_private_adjacent_tab(window_id, delta)
}

#[no_mangle]
pub extern "C" fn browser_private_tab_close(window_id: i32, id: i32) {
    browser_core::embed::close_private_tab(window_id, id);
}

#[no_mangle]
pub extern "C" fn browser_private_destroy_window(window_id: i32) {
    browser_core::embed::destroy_private_window(window_id);
}

#[no_mangle]
pub extern "C" fn browser_private_adblock_site_allowlisted(window_id: i32) -> bool {
    let host = browser_core::embed::active_tab_host();
    if host.is_empty() {
        return false;
    }
    browser_core::embed::private_is_site_allowlisted(window_id, &host)
}

#[no_mangle]
pub extern "C" fn browser_private_adblock_set_site_allowlisted(window_id: i32, allowlisted: bool) {
    let host = browser_core::embed::active_tab_host();
    if host.is_empty() {
        return;
    }
    browser_core::embed::private_set_site_allowlisted(window_id, &host, allowlisted);
}

#[no_mangle]
pub extern "C" fn browser_settings_private_vault_enabled() -> bool {
    browser_core::settings::private_vault_enabled()
}

#[no_mangle]
pub extern "C" fn browser_settings_set_private_vault_enabled(enabled: bool) {
    browser_core::settings::set_private_vault_enabled(enabled);
}

#[no_mangle]
pub extern "C" fn browser_settings_private_vault_prompt_seen() -> bool {
    browser_core::settings::private_vault_prompt_seen()
}

#[no_mangle]
pub extern "C" fn browser_settings_set_private_vault_prompt_seen(seen: bool) {
    browser_core::settings::set_private_vault_prompt_seen(seen);
}

// ─── Autocomplete: history suggestions + favicons ────────────────────────

/// Fuzzy-ranked history suggestions for `query`, as a JSON array string
/// (empty `[]` when nothing matches). Each object is
/// `{"url","title","host","displayUrl","titleMatches":[…],"urlMatches":[…]}`:
/// `displayUrl` is the pretty, matched-against URL (scheme + `www.` stripped),
/// and the two index arrays are the characters in `title` / `displayUrl` the
/// query matched, for highlighting. Ranking is host-prefix → host → URL → title
/// class, blended with frecency (see `browser_core::history::query`).
///
/// The returned pointer is heap-allocated by Rust; free it with
/// [`browser_string_free`]. Returns null only if the JSON contains an interior
/// NUL (it cannot - the fields come from CEF as valid strings).
///
/// # Safety
/// `query` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_query_suggestions(query: *const c_char) -> *mut c_char {
    let query = if query.is_null() {
        ""
    } else {
        CStr::from_ptr(query).to_str().unwrap_or("")
    };
    let suggestions = browser_core::history::query(query, 8);
    let json = serde_json::to_string(&suggestions).unwrap_or_else(|_| "[]".to_string());
    CString::new(json)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Fuzzy-match `query` against `text` (the omnibox scorer in
/// `browser_core::fuzzy`). Returns a heap-allocated JSON object
/// `{"score":…,"indices":[…]}` on success, or the literal `"null"` when there is
/// no match. Free with [`browser_string_free`].
///
/// # Safety
/// Both pointers must be null or valid, NUL-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn browser_fuzzy_match(
    query: *const c_char,
    text: *const c_char,
) -> *mut c_char {
    let query = if query.is_null() {
        ""
    } else {
        CStr::from_ptr(query).to_str().unwrap_or("")
    };
    let text = if text.is_null() {
        ""
    } else {
        CStr::from_ptr(text).to_str().unwrap_or("")
    };
    let json = match browser_core::fuzzy::fuzzy_match(query, text) {
        Some(m) => serde_json::json!({ "score": m.score, "indices": m.indices }).to_string(),
        None => "null".to_string(),
    };
    CString::new(json)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

// ─── History surface (the native History page) ───────────────────────────

/// History-page rows for `query`, as a JSON array string of
/// `{"url","title","host","last_visit","visit_count"}` objects, most-recent
/// first (empty `[]` when nothing matches). An empty `query` lists all history;
/// `limit`/`offset` paginate. Distinct from [`browser_query_suggestions`], which
/// ranks by frecency for autocomplete; this orders by recency for the page.
///
/// The returned pointer is heap-allocated by Rust; free it with
/// [`browser_string_free`]. Call on the main thread.
///
/// # Safety
/// `query` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_history_list(
    query: *const c_char,
    limit: u32,
    offset: u32,
) -> *mut c_char {
    let query = if query.is_null() {
        ""
    } else {
        CStr::from_ptr(query).to_str().unwrap_or("")
    };
    let entries = browser_core::history::list(query, limit, offset);
    let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
    CString::new(json)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Delete the history entry for `url`. Returns whether a row was removed. Call
/// on the main thread.
///
/// # Safety
/// `url` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_history_delete(url: *const c_char) -> bool {
    if url.is_null() {
        return false;
    }
    match CStr::from_ptr(url).to_str() {
        Ok(url) => browser_core::history::delete(url),
        Err(_) => false,
    }
}

/// Clear history entries last visited at or after `since` (Unix seconds); `0`
/// clears all history. Returns the number of rows removed. Call on the main
/// thread.
#[no_mangle]
pub extern "C" fn browser_history_clear(since: i64) -> u64 {
    browser_core::history::clear_since(since) as u64
}

/// Captured favicon PNG bytes for `host`, or null if none is cached. On success
/// `*out_len` is set to the byte length; free the buffer with
/// [`browser_free_bytes`]. `*out_len` is set to 0 when null is returned.
///
/// # Safety
/// `host` must be a valid, NUL-terminated UTF-8 C string; `out_len` must be a
/// valid pointer to a `usize`.
#[no_mangle]
pub unsafe extern "C" fn browser_favicon_png(host: *const c_char, out_len: *mut usize) -> *mut u8 {
    if out_len.is_null() {
        return std::ptr::null_mut();
    }
    *out_len = 0;
    if host.is_null() {
        return std::ptr::null_mut();
    }
    let Ok(host) = CStr::from_ptr(host).to_str() else {
        return std::ptr::null_mut();
    };
    let Some(bytes) = browser_core::history::favicon(host) else {
        return std::ptr::null_mut();
    };
    // `into_boxed_slice` guarantees capacity == length, so `browser_free_bytes`
    // can reconstruct the exact allocation from (ptr, len).
    let mut boxed = bytes.into_boxed_slice();
    *out_len = boxed.len();
    let ptr = boxed.as_mut_ptr();
    std::mem::forget(boxed);
    ptr
}

/// Free a byte buffer returned by [`browser_favicon_png`].
///
/// # Safety
/// `ptr`/`len` must be exactly a `(pointer, length)` pair returned by
/// [`browser_favicon_png`], freed at most once.
#[no_mangle]
pub unsafe extern "C" fn browser_free_bytes(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let slice = std::slice::from_raw_parts_mut(ptr, len);
    drop(Box::from_raw(slice as *mut [u8]));
}

// --- Password vault (browser_vault_*, ADR-0016) ---
//
// Thin glue over `browser_core::vault::service` (which owns the one global
// `VaultManager`). Every returned string is heap-allocated by Rust and the caller
// **must** free it with [`browser_string_free`]. The fallible calls return a JSON
// envelope - `{"ok":true}` / `{"ok":true,"imported":N}` / `{"ok":false,"error":"…"}`.
// The networked/heavy calls (`unlock_*`, `import_*`) block: **call them off the
// main thread.**

/// JSON envelope for a fallible no-data op.
fn vault_ok_or_err(result: Result<(), String>) -> *mut c_char {
    let value = match result {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(error) => serde_json::json!({ "ok": false, "error": error }),
    };
    vault_string(value.to_string())
}

/// JSON envelope for an online unlock/setup attempt: `{"ok":true}` on success,
/// `{"ok":false,"two_factor":"email"}` when a second factor is needed (the UI
/// then re-submits with the code + that tag), or `{"ok":false,"error":"…"}`.
fn vault_unlock_envelope(outcome: browser_core::vault::service::UnlockOutcome) -> *mut c_char {
    use browser_core::vault::service::UnlockOutcome;
    let value = match outcome {
        UnlockOutcome::Unlocked => serde_json::json!({ "ok": true }),
        UnlockOutcome::TwoFactorRequired { provider_tag } => {
            serde_json::json!({ "ok": false, "two_factor": provider_tag })
        }
        UnlockOutcome::Failed(error) => serde_json::json!({ "ok": false, "error": error }),
    };
    vault_string(value.to_string())
}

/// JSON envelope for an import (returns a count on success).
fn vault_count_or_err(result: Result<usize, String>) -> *mut c_char {
    let value = match result {
        Ok(count) => serde_json::json!({ "ok": true, "imported": count }),
        Err(error) => serde_json::json!({ "ok": false, "error": error }),
    };
    vault_string(value.to_string())
}

fn vault_string(s: String) -> *mut c_char {
    CString::new(s)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Initialize the global vault for `server_url` (empty ⇒ the default Bitwarden
/// cloud; otherwise a self-hosted base URL), caching under `cache_dir`.
///
/// # Safety
/// Both pointers must be null or valid NUL-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_init(server_url: *const c_char, cache_dir: *const c_char) {
    browser_core::vault::service::init(&cstr(server_url), &cstr(cache_dir));
}

/// True when the vault is locked or was never initialized.
#[no_mangle]
pub extern "C" fn browser_vault_is_locked() -> bool {
    browser_core::vault::service::is_locked()
}

/// Whether the vault has been set up on this machine (a server/account is
/// configured) - the UI shows the setup screen when this is false.
#[no_mangle]
pub extern "C" fn browser_vault_is_configured() -> bool {
    browser_core::vault::service::is_configured()
}

/// First-run setup: point the vault at `server` (**empty ⇒ the default Bitwarden
/// cloud**; otherwise a self-hosted base URL), log in with `email`/`password`,
/// and (on success) persist the config so later launches auto-init. `code` empty
/// ⇒ first attempt; a `{"two_factor":"…"}` envelope means re-call with the code
/// and that same `provider` tag. Blocks - call off the main thread. JSON
/// envelope; free with [`browser_string_free`].
///
/// # Safety
/// All pointers must be null or valid NUL-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_setup(
    server: *const c_char,
    email: *const c_char,
    password: *const c_char,
    code: *const c_char,
    provider: *const c_char,
) -> *mut c_char {
    vault_unlock_envelope(browser_core::vault::service::setup(
        &cstr(server),
        &cstr(email),
        &cstr_secret(password),
        &cstr(code),
        &cstr(provider),
    ))
}

/// Online unlock. `code` empty ⇒ no second factor supplied yet; a
/// `{"two_factor":"…"}` envelope means re-call with the code + that `provider`
/// tag. Blocks - call off the main thread. JSON envelope; free with
/// [`browser_string_free`].
///
/// # Safety
/// The pointers must be null or valid NUL-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_unlock_online(
    email: *const c_char,
    password: *const c_char,
    code: *const c_char,
    provider: *const c_char,
) -> *mut c_char {
    vault_unlock_envelope(browser_core::vault::service::unlock_online(
        &cstr(email),
        &cstr_secret(password),
        &cstr(code),
        &cstr(provider),
    ))
}

/// Re-dispatch the login two-factor email (the "Resend code" affordance). Blocks
/// (network + KDF) - call off the main thread. JSON envelope; free with
/// [`browser_string_free`].
///
/// # Safety
/// The pointers must be null or valid NUL-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_send_email_code(
    email: *const c_char,
    password: *const c_char,
) -> *mut c_char {
    vault_ok_or_err(browser_core::vault::service::send_email_code(
        &cstr(email),
        &cstr_secret(password),
    ))
}

/// Offline unlock from cache. Blocks (KDF) - call off the main thread.
///
/// # Safety
/// `password` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_unlock_offline(password: *const c_char) -> *mut c_char {
    vault_ok_or_err(browser_core::vault::service::unlock_offline(&cstr_secret(
        password,
    )))
}

/// Lock the vault (wipes the in-memory key).
#[no_mangle]
pub extern "C" fn browser_vault_lock() {
    browser_core::vault::service::lock();
}

/// Logins matching `host` as a JSON array (metadata only, no passwords). Empty
/// array when locked. Free with [`browser_string_free`].
///
/// # Safety
/// `host` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_lookup(host: *const c_char) -> *mut c_char {
    let views = browser_core::vault::service::lookup(&cstr(host));
    vault_string(serde_json::to_string(&views).unwrap_or_else(|_| "[]".to_string()))
}

/// Logins matching a free-text `query` across the **whole vault**, as a JSON
/// array (metadata only), best-first. An empty query lists everything - so the
/// filtered and unfiltered list states are one call (ADR-0049). Empty array when
/// locked. Free with [`browser_string_free`].
///
/// # Safety
/// `query` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_search(query: *const c_char) -> *mut c_char {
    let views = browser_core::vault::service::search(&cstr(query));
    vault_string(serde_json::to_string(&views).unwrap_or_else(|_| "[]".to_string()))
}

/// All logins as a JSON array (metadata only). Free with [`browser_string_free`].
#[no_mangle]
pub extern "C" fn browser_vault_list() -> *mut c_char {
    let views = browser_core::vault::service::list();
    vault_string(serde_json::to_string(&views).unwrap_or_else(|_| "[]".to_string()))
}

/// The password for login `id` - the only path a secret leaves the core, on an
/// explicit user gesture. Null when locked/unknown. Free with [`browser_string_free`].
///
/// # Safety
/// `id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_password(id: *const c_char) -> *mut c_char {
    if browser_core::private_window::focused_is_private()
        && !browser_core::settings::private_vault_enabled()
    {
        return std::ptr::null_mut();
    }
    match browser_core::vault::service::password_for(&cstr(id)) {
        Some(password) => vault_string(password),
        None => std::ptr::null_mut(),
    }
}

/// Full detail for login `id` as JSON (incl. password + notes) for the edit form
/// (a secret path, on an explicit gesture). Null when locked/unknown. Free with
/// [`browser_string_free`].
///
/// # Safety
/// `id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_login(id: *const c_char) -> *mut c_char {
    match browser_core::vault::service::login_detail(&cstr(id)) {
        Some(detail) => vault_string(serde_json::to_string(&detail).unwrap_or_default()),
        None => std::ptr::null_mut(),
    }
}

/// Fill the active tab's login form with vault login `id` - the button-driven
/// autofill path (P2-34, ADR-0029). The credential enters the page on this
/// explicit gesture; a lenient fill targets the current login fields (an email-
/// only first step included). No-op when locked/unknown or no active tab. Call on
/// the main thread.
///
/// # Safety
/// `id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_fill_active_tab(id: *const c_char) {
    browser_core::embed::vault_fill_active_tab(&cstr(id));
}

/// Record that login `id` was just used, so it leads its site's list next time
/// (ADR-0048). For the gestures the **host** owns - copying the username or
/// password out of the URL-bar popover; the core-side fill/picker paths record
/// themselves. Not for merely revealing an entry on the Passwords page. Use
/// inside a private window is ignored by the core.
///
/// # Safety
/// `id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_note_used(id: *const c_char) {
    browser_core::vault::service::note_used(&cstr(id));
}

/// Import a CSV export. Blocks - call off the main thread. JSON envelope with an
/// `imported` count; free with [`browser_string_free`].
///
/// # Safety
/// `data` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_import_csv(data: *const c_char) -> *mut c_char {
    vault_count_or_err(browser_core::vault::service::import_csv(&cstr(data)))
}

/// Import a Bitwarden JSON export. Blocks - call off the main thread. JSON
/// envelope with an `imported` count; free with [`browser_string_free`].
///
/// # Safety
/// `data` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_import_bitwarden_json(data: *const c_char) -> *mut c_char {
    vault_count_or_err(browser_core::vault::service::import_bitwarden_json(&cstr(
        data,
    )))
}

/// Create a new login. Blocks - call off the main thread. JSON envelope.
///
/// # Safety
/// All pointers must be null or valid NUL-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_add_login(
    name: *const c_char,
    username: *const c_char,
    password: *const c_char,
    uri: *const c_char,
    totp: *const c_char,
    notes: *const c_char,
) -> *mut c_char {
    vault_ok_or_err(browser_core::vault::service::add_login(
        &cstr(name),
        &cstr(username),
        &cstr(password),
        &cstr(uri),
        &cstr(totp),
        &cstr(notes),
    ))
}

/// Update login `id`. Blocks - call off the main thread. JSON envelope.
///
/// # Safety
/// All pointers must be null or valid NUL-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_update_login(
    id: *const c_char,
    name: *const c_char,
    username: *const c_char,
    password: *const c_char,
    uri: *const c_char,
    totp: *const c_char,
    notes: *const c_char,
) -> *mut c_char {
    vault_ok_or_err(browser_core::vault::service::update_login(
        &cstr(id),
        &cstr(name),
        &cstr(username),
        &cstr(password),
        &cstr(uri),
        &cstr(totp),
        &cstr(notes),
    ))
}

/// Delete login `id`. Blocks - call off the main thread. JSON envelope.
///
/// # Safety
/// `id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_vault_delete_login(id: *const c_char) -> *mut c_char {
    vault_ok_or_err(browser_core::vault::service::delete_login(&cstr(id)))
}

// --- CDP automation surface (P0-4) ---

/// Enable/disable the consent-gated CDP surface. Persisted; **takes effect on the
/// next launch** (the debugging port is a Chromium startup switch).
#[no_mangle]
pub extern "C" fn browser_cdp_set_enabled(enabled: bool) {
    browser_core::cdp::set_enabled(enabled);
}

/// Whether CDP is enabled (the persisted choice / what the next launch will do).
#[no_mangle]
pub extern "C" fn browser_cdp_is_enabled() -> bool {
    browser_core::cdp::is_enabled()
}

// --- Adblock (ADR-0020, spec §11) ---

/// Whether ad-blocking is globally enabled (on by default).
#[no_mangle]
pub extern "C" fn browser_adblock_is_enabled() -> bool {
    browser_core::adblock::is_enabled()
}

/// Toggle ad-blocking globally (persisted); reloads the active tab so it applies
/// on screen. Main thread.
#[no_mangle]
pub extern "C" fn browser_adblock_set_enabled(enabled: bool) {
    browser_core::embed::adblock_set_enabled(enabled);
}

/// Count of requests blocked on the active tab since its last navigation - the
/// sidebar shield's badge. Main thread.
#[no_mangle]
pub extern "C" fn browser_adblock_active_blocked_count() -> u64 {
    browser_core::embed::adblock_active_blocked_count()
}

/// Whether blocking is turned **off** for the active tab's site (the shield /
/// "Block Ads on This Site" toggle state). Main thread.
#[no_mangle]
pub extern "C" fn browser_adblock_active_site_allowlisted() -> bool {
    browser_core::embed::adblock_active_site_allowlisted()
}

/// Turn blocking off (`true`) / back on (`false`) for the active tab's site
/// (persisted); reloads the tab so it applies on screen. Main thread.
#[no_mangle]
pub extern "C" fn browser_adblock_set_active_site_allowlisted(allowlisted: bool) {
    browser_core::embed::adblock_set_active_site_allowlisted(allowlisted);
}

// --- Cookie-consent (ADR-0026) ---

/// Whether cookie-consent handling (reject + hide) is on (on by default).
#[no_mangle]
pub extern "C" fn browser_consent_is_enabled() -> bool {
    browser_core::consent::is_enabled()
}

/// Toggle cookie-consent handling (persisted); reloads the active tab so it
/// applies on screen. Main thread.
#[no_mangle]
pub extern "C" fn browser_consent_set_enabled(enabled: bool) {
    browser_core::embed::consent_set_enabled(enabled);
}

/// Count of consent banners auto-rejected on the active tab since its last
/// navigation - the shield popover's transparency line. Main thread.
#[no_mangle]
pub extern "C" fn browser_consent_active_rejected_count() -> u64 {
    browser_core::embed::consent_active_rejected_count()
}

// --- Browser settings ---

/// Vault auto-lock policy: 0 Never, 1 OnBrowserClose, 2 OnMacRestart, 3 OnPasswordUse.
#[no_mangle]
pub extern "C" fn browser_settings_vault_auto_lock() -> i32 {
    browser_core::settings::vault_auto_lock().to_i32()
}

/// Persist the vault auto-lock policy.
#[no_mangle]
pub extern "C" fn browser_settings_set_vault_auto_lock(policy: i32) {
    browser_core::settings::set_vault_auto_lock(browser_core::settings::VaultAutoLock::from_i32(
        policy,
    ));
}

/// Lock the vault on graceful browser shutdown when the policy requires it.
#[no_mangle]
pub extern "C" fn browser_settings_lock_vault_on_shutdown() {
    browser_core::vault::service::lock_on_shutdown();
}

/// Forward the Mac's current system boot time (Unix seconds). Drives the
/// OnMacRestart auto-lock policy.
#[no_mangle]
pub extern "C" fn browser_settings_set_system_boot_secs(secs: u64) {
    browser_core::settings::set_system_boot_secs(secs);
    browser_core::vault::service::check_mac_restart_lock();
}

/// Default search engine: 0 Google, 1 DuckDuckGo, 2 Bing, 3 Brave, 4 Startpage,
/// 5 Ecosia, 6 Kagi.
#[no_mangle]
pub extern "C" fn browser_settings_search_engine() -> i32 {
    browser_core::search_engine::engine().to_i32()
}

/// Persist the default search engine.
#[no_mangle]
pub extern "C" fn browser_settings_set_search_engine(engine: i32) {
    browser_core::search_engine::set_engine(browser_core::search_engine::SearchEngine::from_i32(
        engine,
    ));
}

/// Build a search-results URL for `query` using the selected engine. Returns a
/// heap-allocated UTF-8 string; free with [`browser_string_free`].
///
/// # Safety
/// `query` must be a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_search_url_for_query(query: *const c_char) -> *mut c_char {
    if query.is_null() {
        return std::ptr::null_mut();
    }
    let query = CStr::from_ptr(query).to_str().unwrap_or("");
    CString::new(browser_core::search_engine::search_url(query))
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

// --- Appearance / color scheme (ADR-0030) ---

/// The appearance preference the browser applies to web content and native
/// chrome: `0` System, `1` Light, `2` Dark. Read to drive the View → Appearance
/// menu checkmarks and `NSApp.appearance`.
#[no_mangle]
pub extern "C" fn browser_theme_preference() -> i32 {
    browser_core::theme::preference().to_i32()
}

/// Set the appearance preference (`0` System, `1` Light, `2` Dark); persisted and
/// applied live to open tabs via the global request context. Main thread.
#[no_mangle]
pub extern "C" fn browser_theme_set_preference(preference: i32) {
    browser_core::theme::set_preference(browser_core::theme::ThemePreference::from_i32(preference));
}

/// The host forwards the current OS appearance (launch + every OS theme change).
/// In System mode the resolved light/dark scheme follows this; ignored under a
/// fixed Light/Dark preference. Main thread.
#[no_mangle]
pub extern "C" fn browser_theme_set_system_dark(is_dark: bool) {
    browser_core::theme::set_system_dark(is_dark);
}

// --- Auto-update (ADR-0052, Zinnia model) ---

/// Check GitHub Releases for a newer version. `current_version` is the running
/// app's marketing version (no required `v` prefix). Returns a heap-allocated
/// JSON object (`version`, `current_version`, `notes`, …) when an update is
/// available, null when up to date or on error (errors are logged). Free with
/// [`browser_string_free`].
///
/// Blocks on the network - call off the main thread.
///
/// # Safety
/// `current_version` must be a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_update_check(current_version: *const c_char) -> *mut c_char {
    let current = if current_version.is_null() {
        ""
    } else {
        CStr::from_ptr(current_version).to_str().unwrap_or("")
    };
    match browser_core::update::check(current) {
        Ok(Some(update)) => match serde_json::to_string(&update) {
            Ok(json) => CString::new(json)
                .map(CString::into_raw)
                .unwrap_or(std::ptr::null_mut()),
            Err(err) => {
                eprintln!("[update] serialize: {err}");
                std::ptr::null_mut()
            }
        },
        Ok(None) => std::ptr::null_mut(),
        Err(err) => {
            eprintln!("[update] check failed: {err}");
            std::ptr::null_mut()
        }
    }
}

/// Last update found by a prior [`browser_update_check`], as JSON, or null.
/// Free with [`browser_string_free`].
#[no_mangle]
pub extern "C" fn browser_update_last_available() -> *mut c_char {
    match browser_core::update::last_available() {
        Some(update) => match serde_json::to_string(&update) {
            Ok(json) => CString::new(json)
                .map(CString::into_raw)
                .unwrap_or(std::ptr::null_mut()),
            Err(_) => std::ptr::null_mut(),
        },
        None => std::ptr::null_mut(),
    }
}

/// Dismiss the cached available update (e.g. user closed the card).
#[no_mangle]
pub extern "C" fn browser_update_dismiss() {
    browser_core::update::clear_last_available();
}

/// Download, verify (minisign), stage, and spawn the replace-and-relaunch
/// helper for the last available update (or the JSON in `update_json` if non-null).
/// `app_bundle` is the path to the running `.app`. Returns true on success; the
/// host should then quit so the helper can swap the bundle and relaunch.
///
/// Blocks - call off the main thread.
///
/// # Safety
/// `app_bundle` must be a valid, NUL-terminated UTF-8 path. `update_json` may be
/// null (use last check) or a valid JSON string matching [`AvailableUpdate`].
#[no_mangle]
pub unsafe extern "C" fn browser_update_download_and_install(
    app_bundle: *const c_char,
    update_json: *const c_char,
) -> bool {
    if app_bundle.is_null() {
        eprintln!("[update] install: missing app_bundle");
        return false;
    }
    let app = CStr::from_ptr(app_bundle).to_str().unwrap_or("");
    if app.is_empty() {
        return false;
    }

    let update = if update_json.is_null() {
        browser_core::update::last_available()
    } else {
        let raw = CStr::from_ptr(update_json).to_str().unwrap_or("");
        serde_json::from_str(raw).ok()
    };
    let Some(update) = update else {
        eprintln!("[update] install: no available update");
        return false;
    };

    match browser_core::update::download_and_install(&update, std::path::Path::new(app)) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("[update] install failed: {err}");
            false
        }
    }
}

/// Progress of the in-flight download: writes `downloaded` and `total` (0 if
/// unknown). Safe to call from any thread while install runs.
///
/// # Safety
/// `downloaded` and `total` must be valid writable pointers.
#[no_mangle]
pub unsafe extern "C" fn browser_update_download_progress(downloaded: *mut u64, total: *mut u64) {
    let (d, t) = browser_core::update::download_progress();
    if !downloaded.is_null() {
        *downloaded = d;
    }
    if !total.is_null() {
        *total = t;
    }
}

/// Tear down CEF. Call after [`browser_run_message_loop`] returns.
#[no_mangle]
pub extern "C" fn browser_shutdown() {
    browser_core::embed::shutdown();
}
