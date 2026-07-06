//! C ABI bridge between the Rust core and native UI hosts (the macOS app).
//!
//! Built as a `cdylib` (`libbrowser_ffi.dylib`) that the SwiftUI app links
//! against and calls through a bridging header. The functions here are a thin,
//! `extern "C"` surface over [`browser_core::embed`] — the embedded-hosting
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

// ─── Tabs: lifecycle commands + the event stream ─────────────────────────

/// Register (or clear, with null) the tab/space event callback. It is invoked
/// on the main thread with `(event_type, id, payload)`; `event_type` selects the
/// event, `id` names the tab (or space), and `payload` is a nullable UTF-8 C
/// string whose meaning is per-event (see the bridging header). The core owns
/// `payload` — copy out during the call, don't retain it.
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

/// Whether a closed tab is available to reopen — for the "Reopen Closed Tab"
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

/// The current structure as a JSON snapshot (ADR-0005) — the host rebuilds its
/// tree from this on `StructureChanged`. Shape:
/// `{"active_space":i,"active_tab":i|null,"spaces":[{"id","name"}],
///   "folders":[{"id","space","name"}],
///   "tabs":[{"id","space","folder":i|null,"pinned","url","title","loading"}]}`.
/// Heap-allocated by Rust — free with [`browser_string_free`].
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

/// Move tab `id` into folder `folder` (negative ⇒ out of any folder). Moving
/// into a folder auto-pins the tab. Emits `StructureChanged`. Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_move_to_folder(id: i32, folder: i32) {
    browser_core::embed::move_tab_to_folder(id, folder);
}

/// Relocate tab `id` for drag-and-drop: set its section (`pinned`, and `folder`
/// — negative ⇒ no folder) and position it just before tab `before` (negative ⇒
/// the section's end). A folder destination auto-pins. Emits `StructureChanged`.
/// Main thread only.
#[no_mangle]
pub extern "C" fn browser_tab_move(id: i32, pinned: bool, folder: i32, before: i32) {
    browser_core::embed::move_tab(id, pinned, folder, before);
}

/// Create an empty folder (named `name`) in the active space; returns its id.
///
/// # Safety
/// `name` must be null or a valid, NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn browser_create_folder(name: *const c_char) -> i32 {
    browser_core::embed::create_folder(&cstr(name))
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
/// keyboard handler). Call on the main thread.
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
/// Returns `true` if a close was initiated — veto the window close and wait
/// for CEF to re-request it; the message loop quits once the last browser is
/// gone. Returns `false` if closing is already underway — let the window
/// close, and release the parent view so browser teardown can complete.
#[no_mangle]
pub extern "C" fn browser_request_close_all() -> bool {
    browser_core::embed::request_close_all()
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
/// NUL (it cannot — the fields come from CEF as valid strings).
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

/// Tear down CEF. Call after [`browser_run_message_loop`] returns.
#[no_mangle]
pub extern "C" fn browser_shutdown() {
    browser_core::embed::shutdown();
}
