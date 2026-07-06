// Bridging header — exposes the Rust core's C ABI (libbrowser_ffi) to Swift.
// Mirrors the `#[no_mangle] extern "C"` functions in crates/ffi/src/lib.rs.

#ifndef BROWSER_BRIDGING_HEADER_H
#define BROWSER_BRIDGING_HEADER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Initialize CEF for embedded hosting. Returns true on success.
// Call once, on the main thread, before creating any window.
bool browser_initialize(void);

// ─── Tabs / folders / spaces: commands + the event stream ───────────────────
//
// The app owns one NSView per tab and drives the tree over these commands; the
// core pushes changes back over the event callback (ADR-0003/0005/0006).

// Event types delivered to the callback. Hot per-tab CONTENT is fine-grained
// (id = tab id); every STRUCTURAL change (create/close/select, pin, folder
// membership, space/folder CRUD, transforms) collapses to StructureChanged, on
// which the host re-pulls browser_structure_json (ADR-0005).
//   0  TabTitleChanged   payload = title
//   1  TabUrlChanged     payload = url
//   2  TabLoadStarted    payload = NULL
//   3  TabLoadEnded      payload = NULL
//   4  TabFaviconChanged payload = host (fetch the PNG via browser_favicon_png)
//   5  StructureChanged  no id / payload — re-pull browser_structure_json
//   6  TabNavStateChanged payload = "bf" — back/forward enablement as '0'/'1'

// Register (or clear, with NULL) the tab/space event callback. Invoked on the
// main thread. `payload` is owned by the core and valid only during the call.
void browser_set_event_callback(void (*callback)(int32_t event_type, int32_t id,
                                                 const char *payload));

// The whole tree as a JSON snapshot (see crates/ffi browser_structure_json).
// Heap-allocated by Rust — free with browser_string_free.
char *browser_structure_json(void);

// Create a new tab rendering into `parent_view` (NSView*) at the given size,
// loading `url` (NUL-terminated UTF-8, or NULL for blank). Returns the tab id
// (>= 0) or negative on failure. The tab becomes active (ephemeral / "today").
// Main thread only.
int32_t browser_tab_create(void *parent_view, int32_t width, int32_t height,
                           const char *url);

// Hydrate a suspended tab restored from the persisted session: create its CEF
// browser as a child of `parent_view`, loading the tab's saved URL. Returns
// false when `id` isn't a suspended tab or creation fails. Main thread only.
bool browser_tab_restore(int32_t id, void *parent_view, int32_t width,
                         int32_t height);

// Make tab `id` the active tab of its space. Main thread only.
void browser_tab_select(int32_t id);

// Close tab `id`. Main thread only.
void browser_tab_close(int32_t id);

// Reopen the most recently closed tab (Cmd+Shift+T): restore it into its
// original space, folder, pinned state, and slot. Returns the reopened tab id,
// or negative when there's nothing to reopen. The host hydrates it as the new
// active tab on the resulting StructureChanged. Main thread only.
int32_t browser_tab_reopen_closed(void);

// Whether a closed tab is available to reopen (for the Reopen Closed Tab menu
// item's enabled state). Main thread only.
bool browser_tab_can_reopen_closed(void);

// Pin (persist) or unpin tab `id`. Unpinning returns it to "today" and clears
// its folder. Main thread only.
void browser_tab_set_pinned(int32_t id, bool pinned);

// Move tab `id` into folder `folder` (negative ⇒ out of any folder). Moving
// into a folder auto-pins the tab. Main thread only.
void browser_tab_move_to_folder(int32_t id, int32_t folder);

// Drag-and-drop relocation: set tab `id`'s section (`pinned`; `folder` negative
// ⇒ no folder) and position it just before tab `before` (negative ⇒ the
// section's end). A folder destination auto-pins. Main thread only.
void browser_tab_move(int32_t id, bool pinned, int32_t folder, int32_t before);

// Folders live in a space's pinned section and hold pinned tabs.
// Create an empty folder (named `name`) in the active space; returns its id.
int32_t browser_create_folder(const char *name);
// Delete folder `id`, keeping its tabs (they drop to loose pinned).
void browser_delete_folder(int32_t id);
void browser_rename_folder(int32_t id, const char *name);

// Spaces are switchable tab sets. Create a new empty space (with an emoji
// icon, shown in the switcher) and switch to it.
int32_t browser_create_space(const char *name, const char *icon);
void browser_select_space(int32_t id);
void browser_rename_space(int32_t id, const char *name);
void browser_set_space_icon(int32_t id, const char *icon);
// Delete space `id` and close its tabs (refused for the last space).
void browser_delete_space(int32_t id);

// Bidirectional transform. Promote folder `id` to its own space (its tabs move
// with it) → new space id. Demote space `id` to a folder in the ACTIVE space
// (its pinned tabs move in, its ephemeral tabs are closed) → new folder id.
int32_t browser_folder_to_space(int32_t id);
int32_t browser_space_to_folder(int32_t id);

// Run the CEF message loop. Blocks until browser_quit / all browsers close.
void browser_run_message_loop(void);

// Navigation — all main-thread only, no-ops when no browser is alive.
// Load `url` (NUL-terminated UTF-8) in the primary browser's main frame.
void browser_navigate(const char *url);
// Navigate back / forward one history entry.
void browser_go_back(void);
void browser_go_forward(void);
// Reload the current page.
void browser_reload(void);
// Reload ignoring the cache (hard reload).
void browser_reload_ignore_cache(void);
// Stop the current page's in-flight load (the reload button's cancel state).
void browser_stop_load(void);

// Return keyboard focus to the active tab's web view (no-op if none). Called
// when a transient overlay like the command bar closes, so the page reliably
// regains focus instead of leaving keystrokes falling on the bare window.
void browser_focus_active_tab(void);

// Whether the primary browser can navigate back / forward (for menu validation).
bool browser_can_go_back(void);
bool browser_can_go_forward(void);

// Current page URL as a newly-allocated C string (or NULL). Free with
// browser_string_free. Used to prefill the command bar for edit-in-place (Cmd+L).
char *browser_get_url(void);
void browser_string_free(char *ptr);

// Zoom the current page in / out / back to actual size.
void browser_zoom_in(void);
void browser_zoom_out(void);
void browser_zoom_reset(void);

// Toggle DevTools for the current page — open it in its own window, or close it
// if already open. Backs the Cmd+Opt+I menu item (F12 is handled in the core).
void browser_toggle_dev_tools(void);

// Find-in-page. `find_next` is false for a fresh search, true when stepping to
// the next/previous match of the same term; `forward` picks direction. Results
// arrive via the callback registered below.
void browser_find(const char *text, bool forward, bool match_case, bool find_next);
// End the current find-in-page session.
void browser_stop_finding(bool clear_selection);
// Register (or clear, with NULL) the find-result callback. Invoked on the main
// thread with (match count, 1-based active ordinal, whether this is the final
// update for the search).
void browser_set_find_result_callback(void (*callback)(int32_t count,
                                                       int32_t active_ordinal,
                                                       bool final_update));

// Ask CEF to exit its message loop.
void browser_quit(void);

// Route a host window close through CEF's browser-close flow. Returns true if
// a close was initiated — veto the window close and wait for CEF to re-request
// it (the message loop quits once the last browser is gone). Returns false if
// closing is already underway — let the window close, and release the parent
// view so browser teardown can complete.
bool browser_request_close_all(void);

// ─── Autocomplete: history suggestions + favicons ───────────────────────────
// Frecency-ranked history suggestions for `query`, as a JSON array string of
// {"url","title","host"} objects. Heap-allocated by Rust — free with
// browser_string_free (declared above).
char *browser_query_suggestions(const char *query);

// Captured favicon PNG bytes for `host`, or NULL if none is cached. On success
// *out_len holds the byte length; free the buffer with browser_free_bytes.
uint8_t *browser_favicon_png(const char *host, size_t *out_len);

// Free a byte buffer returned by browser_favicon_png.
void browser_free_bytes(uint8_t *ptr, size_t len);

// ─── History surface (the native History page) ──────────────────────────────
// Recency-ordered history rows for `query`, as a JSON array string of
// {"url","title","host","last_visit","visit_count"} objects (empty [] when none
// match). An empty query lists all history; limit/offset paginate. Heap-allocated
// by Rust — free with browser_string_free. Main thread only.
char *browser_history_list(const char *query, uint32_t limit, uint32_t offset);

// Delete the history entry for `url`. Returns whether a row was removed. Main
// thread only.
bool browser_history_delete(const char *url);

// Clear history entries last visited at or after `since` (Unix seconds); 0 clears
// all history. Returns the number of rows removed. Main thread only.
uint64_t browser_history_clear(int64_t since);

// Tear down CEF. Call after browser_run_message_loop returns.
void browser_shutdown(void);

#endif /* BROWSER_BRIDGING_HEADER_H */
