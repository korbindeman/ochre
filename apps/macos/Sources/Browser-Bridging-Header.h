// Bridging header - exposes the Rust core's C ABI (libbrowser_ffi) to Swift.
// Mirrors the `#[no_mangle] extern "C"` functions in crates/ffi/src/lib.rs.

#ifndef BROWSER_BRIDGING_HEADER_H
#define BROWSER_BRIDGING_HEADER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Initialize CEF for embedded hosting. Returns true on success.
// Call once, on the main thread, before creating any window.
bool browser_initialize(void);

// Write a local crash JSON payload for an uncaught ObjC/Swift exception.
void browser_crash_report_swift_exception(const char *name, const char *reason);

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
//   5  StructureChanged  no id / payload - re-pull browser_structure_json
//   6  TabNavStateChanged payload = "bf" - back/forward enablement as '0'/'1'
//   7  OpenTabRequested   payload = url; id = 1 foreground / 0 background
//   8  TabSuspended       no payload - host detaches the CEF view (ADR-0033)
//   9  TabThemeColorChanged payload = CSS color string (viewport URL bar tint)
//  10  FullscreenChanged  id = 1 enter / 0 exit - HTML5 content fullscreen;
//                         Alloy host must enter/exit OS window fullscreen
//  11  OpenLittleRequested payload = url - open a little window (a favorite's
//                         link left its site, P1-45)

// Register (or clear, with NULL) the tab/space event callback. Invoked on the
// main thread. `payload` is owned by the core and valid only during the call.
void browser_set_event_callback(void (*callback)(int32_t event_type,
                                                 int32_t window_id, int32_t id,
                                                 const char *payload));

// The whole tree as a JSON snapshot (see crates/ffi browser_structure_json).
// Heap-allocated by Rust - free with browser_string_free.
char *browser_structure_json(void);

// Little windows (P1-45): a tab in its own small window, outside the sidebar.
// Content events arrive on window 0; the snapshot lists them in little_tabs.
int32_t browser_little_tab_create(void *parent_view, int32_t width, int32_t height,
                                  const char *url);
bool browser_little_tab_promote(int32_t id);
void browser_little_tab_close(int32_t id);
void browser_set_focused_little(int32_t id);

// Save a friction report (structure, log tail, note, focused surface) and
// return its directory for the host to add window.png; NULL on failure.
// Heap-allocated by Rust - free with browser_string_free.
char *browser_capture_friction(const char *note, const char *focused);

// Create a new tab rendering into `parent_view` (NSView*) at the given size,
// loading `url` (NUL-terminated UTF-8, or NULL for blank). Returns the tab id
// (>= 0) or negative on failure. The tab becomes active (open / working set).
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

// Hide an active saved tab without removing it from the saved section. Main
// thread only.
void browser_tab_hide_saved(int32_t id);

// Permanently delete tab `id` (context-menu path). Main thread only.
void browser_tab_delete(int32_t id);

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

// Save (persist) or move-to-open tab `id` (`pinned` flag in core). Moving to open
// clears its folder. Main thread only.
void browser_tab_set_pinned(int32_t id, bool pinned);

// Pin or unpin tab `id` in the Arc-style icon strip (ADR-0032). Main thread only.
void browser_tab_set_favorite(int32_t id, bool favorite);
void browser_space_set_pinned_expanded(int32_t id, bool expanded);
// Make the current live URL the durable URL of a saved tab / favorite.
void browser_tab_replace_pinned_url(int32_t id);

// Move tab `id` into folder `folder` (negative ⇒ out of any folder). Moving
// into a folder auto-saves the tab. Main thread only.
void browser_tab_move_to_folder(int32_t id, int32_t folder);

// Drag-and-drop relocation: set tab `id`'s section (`favorite`, `pinned`;
// `folder` negative ⇒ no folder) and position it just before tab `before`
// (negative ⇒ the section's end). A folder destination auto-pins. Main thread only.
void browser_tab_move(int32_t id, bool favorite, bool pinned, int32_t folder,
                      int32_t before);

// Folders live in a space's saved section and hold saved tabs.
// Create an empty folder (named `name`) in the active space; returns its id.
int32_t browser_create_folder(const char *name);
// Create a folder and move `tab_ids` (length `count`) into it (auto-pins).
int32_t browser_create_folder_with_tabs(const char *name, const int32_t *tab_ids,
                                        int32_t count);
// Close open tabs except those in `keep_ids` (`count` 0 ⇒ keep active).
void browser_clear_ephemeral_tabs(const int32_t *keep_ids, int32_t count);
// Delete folder `id`, keeping its tabs (they drop to loose saved).
void browser_delete_folder(int32_t id);
void browser_rename_folder(int32_t id, const char *name);
// Persist folder `id`'s sidebar expand/collapse state (survives relaunch and
// space switches).
void browser_set_folder_expanded(int32_t id, bool expanded);

// Spaces are switchable tab sets. Create a new empty space (with an emoji
// icon, shown in the switcher) and switch to it.
int32_t browser_create_space(const char *name, const char *icon);
void browser_select_space(int32_t id);
// Keyboard shortcuts (style §9) - main window, active space.
bool browser_tab_select_by_index(int32_t index);   // 1…8, 9 = last
bool browser_tab_select_adjacent(int32_t delta);   // -1 prev, +1 next
bool browser_space_select_adjacent(int32_t delta); // -1 prev, +1 next
bool browser_tab_move_visible(int32_t delta);      // -1 up, +1 down
void browser_rename_space(int32_t id, const char *name);
void browser_set_space_icon(int32_t id, const char *icon);
// Delete space `id` and close its tabs (refused for the last space).
void browser_delete_space(int32_t id);

// Bidirectional transform. Promote folder `id` to its own space (its tabs move
// with it) → new space id. Demote space `id` to a folder in the ACTIVE space
// (its pinned tabs move in, its ephemeral tabs are closed) → new folder id.
int32_t browser_folder_to_space(int32_t id);
int32_t browser_space_to_folder(int32_t id);

// Arc sidebar import (`StorableSidebar.json`). Preview counts without mutating
// the tab store; import merges or replaces the session. JSON envelopes - free
// with browser_string_free. Main thread only.
char *browser_import_arc_default_path(void);
char *browser_import_arc_preview(const char *path);
char *browser_import_arc(const char *path, bool replace);

// Run the CEF message loop. Blocks until browser_quit / all browsers close.
void browser_run_message_loop(void);

// Navigation - all main-thread only, no-ops when no browser is alive.
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

// Exit HTML5 / browser content fullscreen on the active tab. Call when the
// host window leaves OS fullscreen while content is still fullscreen (Alloy).
void browser_exit_fullscreen(void);

// Zoom the current page in / out / back to actual size.
void browser_zoom_in(void);
void browser_zoom_out(void);
void browser_zoom_reset(void);

// Toggle DevTools for the current page - open it in its own window, or close it
// if already open. Backs the Cmd+Opt+I menu item (F12 is handled in the core).
// DevTools opens as a separate window; inline docking isn't feasible under CEF
// 149's Chrome bootstrap (ADR-0021).
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
// a close was initiated - veto the window close and wait for CEF to re-request
// it (the message loop quits once the last browser is gone). Returns false if
// closing is already underway - let the window close, and release the parent
// view so browser teardown can complete.
bool browser_request_close_all(void);

// Close one window (`0` = normal, `>= 1` = private). Main thread only.
bool browser_request_close_window(int32_t window_id);

// Tell the core which window owns nav commands. Main thread only.
void browser_set_focused_window(int32_t window_id);

// Private (incognito) windows - ephemeral tabs in isolated CEF profiles.
int32_t browser_private_window_create(void);
char *browser_private_structure_json(int32_t window_id);
int32_t browser_private_tab_create(int32_t window_id, void *parent_view,
                                   int32_t width, int32_t height, const char *url);
void browser_private_tab_select(int32_t window_id, int32_t id);
bool browser_private_tab_select_by_index(int32_t window_id, int32_t index);
bool browser_private_tab_select_adjacent(int32_t window_id, int32_t delta);
void browser_private_tab_close(int32_t window_id, int32_t id);
void browser_private_destroy_window(int32_t window_id);
bool browser_private_adblock_site_allowlisted(int32_t window_id);
void browser_private_adblock_set_site_allowlisted(int32_t window_id, bool allowlisted);

bool browser_settings_external_links_little(void);
void browser_settings_set_external_links_little(bool enabled);
bool browser_settings_private_vault_enabled(void);
void browser_settings_set_private_vault_enabled(bool enabled);
bool browser_settings_private_vault_prompt_seen(void);
void browser_settings_set_private_vault_prompt_seen(bool seen);

// ─── Autocomplete: history suggestions + favicons ───────────────────────────
// Frecency-ranked history suggestions for `query`, as a JSON array string of
// {"url","title","host"} objects. Heap-allocated by Rust - free with
// browser_string_free (declared above).
char *browser_query_suggestions(const char *query);

// Fuzzy-match `query` against `text` using the core omnibox scorer. Returns a
// JSON object {"score":…,"indices":[…]} on success, or the literal "null" when
// there is no match. Heap-allocated by Rust - free with browser_string_free.
char *browser_fuzzy_match(const char *query, const char *text);

// Captured favicon PNG bytes for `host`, or NULL if none is cached. On success
// *out_len holds the byte length; free the buffer with browser_free_bytes.
uint8_t *browser_favicon_png(const char *host, size_t *out_len);

// Free a byte buffer returned by browser_favicon_png.
void browser_free_bytes(uint8_t *ptr, size_t len);

// ─── History surface (the native History page) ──────────────────────────────
// Recency-ordered history rows for `query`, as a JSON array string of
// {"url","title","host","last_visit","visit_count"} objects (empty [] when none
// match). An empty query lists all history; limit/offset paginate. Heap-allocated
// by Rust - free with browser_string_free. Main thread only.
char *browser_history_list(const char *query, uint32_t limit, uint32_t offset);

// Delete the history entry for `url`. Returns whether a row was removed. Main
// thread only.
bool browser_history_delete(const char *url);

// Clear history entries last visited at or after `since` (Unix seconds); 0 clears
// all history. Returns the number of rows removed. Main thread only.
uint64_t browser_history_clear(int64_t since);

// ─── Password vault surface (the native Passwords page) ─────────────────────
// Thin C ABI over the vault service (ADR-0016/0017). Fallible calls return a JSON
// envelope string ({"ok":true} / {"ok":true,"imported":N} / {"ok":false,"error":"…"});
// every returned string is heap-allocated - free with browser_string_free. The
// networked/heavy calls (unlock_*, import_*) BLOCK - call them off the main thread.

// Initialize the global vault for `server_url` (empty ⇒ default Bitwarden cloud;
// else a self-hosted base URL), caching under `cache_dir`.
void browser_vault_init(const char *server_url, const char *cache_dir);
// True when the vault is locked or was never initialized.
bool browser_vault_is_locked(void);
// Whether the vault has been set up (server/account configured). UI shows the
// setup screen when false.
bool browser_vault_is_configured(void);
// First-run setup: point the vault at `server` (empty ⇒ default Bitwarden cloud),
// log in, and persist the config so later launches auto-init. `code` empty ⇒ first
// attempt; a {"two_factor":"…"} envelope means re-call with the code + that provider
// tag. Blocks. JSON envelope.
char *browser_vault_setup(const char *server, const char *email, const char *password,
                          const char *code, const char *provider);
// Online unlock. `code` empty ⇒ no second factor supplied yet; a {"two_factor":"…"}
// envelope means re-call with the code + provider tag. Blocks. JSON envelope.
char *browser_vault_unlock_online(const char *email, const char *password, const char *code,
                                  const char *provider);
// Re-dispatch the login two-factor email ("Resend code"). Blocks. JSON envelope.
char *browser_vault_send_email_code(const char *email, const char *password);
// Offline unlock from cache. Blocks (KDF). JSON envelope.
char *browser_vault_unlock_offline(const char *password);
// Lock the vault (wipes the in-memory key).
void browser_vault_lock(void);
// Logins matching `host` as a JSON array [{ "id","name","username","uris" }]
// (metadata only, no passwords). Empty array when locked. Ordered strongest
// match first, then most-recently-used within a tier (ADR-0048).
char *browser_vault_lookup(const char *host);
// Record that login `id` was just used, so it leads its site's list next time
// (ADR-0048). For the gestures the host owns - copying the username/password out
// of the URL-bar popover; the core's own fill and in-page picker record
// themselves. NOT for revealing an entry on the Passwords page. Private-window
// use is ignored by the core.
void browser_vault_note_used(const char *id);
// All logins as a JSON array (metadata only).
char *browser_vault_list(void);
// Logins matching a free-text `query` across the whole vault, as a JSON array
// (metadata only), best-first: fuzzy-scored on name/username/URI-host, ties
// broken by last-used (ADR-0049). An EMPTY query lists the whole vault, so the
// filtered and unfiltered states are one call. Empty array when locked. The
// popover's search field and the Passwords page list both use this - never
// filter the list in the frontend.
char *browser_vault_search(const char *query);
// The password for login `id` - the one path a secret leaves the core, on an
// explicit user gesture. NULL when locked/unknown. Free with browser_string_free.
char *browser_vault_password(const char *id);
// Full detail for login `id` as JSON {id,name,username,password,uri,totp,notes}
// for the edit form - a secret path, on an explicit gesture. NULL when
// locked/unknown. Free with browser_string_free.
char *browser_vault_login(const char *id);
// Fill the active tab's login form with vault login `id` - the URL-bar vault
// button's Fill action (P2-34/ADR-0029). The credential enters the page on this
// gesture; a lenient fill targets the current login fields (works on a two-step
// form's email-only first step too). No-op when locked/unknown or no active tab.
// Main thread only.
void browser_vault_fill_active_tab(const char *id);
// Import a CSV export. Blocks. JSON envelope with an `imported` count.
char *browser_vault_import_csv(const char *data);
// Import a Bitwarden JSON export. Blocks. JSON envelope with `imported`.
char *browser_vault_import_bitwarden_json(const char *data);
// Create a new login. Blocks. JSON envelope. Empty fields → omitted.
char *browser_vault_add_login(const char *name, const char *username, const char *password,
                              const char *uri, const char *totp, const char *notes);
// Update login `id`. Blocks. JSON envelope.
char *browser_vault_update_login(const char *id, const char *name, const char *username,
                                 const char *password, const char *uri, const char *totp,
                                 const char *notes);
// Delete login `id`. Blocks. JSON envelope.
char *browser_vault_delete_login(const char *id);

// ─── CDP automation surface (P0-4) ──────────────────────────────────────────
// Enable/disable the consent-gated CDP surface; persisted, takes effect next launch.
void browser_cdp_set_enabled(bool enabled);
bool browser_cdp_is_enabled(void);

// ─── Adblock (ADR-0020, spec §11) ───────────────────────────────────────────
// Native request-path ad-blocking. The global switch and the per-site allowlist
// are persisted; the shield reads the ACTIVE tab's state. Main thread only.
// Whether blocking is globally enabled (on by default).
bool browser_adblock_is_enabled(void);
// Toggle blocking globally (persisted); reloads the active tab so it applies.
void browser_adblock_set_enabled(bool enabled);
// Requests blocked on the active tab since its last navigation (shield badge).
uint64_t browser_adblock_active_blocked_count(void);
// Whether blocking is turned OFF for the active tab's site (allowlisted).
bool browser_adblock_active_site_allowlisted(void);
// Turn blocking off (true) / back on (false) for the active tab's site
// (persisted); reloads the tab so it applies on screen.
void browser_adblock_set_active_site_allowlisted(bool allowlisted);

// Cookie-consent handling (reject + hide) - its own toggle, persisted (ADR-0026).
// Whether cookie-consent handling is on (on by default).
bool browser_consent_is_enabled(void);
// Toggle cookie-consent handling (persisted); reloads the active tab so it applies.
void browser_consent_set_enabled(bool enabled);
// Consent banners auto-rejected on the active tab since its last navigation.
uint64_t browser_consent_active_rejected_count(void);

// Browser settings - vault auto-lock policy (0 Never / 1 OnBrowserClose /
// 2 OnMacRestart / 3 OnPasswordUse).
int32_t browser_settings_vault_auto_lock(void);
void browser_settings_set_vault_auto_lock(int32_t policy);
// Lock the vault on graceful shutdown when the policy requires it.
void browser_settings_lock_vault_on_shutdown(void);
// Forward the Mac's system boot time (Unix seconds) for OnMacRestart policy.
void browser_settings_set_system_boot_secs(uint64_t secs);

// Default search engine (0 Google / 1 DuckDuckGo / 2 Bing / 3 Brave /
// 4 Startpage / 5 Ecosia / 6 Kagi).
int32_t browser_settings_search_engine(void);
void browser_settings_set_search_engine(int32_t engine);
// Build a search-results URL for the query using the selected engine.
char *browser_search_url_for_query(const char *query);

// Appearance / color scheme (ADR-0030). One preference drives web content
// (prefers-color-scheme) and native chrome: 0 System, 1 Light, 2 Dark.
// Current appearance preference (0 System / 1 Light / 2 Dark).
int32_t browser_theme_preference(void);
// Set the appearance preference (persisted); applied live to open tabs.
void browser_theme_set_preference(int32_t preference);
// Forward the current OS appearance (launch + on OS theme change); drives System mode.
void browser_theme_set_system_dark(bool is_dark);

// Auto-update (ADR-0052, Zinnia model). Check/install block - call off main.
// Returns JSON AvailableUpdate or null. Free with browser_string_free.
char *browser_update_check(const char *current_version);
// Last check result as JSON, or null. Free with browser_string_free.
char *browser_update_last_available(void);
void browser_update_dismiss(void);
// Download + verify + stage + spawn relaunch helper. True ⇒ host should quit.
bool browser_update_download_and_install(const char *app_bundle, const char *update_json);
void browser_update_download_progress(uint64_t *downloaded, uint64_t *total);

// Tear down CEF. Call after browser_run_message_loop returns.
void browser_shutdown(void);

#endif /* BROWSER_BRIDGING_HEADER_H */
