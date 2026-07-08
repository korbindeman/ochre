//! Tab / folder / space model — the browser's structural spine.
//!
//! The core owns the authoritative structure; the native app is a *view* of it.
//! It is a tree (ADR-0006):
//!
//! * A **space** is a named container with its own remembered active tab. It is
//!   rendered as a *saved* section (folders + loose saved tabs) above an *open*
//!   section (the working set).
//! * A **folder** lives in a space's saved section and holds saved tabs.
//! * A **tab** belongs to a space. The `pinned` flag means *saved* (persistent
//!   in the sidebar's tree below New Tab). The `favorite` flag (ADR-0032) is the
//!   Arc-style pinned icon strip below the URL bar — always resident, dismiss-
//!   not-destroy on close. The two are mutually exclusive. Only saved tabs may
//!   belong to a folder. New tabs start open.
//!
//! **Identity (ADR-0013).** [`TabId`] is core-assigned (a persisted counter),
//! *not* CEF's browser identifier — CEF ids restart every session, and a
//! restored tab must keep its identity before any browser exists. A tab's
//! `browser` is therefore `Option`al: `None` means *suspended* — restored from
//! the persisted session but not yet hydrated. Engine-side handlers address
//! tabs by CEF identifier (the `note_*` functions translate); popups /
//! DevTools, never registered here, translate to no tab.
//!
//! **Persistence (ADR-0013).** The whole structure (plus per-tab URL/title) is
//! written through to `session.json` on every structural / URL / title change
//! ([`crate::session`]), and loaded once in [`init`]. Tabs stay until they are
//! explicitly closed — quitting the app persists, it never discards; the
//! app-quit teardown path is masked off by [`begin_shutdown`] so the browsers
//! draining on exit are not mistaken for the user closing tabs.
//!
//! **Events (ADR-0005).** Hot per-tab *content* changes are fine-grained
//! ([`note_title`]/[`note_url`]/[`note_load_state`]/[`note_favicon`]). Every
//! *structural* change — create/close/select, pin, folder membership, space /
//! folder CRUD, and the folder↔space transforms — emits a single
//! [`Event::StructureChanged`]; the host responds by re-pulling [`snapshot`] and
//! rebuilding its tree. All mutation runs on the main/UI thread.

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use cef::{Browser, ImplBrowser, ImplBrowserHost};
use serde::Serialize;

use crate::session::{self, SavedFolder, SavedSpace, SavedState, SavedTab};

pub type TabId = i32;
pub type SpaceId = i32;
pub type FolderId = i32;

/// Sentinel meaning "no id" on the wire (e.g. move-to-no-folder). Real CEF /
/// core ids are always ≥ 1.
pub const NONE_ID: i32 = -1;

/// Events pushed to the host over the FFI callback. The discriminant is the
/// `event_type` int the host switches on. Content events carry the tab id and a
/// payload; [`StructureChanged`](Event::StructureChanged) carries neither — the
/// host re-pulls [`snapshot`] instead.
#[repr(i32)]
pub(crate) enum Event {
    TabTitleChanged = 0,    // payload = title
    TabUrlChanged = 1,      // payload = url
    TabLoadStarted = 2,     // no payload
    TabLoadEnded = 3,       // no payload
    TabFaviconChanged = 4,  // payload = host
    StructureChanged = 5,   // no id / payload — host re-pulls snapshot
    TabNavStateChanged = 6, // payload = "bf": back/forward enablement as '0'/'1'
    /// Host opens a tab loading `payload`. `id` is 1 for foreground (activate),
    /// 0 for background — emitted when a link/window.open would otherwise spawn
    /// a bare CEF popup (target="_blank" and friends).
    OpenTabRequested = 7,
    /// A background tab's CEF browser was discarded; the host detaches its view
    /// (ADR-0033). Re-show hydrates via `restore_tab`.
    TabSuspended = 8,
    /// Sampled page theme/background color for viewport chrome tinting.
    /// Payload is a CSS color string (`#rrggbb`, `rgb(...)`, …) or empty.
    TabThemeColorChanged = 9,
}

/// Host callback, invoked on the main (UI) thread. `window_id` is `0` for the
/// normal window, `>= 1` for a private window. `payload` is owned by the core
/// and valid only for the duration of the call — copy out, don't retain.
pub type EventCallback =
    extern "C" fn(event_type: i32, window_id: i32, id: i32, payload: *const c_char);

static CALLBACK: Mutex<Option<EventCallback>> = Mutex::new(None);
static STORE: Mutex<Option<TabStore>> = Mutex::new(None);
/// Where the session is persisted; set once in [`init`]. Absent (tests, or an
/// uninitialized store) means persistence is off.
static SESSION_PATH: OnceLock<String> = OnceLock::new();
/// Set by [`begin_shutdown`] when app quit starts: the teardown then closes
/// every browser through the same handlers as a user tab-close, and persisting
/// those removals would wipe the session the user expects back next launch.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

struct Tab {
    id: TabId,
    space: SpaceId,
    /// `Some` only when `pinned` — folders hold persistent tabs.
    folder: Option<FolderId>,
    pinned: bool,
    /// Arc-style pinned icon strip (ADR-0032). `favorite ⇒ !pinned`.
    favorite: bool,
    /// Hidden from the strip after a user close; browser stays loaded.
    dismissed: bool,
    /// The live CEF browser, or `None` for a *suspended* tab — restored from
    /// the persisted session and hydrated (via [`attach_browser`]) the first
    /// time it is shown.
    browser: Option<Browser>,
    url: String,
    title: String,
    loading: bool,
    /// History enablement, mirrored from CEF's `on_loading_state_change` so the
    /// host can disable the back/forward controls (streamed, not polled).
    can_go_back: bool,
    can_go_forward: bool,
    /// Unix millis when the tab was last shown. `0` = never shown (first to
    /// suspend when over the warm-tab budget). Runtime-only — not persisted.
    last_shown_at: i64,
}

/// A tab's state captured the moment it is closed, enough to faithfully reopen
/// it (⌘⇧T): its identity, its section (space / folder / pinned), the content to
/// reload, and the sibling it sat before so it returns to its old slot.
struct ClosedTab {
    id: TabId,
    space: SpaceId,
    folder: Option<FolderId>,
    pinned: bool,
    favorite: bool,
    url: String,
    title: String,
    /// The tab that immediately followed it in its section at close time, so
    /// reopen can slot it back before that sibling; `None` if it was last.
    before: Option<TabId>,
}

/// How many recently-closed tabs the reopen stack retains (Chrome keeps ~25).
const CLOSED_STACK_CAP: usize = 25;

struct Folder {
    id: FolderId,
    space: SpaceId,
    name: String,
}

struct Space {
    id: SpaceId,
    name: String,
    /// Emoji icon shown in the space switcher (a single grapheme; the host
    /// validates/normalizes input).
    icon: String,
    /// The tab shown when this space is active (per-space memory).
    active: Option<TabId>,
}

/// Icon for spaces created without an explicit one (folder→space promotion).
const DEFAULT_SPACE_ICON: &str = "✨";

struct TabStore {
    tabs: Vec<Tab>,
    folders: Vec<Folder>,
    spaces: Vec<Space>,
    active_space: SpaceId,
    next_space: SpaceId,
    next_folder: FolderId,
    next_tab: TabId,
    /// Most-recently-closed tabs, newest last — the stack ⌘⇧T pops from
    /// ([`reopen_closed`](TabStore::reopen_closed)). Session-scoped and capped
    /// ([`CLOSED_STACK_CAP`]); deliberately *not* persisted — session restore
    /// already brings back every tab still open at quit (ADR-0013). Only
    /// single-tab closes record here; bulk removals (space delete / demote) do
    /// not.
    closed: Vec<ClosedTab>,
}

impl TabStore {
    fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn browser_of(&self, id: TabId) -> Option<Browser> {
        self.tab(id).and_then(|t| t.browser.clone())
    }

    /// Translate a CEF browser identifier to its tab, if that browser is a
    /// registered, hydrated tab (popups / DevTools / suspended tabs → `None`).
    fn tab_id_by_cef(&self, cef_id: i32) -> Option<TabId> {
        self.tabs
            .iter()
            .find(|t| t.browser.as_ref().is_some_and(|b| b.identifier() == cef_id))
            .map(|t| t.id)
    }

    /// The tab currently shown — the active space's remembered active tab.
    fn visible_tab(&self) -> Option<TabId> {
        self.spaces
            .iter()
            .find(|s| s.id == self.active_space)
            .and_then(|s| s.active)
    }

    fn set_space_active(&mut self, space: SpaceId, tab: Option<TabId>) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == space) {
            s.active = tab;
        }
    }

    /// First tab belonging to `space` in store order, if any.
    fn first_tab_in(&self, space: SpaceId) -> Option<TabId> {
        self.tabs.iter().find(|t| t.space == space).map(|t| t.id)
    }

    /// Whether `t` belongs to the same sidebar section as the destination triple.
    fn same_section(
        t: &Tab,
        space: SpaceId,
        favorite: bool,
        pinned: bool,
        folder: Option<FolderId>,
    ) -> bool {
        t.space == space && t.favorite == favorite && t.pinned == pinned && t.folder == folder
    }

    /// The next tab in `space` the user can activate — skips dismissed favorites.
    fn next_selectable_in(&self, space: SpaceId) -> Option<TabId> {
        self.tabs
            .iter()
            .find(|t| t.space == space && (!t.favorite || !t.dismissed))
            .map(|t| t.id)
    }

    /// Sidebar-selectable tab order for `space` — mirrors the host's sidebar
    /// layout (favorites → each folder's tabs → loose saved → open). Skips
    /// dismissed favorites. Drives keyboard shortcuts (style §9).
    fn sidebar_tab_order(&self, space: SpaceId) -> Vec<TabId> {
        let mut ids = Vec::new();
        for t in &self.tabs {
            if t.space == space && t.favorite && !t.dismissed {
                ids.push(t.id);
            }
        }
        for folder in self.folders.iter().filter(|f| f.space == space) {
            for t in &self.tabs {
                if t.folder == Some(folder.id) {
                    ids.push(t.id);
                }
            }
        }
        for t in &self.tabs {
            if t.space == space && t.pinned && !t.favorite && t.folder.is_none() {
                ids.push(t.id);
            }
        }
        for t in &self.tabs {
            if t.space == space && !t.pinned && !t.favorite {
                ids.push(t.id);
            }
        }
        ids
    }

    /// Hide a favorite from the strip without tearing down its browser.
    fn dismiss_favorite(&mut self, id: TabId) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        if !self.tabs[pos].favorite {
            return;
        }
        self.tabs[pos].dismissed = true;
        let space = self.tabs[pos].space;
        let was_active = self
            .spaces
            .iter()
            .any(|s| s.id == space && s.active == Some(id));
        if was_active {
            self.set_space_active(space, self.next_selectable_in(space));
        }
    }

    /// Remove a tab, repairing its space's active pointer, and hand back its
    /// browser (if hydrated) so the caller can tear it down. Does *not* emit —
    /// callers run inside [`structural`].
    fn remove_tab(&mut self, id: TabId) -> Option<Browser> {
        let pos = self.tabs.iter().position(|t| t.id == id)?;
        let space = self.tabs[pos].space;
        let browser = self.tabs.remove(pos).browser;
        let was_active = self
            .spaces
            .iter()
            .any(|s| s.id == space && s.active == Some(id));
        if was_active {
            self.set_space_active(space, self.next_selectable_in(space));
        }
        browser
    }

    /// Record `id` on the closed-tab stack (so ⌘⇧T can bring it back), then
    /// remove it via [`remove_tab`]. The reopen-aware close: only the single-tab
    /// close paths use this; bulk removals call [`remove_tab`] directly and are
    /// not reopenable.
    fn remove_tab_recording(&mut self, id: TabId) -> Option<Browser> {
        let captured = self.tab(id).map(|tab| ClosedTab {
            id: tab.id,
            space: tab.space,
            folder: tab.folder,
            pinned: tab.pinned,
            favorite: tab.favorite,
            url: tab.url.clone(),
            title: tab.title.clone(),
            // The next tab in this tab's own section (same space / favorite /
            // pinned / folder) — the anchor reopen slots the tab back before.
            before: self
                .tabs
                .iter()
                .skip_while(|t| t.id != id)
                .skip(1)
                .find(|t| Self::same_section(t, tab.space, tab.favorite, tab.pinned, tab.folder))
                .map(|t| t.id),
        });
        if let Some(captured) = captured {
            self.closed.push(captured);
            if self.closed.len() > CLOSED_STACK_CAP {
                self.closed.remove(0);
            }
        }
        self.remove_tab(id)
    }

    /// Pop the most recently closed tab and re-insert it, *suspended*
    /// (`browser: None`), into its original space / section / slot — restoring
    /// its identity, pinned state, and folder membership — then make it that
    /// space's active tab and switch to that space. The host hydrates it on the
    /// resulting event, exactly like a session-restored tab. Returns the
    /// reopened tab id, or `None` when the stack is empty. See
    /// [`reopen_closed_tab`].
    ///
    /// Faithful but self-healing: if the original space or folder is gone, the
    /// tab falls back to the active space / loose-pinned so it always comes back
    /// somewhere reachable; if its old sibling has since moved or closed, it
    /// lands at its section's end.
    fn reopen_closed(&mut self) -> Option<TabId> {
        let closed = self.closed.pop()?;
        let space = if self.spaces.iter().any(|s| s.id == closed.space) {
            closed.space
        } else {
            self.active_space
        };
        let folder = closed.folder.filter(|fid| {
            self.folders
                .iter()
                .any(|f| f.id == *fid && f.space == space)
        });
        let favorite = closed.favorite;
        let pinned = closed.pinned || folder.is_some();
        // Slot before the old sibling only if it still exists in the same
        // section; otherwise append to the section's end (mirrors relocate_tab).
        let index = closed
            .before
            .and_then(|anchor| {
                self.tabs.iter().position(|t| {
                    t.id == anchor && Self::same_section(t, space, favorite, pinned, folder)
                })
            })
            .unwrap_or_else(|| {
                self.tabs
                    .iter()
                    .rposition(|t| Self::same_section(t, space, favorite, pinned, folder))
                    .map_or(self.tabs.len(), |p| p + 1)
            });
        self.tabs.insert(
            index,
            Tab {
                id: closed.id,
                space,
                folder,
                pinned,
                favorite,
                dismissed: false,
                browser: None,
                url: closed.url,
                title: closed.title,
                loading: false,
                can_go_back: false,
                can_go_forward: false,
                last_shown_at: 0,
            },
        );
        self.active_space = space;
        self.set_space_active(space, Some(closed.id));
        Some(closed.id)
    }

    /// Drag-and-drop relocation: move `id` into a destination section within its
    /// own space — `(pinned, folder)` — positioned immediately before `before`,
    /// or at the section's end when `before` is `None`. This both *re-parents*
    /// the tab (pin/unpin, file into / out of a folder) and *reorders* it among
    /// its new siblings, in one step.
    ///
    /// Order is implicit in `tabs` order (the host's sidebar sub-lists are
    /// filters that preserve it, ADR-0006), so "reorder" means moving the tab's
    /// slot in `tabs`. A folder destination implies `pinned` and must belong to
    /// the tab's space; an invalid folder, or an unknown `id`, is a no-op. Active
    /// pointers are untouched — a relocated tab keeps its space and identity, so
    /// nothing to repair.
    fn relocate_tab(
        &mut self,
        id: TabId,
        favorite: bool,
        pinned: bool,
        folder: Option<FolderId>,
        before: Option<TabId>,
    ) {
        let Some(from) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let space = self.tabs[from].space;
        let (favorite, pinned, folder) = if favorite {
            (true, false, None)
        } else {
            match folder {
                Some(fid) if self.folders.iter().any(|f| f.id == fid && f.space == space) => {
                    (false, true, Some(fid)) // folders hold persistent tabs
                }
                Some(_) => return, // folder isn't in this tab's space — reject
                None => (false, pinned, None),
            }
        };
        {
            let tab = &mut self.tabs[from];
            tab.favorite = favorite;
            tab.pinned = pinned;
            tab.folder = folder;
            tab.dismissed = false;
        }
        // Pull the tab out, then re-insert it. Indices below are computed against
        // the post-removal Vec so they land where intended.
        let tab = self.tabs.remove(from);
        let index = match before {
            Some(anchor) if anchor != id => self.tabs.iter().position(|t| t.id == anchor),
            _ => None,
        }
        .unwrap_or_else(|| {
            // No (or self-referential / stale) anchor: append after the
            // destination section's last member, i.e. the section's end.
            self.tabs
                .iter()
                .rposition(|t| Self::same_section(t, space, favorite, pinned, folder))
                .map_or(self.tabs.len(), |p| p + 1)
        });
        self.tabs.insert(index, tab);
    }

    /// Fresh store: one default Home space, no tabs (first launch / no session).
    fn default_store() -> TabStore {
        TabStore {
            tabs: Vec::new(),
            folders: Vec::new(),
            spaces: vec![Space {
                id: 1,
                name: "Home".to_string(),
                icon: "🏠".to_string(),
                active: None,
            }],
            active_space: 1,
            next_space: 2,
            next_folder: 1,
            next_tab: 1,
            closed: Vec::new(),
        }
    }

    /// Rebuild a store from a persisted session (ADR-0013). Every tab comes
    /// back suspended (`browser: None`) and hydrates lazily. The saved tree is
    /// sanitized — rows referencing a missing space are dropped, dangling
    /// folder refs cleared, invariants (folder ⇒ pinned) re-imposed, active
    /// pointers validated — so a stale or hand-edited file degrades gracefully
    /// instead of panicking.
    fn from_saved(saved: SavedState) -> TabStore {
        if saved.spaces.is_empty() {
            return TabStore::default_store();
        }
        let spaces: Vec<Space> = saved
            .spaces
            .iter()
            .map(|s| Space {
                id: s.id,
                name: s.name.clone(),
                icon: s.icon.clone(),
                active: s.active,
            })
            .collect();
        let space_exists = |id: SpaceId| spaces.iter().any(|s| s.id == id);
        let folders: Vec<Folder> = saved
            .folders
            .into_iter()
            .filter(|f| space_exists(f.space))
            .map(|f| Folder {
                id: f.id,
                space: f.space,
                name: f.name,
            })
            .collect();
        let tabs: Vec<Tab> = saved
            .tabs
            .into_iter()
            .filter(|t| space_exists(t.space))
            .map(|t| {
                let folder = t
                    .folder
                    .filter(|fid| folders.iter().any(|f| f.id == *fid && f.space == t.space));
                let favorite = t.favorite;
                Tab {
                    id: t.id,
                    space: t.space,
                    pinned: (t.pinned || folder.is_some()) && !favorite,
                    folder: if favorite { None } else { folder },
                    favorite,
                    dismissed: t.dismissed,
                    browser: None,
                    url: t.url,
                    title: t.title,
                    loading: false,
                    can_go_back: false,
                    can_go_forward: false,
                    last_shown_at: 0,
                }
            })
            .collect();

        let mut store = TabStore {
            active_space: if space_exists(saved.active_space) {
                saved.active_space
            } else {
                spaces[0].id
            },
            next_space: spaces.iter().map(|s| s.id).max().unwrap_or(0) + 1,
            next_folder: folders.iter().map(|f| f.id).max().unwrap_or(0) + 1,
            next_tab: tabs.iter().map(|t| t.id).max().unwrap_or(0) + 1,
            tabs,
            folders,
            spaces,
            closed: Vec::new(),
        };
        // Validate each space's remembered active tab; fall back to its first
        // tab so a restored space never points at a tab it doesn't hold.
        let repairs: Vec<(SpaceId, Option<TabId>)> = store
            .spaces
            .iter()
            .map(|s| {
                let active = s
                    .active
                    .filter(|tid| store.tabs.iter().any(|t| t.id == *tid && t.space == s.id))
                    .or_else(|| store.first_tab_in(s.id));
                (s.id, active)
            })
            .collect();
        for (space, active) in repairs {
            store.set_space_active(space, active);
        }
        store
    }

    /// The persistable mirror of the current structure (ADR-0013).
    fn saved_state(&self) -> SavedState {
        SavedState {
            active_space: self.active_space,
            spaces: self
                .spaces
                .iter()
                .map(|s| SavedSpace {
                    id: s.id,
                    name: s.name.clone(),
                    icon: s.icon.clone(),
                    active: s.active,
                })
                .collect(),
            folders: self
                .folders
                .iter()
                .map(|f| SavedFolder {
                    id: f.id,
                    space: f.space,
                    name: f.name.clone(),
                })
                .collect(),
            tabs: self
                .tabs
                .iter()
                .map(|t| SavedTab {
                    id: t.id,
                    space: t.space,
                    folder: t.folder,
                    pinned: t.pinned,
                    favorite: t.favorite,
                    dismissed: t.dismissed,
                    url: t.url.clone(),
                    title: t.title.clone(),
                })
                .collect(),
        }
    }
}

/// Create the store, restoring the persisted session at `session_path` (or a
/// fresh default when none exists). All restored tabs are suspended until
/// hydrated. Idempotent; called from [`crate::embed::initialize`].
pub fn init(session_path: &str) {
    let mut guard = STORE.lock().expect("tab store lock poisoned");
    if guard.is_none() {
        let _ = SESSION_PATH.set(session_path.to_string());
        *guard = Some(
            session::load(session_path)
                .map(TabStore::from_saved)
                .unwrap_or_else(TabStore::default_store),
        );
    }
}

/// Serialize tests that mutate the module-global tab store (import, persist, …).
#[cfg(test)]
pub(crate) fn with_global_store_test_lock<R>(f: impl FnOnce() -> R) -> R {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let result = f();
    drop(guard);
    result
}

/// Replace the global store with a fresh default (test-only).
#[cfg(test)]
pub(crate) fn test_reset_store(session_path: &str) {
    SHUTTING_DOWN.store(false, Ordering::Relaxed);
    let _ = SESSION_PATH.get_or_init(|| session_path.to_string());
    *STORE.lock().expect("tab store lock poisoned") = Some(TabStore::default_store());
}

/// Mark that app quit has begun. From here on the store stops persisting:
/// teardown closes every browser through the same close handlers as a user
/// tab-close, and writing those removals out would erase the session.
pub fn begin_shutdown() {
    SHUTTING_DOWN.store(true, Ordering::Relaxed);
}

/// Write the current structure through to the session file (ADR-0013). A no-op
/// before [`init`], and once [`begin_shutdown`] has run.
fn persist() {
    if SHUTTING_DOWN.load(Ordering::Relaxed) {
        return;
    }
    let Some(path) = SESSION_PATH.get() else {
        return;
    };
    if let Some(state) = with_store(TabStore::saved_state) {
        session::save(path, &state);
    }
}

/// Register (or clear, with `None`) the host's event callback.
pub fn set_event_callback(callback: Option<EventCallback>) {
    *CALLBACK.lock().expect("event callback lock poisoned") = callback;
}

/// Read-only access to the store (for [`snapshot`] / [`active_browser`]).
fn with_store<R>(f: impl FnOnce(&TabStore) -> R) -> Option<R> {
    let guard = STORE.lock().expect("tab store lock poisoned");
    guard.as_ref().map(f)
}

/// Run a structural mutation: apply `f`, reconcile which browser is on-screen
/// (hide the old visible tab, show the new one), emit `StructureChanged`, and
/// persist the session. Returns `None` only if the store isn't initialized.
/// `f` must not emit or call into CEF — visibility + the event are handled
/// here, after the lock drops. A suspended visible tab has no browser to
/// reconcile; the host hydrates it in response to the event.
fn structural<R>(f: impl FnOnce(&mut TabStore) -> R) -> Option<R> {
    let (old_visible, old_browser, new_visible, new_browser, old_space, new_space, result) = {
        let mut guard = STORE.lock().expect("tab store lock poisoned");
        let store = guard.as_mut()?;
        let old_visible = store.visible_tab();
        let old_space = store.active_space;
        let old_browser = old_visible.and_then(|id| store.browser_of(id).map(|b| (id, b)));
        let result = f(store);
        let new_visible = store.visible_tab();
        let new_space = store.active_space;
        let new_browser = new_visible.and_then(|id| store.browser_of(id).map(|b| (id, b)));
        (
            old_visible,
            old_browser,
            new_visible,
            new_browser,
            old_space,
            new_space,
            result,
        )
    };
    if old_browser.as_ref().map(|(id, _)| *id) != new_browser.as_ref().map(|(id, _)| *id) {
        if let Some((_, browser)) = old_browser {
            set_visible(&browser, false);
        }
        if let Some((_, browser)) = new_browser {
            set_visible(&browser, true);
        }
    }
    if old_visible != new_visible || old_space != new_space {
        crate::tab_suspension::on_visibility_changed(
            old_visible,
            new_visible,
            old_space,
            new_space,
        );
    }
    emit(Event::StructureChanged, 0, None);
    persist();
    Some(result)
}

/// Tell CEF whether a tab's browser is on-screen. Backgrounded tabs stop
/// painting (Chromium won't render a hidden browser), which saves work and
/// sidesteps the occlusion-suspend "blank tab" trap. The visible tab also takes
/// keyboard focus. Safe on an invalid/closing browser (`host()` returns `None`).
fn set_visible(browser: &Browser, visible: bool) {
    let Some(host) = browser.host() else {
        return;
    };
    host.was_hidden(i32::from(!visible));
    if visible {
        // P0-1: host-driven focus request on tab-switch. The paired
        // `[focus] cef on_got_focus` (client.rs) confirms CEF honored it.
        eprintln!("[focus] host set_focus tab={}", browser.identifier());
        host.set_focus(1);
    }
}

/// Push one event to the host. A no-op when no callback is registered.
fn emit(event: Event, id: i32, payload: Option<&str>) {
    emit_for_window(crate::private_window::NORMAL_WINDOW_ID, event, id, payload);
}

/// Push one event for window `window_id` (`0` = normal).
pub(crate) fn emit_for_window(window_id: i32, event: Event, id: i32, payload: Option<&str>) {
    let callback = *CALLBACK.lock().expect("event callback lock poisoned");
    let Some(callback) = callback else {
        return;
    };
    match payload {
        None => callback(event as i32, window_id, id, std::ptr::null()),
        Some(text) => match CString::new(text) {
            Ok(c) => callback(event as i32, window_id, id, c.as_ptr()),
            Err(_) => callback(event as i32, window_id, id, std::ptr::null()),
        },
    }
}

// ─── Tab lifecycle ─────────────────────────────────────────────────────────

/// Ask the host to open a tab loading `url` instead of a CEF popup window.
/// `foreground` selects whether the new tab becomes active. Posted from
/// `LifeSpanHandler::on_before_popup` after the popup is cancelled.
pub fn request_open_tab(url: &str, foreground: bool) {
    emit(Event::OpenTabRequested, i32::from(foreground), Some(url));
}

/// Add a freshly-created browser to the store as an **ephemeral** ("today") tab
/// in the active space, and make it that space's active tab. Returns the new
/// tab's core-assigned id (or [`NONE_ID`] if the store isn't initialized).
/// Called from `browser_tab_create`.
pub fn register_tab(browser: Browser, url: &str) -> TabId {
    structural(|store| {
        let id = store.next_tab;
        store.next_tab += 1;
        let space = store.active_space;
        store.tabs.push(Tab {
            id,
            space,
            folder: None,
            pinned: false,
            favorite: false,
            dismissed: false,
            browser: Some(browser),
            url: url.to_string(),
            title: String::new(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            last_shown_at: unix_ms_now(),
        });
        store.set_space_active(space, Some(id));
        id
    })
    .unwrap_or(NONE_ID)
}

/// The saved URL of a suspended tab — what hydration loads. `None` when `id`
/// is unknown or the tab already has a browser.
pub fn suspended_url(id: TabId) -> Option<String> {
    with_store(|store| {
        store
            .tab(id)
            .filter(|t| t.browser.is_none())
            .map(|t| t.url.clone())
    })
    .flatten()
}

/// Whether `id` is a favorite (resident strip — never auto-suspended, ADR-0032).
pub fn is_favorite(id: TabId) -> bool {
    with_store(|store| store.tab(id).is_some_and(|t| t.favorite)).unwrap_or(false)
}

/// Hydrated, non-favorite tabs in `space` that are not `keep_visible`.
pub fn hydrated_background_in_space(space: SpaceId, keep_visible: Option<TabId>) -> Vec<TabId> {
    hydrated_background_candidates(Some(space), keep_visible)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// Hydrated, non-favorite background tabs with their last-shown timestamps.
/// `space` filters to one space; `None` considers every space.
pub fn hydrated_background_candidates(
    space: Option<SpaceId>,
    keep_visible: Option<TabId>,
) -> Vec<(TabId, i64)> {
    with_store(|store| {
        store
            .tabs
            .iter()
            .filter(|t| {
                t.browser.is_some()
                    && !t.favorite
                    && keep_visible != Some(t.id)
                    && space.is_none_or(|s| t.space == s)
            })
            .map(|t| (t.id, t.last_shown_at))
            .collect()
    })
    .unwrap_or_default()
}

/// Record that `id` was just shown (updates recency for suspension policy).
pub fn note_tab_shown(id: TabId) {
    let now = unix_ms_now();
    with_store_mut(|store| {
        if let Some(tab) = store.tabs.iter_mut().find(|t| t.id == id) {
            tab.last_shown_at = now;
        }
    });
}

fn unix_ms_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Discard a background tab's CEF browser, keeping url/title in the model
/// (ADR-0033). Returns `true` when the tab was suspended. Clears `browser`
/// before closing so `on_before_close` does not remove the tab.
pub fn suspend_tab(id: TabId) -> bool {
    if SHUTTING_DOWN.load(Ordering::Relaxed) {
        return false;
    }
    let browser = with_store_mut(|store| {
        if store.visible_tab() == Some(id) {
            return None;
        }
        let tab = store.tabs.iter_mut().find(|t| t.id == id)?;
        if tab.browser.is_none() || tab.favorite {
            return None;
        }
        let browser = tab.browser.take();
        tab.loading = false;
        tab.can_go_back = false;
        tab.can_go_forward = false;
        browser
    });
    let Some(browser) = browser else {
        return false;
    };
    close_browser(Some(browser));
    emit(Event::TabSuspended, id, None);
    persist();
    true
}

/// Hydrate a suspended tab with its freshly-created browser (the lazy-restore
/// step, ADR-0013). Ignored if `id` is unknown or already hydrated. If the tab
/// is the visible one (the common case — the host hydrates on show), the
/// browser gets focus via the usual reconciliation; otherwise it is hidden so
/// a background hydration never paints over the active tab.
pub fn attach_browser(id: TabId, browser: Browser) {
    let attached = structural(|store| match store.tabs.iter_mut().find(|t| t.id == id) {
        Some(tab) if tab.browser.is_none() => {
            tab.browser = Some(browser.clone());
            true
        }
        _ => false,
    })
    .unwrap_or(false);
    if attached && with_store(|store| store.visible_tab()) != Some(Some(id)) {
        set_visible(&browser, false);
    }
}

/// Make `id` the active tab of its space (and switch to that space).
pub fn select_tab(id: TabId) {
    structural(|store| {
        if let Some(space) = store.tab(id).map(|t| t.space) {
            store.active_space = space;
            store.set_space_active(space, Some(id));
        }
    });
}

/// Select a tab by its 1-based sidebar position (`1`…`8`, or `9` = last) in the
/// active space. Returns whether a tab was selected.
pub fn select_tab_by_index(index: u8) -> bool {
    structural(|store| {
        let space = store.active_space;
        let order = store.sidebar_tab_order(space);
        if order.is_empty() {
            return false;
        }
        let idx = if index == 9 {
            order.len() - 1
        } else if (1..=8).contains(&index) {
            index as usize - 1
        } else {
            return false;
        };
        if idx >= order.len() {
            return false;
        }
        let id = order[idx];
        store.active_space = space;
        store.set_space_active(space, Some(id));
        true
    })
    .unwrap_or(false)
}

/// Select the next (`delta = 1`) or previous (`delta = -1`) tab in the active
/// space's sidebar order, wrapping at the ends. Returns whether a tab was selected.
pub fn select_adjacent_tab(delta: i32) -> bool {
    if !matches!(delta, -1 | 1) {
        return false;
    }
    structural(|store| {
        let space = store.active_space;
        let order = store.sidebar_tab_order(space);
        if order.is_empty() {
            return false;
        }
        let next = match store
            .visible_tab()
            .and_then(|id| order.iter().position(|&t| t == id))
        {
            None => order[0],
            Some(pos) => {
                if order.len() < 2 {
                    return false;
                }
                let len = order.len() as i32;
                let next_pos = (pos as i32 + delta).rem_euclid(len) as usize;
                order[next_pos]
            }
        };
        store.active_space = space;
        store.set_space_active(space, Some(next));
        true
    })
    .unwrap_or(false)
}

/// Select the next (`delta = 1`) or previous (`delta = -1`) space, wrapping at the
/// ends and restoring each space's remembered active tab.
pub fn select_adjacent_space(delta: i32) -> bool {
    if !matches!(delta, -1 | 1) {
        return false;
    }
    structural(|store| {
        if store.spaces.len() < 2 {
            return false;
        }
        let Some(pos) = store.spaces.iter().position(|s| s.id == store.active_space) else {
            return false;
        };
        let len = store.spaces.len() as i32;
        let next_pos = (pos as i32 + delta).rem_euclid(len) as usize;
        let id = store.spaces[next_pos].id;
        store.active_space = id;
        let remembered = store
            .spaces
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.active)
            .filter(|tid| store.tabs.iter().any(|t| t.id == *tid && t.space == id));
        let active = remembered.or_else(|| store.first_tab_in(id));
        store.set_space_active(id, active);
        true
    })
    .unwrap_or(false)
}

/// Move the visible tab up (`delta = -1`) or down (`delta = 1`) in the active
/// space's sidebar order. Returns whether the tab moved.
pub fn move_visible_tab(delta: i32) -> bool {
    if !matches!(delta, -1 | 1) {
        return false;
    }
    structural(|store| {
        let space = store.active_space;
        let order = store.sidebar_tab_order(space);
        let Some(id) = store.visible_tab() else {
            return false;
        };
        let Some(from_idx) = order.iter().position(|&t| t == id) else {
            return false;
        };
        let target_idx = from_idx as i32 + delta;
        if !(0..order.len() as i32).contains(&target_idx) {
            return false;
        }
        let Some(tab) = store.tab(id) else {
            return false;
        };
        let favorite = tab.favorite;
        let pinned = tab.pinned;
        let folder = tab.folder;
        let before = if delta < 0 {
            Some(order[target_idx as usize])
        } else {
            order.get(target_idx as usize + 1).copied()
        };
        store.relocate_tab(id, favorite, pinned, folder, before);
        true
    })
    .unwrap_or(false)
}

/// Close a tab. Favorites are dismissed from the strip (browser stays loaded);
/// other tabs are recorded for reopen (⌘⇧T), removed, and torn down.
pub fn close_tab(id: TabId) {
    let browser = structural(|store| {
        if store.tab(id).is_some_and(|t| t.favorite) {
            store.dismiss_favorite(id);
            None
        } else {
            store.remove_tab_recording(id)
        }
    })
    .flatten();
    close_browser(browser);
}

/// Remove a tab the *engine* closed (a page that closed itself), addressed by
/// CEF browser identifier. Recorded for reopen like a user close. A no-op when
/// the browser isn't a registered tab, or its tab was already removed by
/// [`close_tab`].
pub fn note_closed(cef_id: i32) {
    let id = with_store(|store| store.tab_id_by_cef(cef_id)).flatten();
    if let Some(id) = id {
        structural(|store| {
            store.remove_tab_recording(id);
        });
    }
}

/// Reopen the most recently closed tab (⌘⇧T), restoring it into its original
/// space, folder, pinned state, and slot (ADR-0013's suspend/hydrate path). The
/// tab returns as the active tab; the host hydrates it on the emitted
/// `StructureChanged`. Repeated calls walk back the close history. Returns the
/// reopened tab id, or [`NONE_ID`] when nothing is left to reopen.
pub fn reopen_closed_tab() -> TabId {
    structural(|store| store.reopen_closed())
        .flatten()
        .unwrap_or(NONE_ID)
}

/// Whether the reopen stack has anything in it — backs the "Reopen Closed Tab"
/// menu item's enabled state.
pub fn can_reopen_closed() -> bool {
    with_store(|store| !store.closed.is_empty()).unwrap_or(false)
}

// ─── Pinning & folders ─────────────────────────────────────────────────────

/// Pin (persist) or unpin a tab. Unpinning returns it to "today" and clears any
/// folder membership. Saving clears the favorite strip.
pub fn set_pinned(id: TabId, pinned: bool) {
    structural(|store| {
        if let Some(tab) = store.tabs.iter_mut().find(|t| t.id == id) {
            tab.pinned = pinned;
            if pinned {
                tab.favorite = false;
                tab.dismissed = false;
            } else {
                tab.folder = None;
            }
        }
    });
}

/// Pin or unpin a tab in the Arc-style icon strip (ADR-0032). Pinning clears
/// saved/folder membership; unpinning returns the tab to the open section.
pub fn set_favorite(id: TabId, favorite: bool) {
    structural(|store| {
        if let Some(tab) = store.tabs.iter_mut().find(|t| t.id == id) {
            tab.favorite = favorite;
            tab.dismissed = false;
            if favorite {
                tab.pinned = false;
                tab.folder = None;
            }
        }
    });
}

/// Restore a dismissed favorite to the icon strip and make it active.
pub fn restore_favorite(id: TabId) {
    structural(|store| {
        let Some(space) = store.tab(id).filter(|t| t.favorite).map(|t| t.space) else {
            return;
        };
        if let Some(tab) = store.tabs.iter_mut().find(|t| t.id == id) {
            tab.dismissed = false;
        }
        store.active_space = space;
        store.set_space_active(space, Some(id));
    });
}

/// Move a tab into `folder` (or out, with `None`). Moving into a folder
/// auto-pins the tab (folders hold persistent tabs). Ignored if the folder isn't
/// in the tab's space.
pub fn move_to_folder(id: TabId, folder: Option<FolderId>) {
    structural(|store| {
        let Some(space) = store.tab(id).map(|t| t.space) else {
            return;
        };
        let ok = folder.is_none_or(|fid| {
            store
                .folders
                .iter()
                .any(|f| f.id == fid && f.space == space)
        });
        if !ok {
            return;
        }
        if let Some(tab) = store.tabs.iter_mut().find(|t| t.id == id) {
            tab.folder = folder;
            if folder.is_some() {
                tab.pinned = true;
                tab.favorite = false;
                tab.dismissed = false;
            }
        }
    });
}

/// The drag-and-drop primitive: relocate tab `id` into a destination section —
/// pinned/loose or a folder — positioned before `before` (or at the section's
/// end when `None`). Re-parents (pin/unpin, file into/out of a folder) and
/// reorders in one structural change. See [`TabStore::relocate_tab`].
pub fn move_tab(
    id: TabId,
    favorite: bool,
    pinned: bool,
    folder: Option<FolderId>,
    before: Option<TabId>,
) {
    structural(|store| store.relocate_tab(id, favorite, pinned, folder, before));
}

/// Create an empty folder in the active space. Returns its id (or -1).
pub fn create_folder(name: &str) -> FolderId {
    structural(|store| {
        let id = store.next_folder;
        store.next_folder += 1;
        let space = store.active_space;
        store.folders.push(Folder {
            id,
            space,
            name: name.to_string(),
        });
        id
    })
    .unwrap_or(NONE_ID)
}

/// Create a folder in the active space and move `tab_ids` into it (auto-pins).
/// Unknown ids and tabs in other spaces are skipped; relative store order among
/// the moved tabs is preserved. Returns the new folder id (or -1 when `tab_ids`
/// is empty or none are in the active space).
pub fn create_folder_with_tabs(name: &str, tab_ids: &[TabId]) -> FolderId {
    let folder_id = structural(|store| {
        if tab_ids.is_empty() {
            return None;
        }
        let space = store.active_space;
        let wanted: std::collections::HashSet<TabId> = tab_ids
            .iter()
            .copied()
            .filter(|id| store.tabs.iter().any(|t| t.id == *id && t.space == space))
            .collect();
        if wanted.is_empty() {
            return None;
        }
        let folder_id = store.next_folder;
        store.next_folder += 1;
        store.folders.push(Folder {
            id: folder_id,
            space,
            name: name.to_string(),
        });
        let ordered: Vec<TabId> = store
            .tabs
            .iter()
            .filter(|t| wanted.contains(&t.id))
            .map(|t| t.id)
            .collect();
        for id in ordered {
            store.relocate_tab(id, false, true, Some(folder_id), None);
        }
        Some(folder_id)
    })
    .flatten();
    folder_id.unwrap_or(NONE_ID)
}

/// Close ephemeral ("today") tabs in the active space except those in `keep`.
/// When `keep` is empty, the active tab is kept. Bulk close — not reopenable.
pub fn clear_ephemeral_tabs(keep: &[TabId]) {
    let browsers = structural(|store| {
        let space = store.active_space;
        let keep_set: std::collections::HashSet<TabId> = if keep.is_empty() {
            store.visible_tab().into_iter().collect()
        } else {
            keep.iter().copied().collect()
        };
        let victims: Vec<(TabId, Option<Browser>)> = store
            .tabs
            .iter()
            .filter(|t| t.space == space && !t.pinned && !t.favorite && !keep_set.contains(&t.id))
            .map(|t| (t.id, t.browser.clone()))
            .collect();
        for (id, _) in &victims {
            store.remove_tab(*id);
        }
        victims
            .into_iter()
            .filter_map(|(_, b)| b)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    for browser in browsers {
        close_browser(Some(browser));
    }
}

/// Delete a folder, keeping its tabs (they drop to loose pinned — no tab loss).
pub fn delete_folder(id: FolderId) {
    structural(|store| {
        for tab in store.tabs.iter_mut().filter(|t| t.folder == Some(id)) {
            tab.folder = None; // stays pinned
        }
        store.folders.retain(|f| f.id != id);
    });
}

pub fn rename_folder(id: FolderId, name: &str) {
    structural(|store| {
        if let Some(folder) = store.folders.iter_mut().find(|f| f.id == id) {
            folder.name = name.to_string();
        }
    });
}

// ─── Spaces ────────────────────────────────────────────────────────────────

/// Create a new (empty) space and switch to it. Returns its id (or -1).
pub fn create_space(name: &str, icon: &str) -> SpaceId {
    structural(|store| {
        let id = store.next_space;
        store.next_space += 1;
        store.spaces.push(Space {
            id,
            name: name.to_string(),
            icon: icon.to_string(),
            active: None,
        });
        store.active_space = id;
        id
    })
    .unwrap_or(NONE_ID)
}

/// Switch to `id`, restoring its remembered active tab (or the first tab in it).
pub fn select_space(id: SpaceId) {
    structural(|store| {
        if !store.spaces.iter().any(|s| s.id == id) {
            return;
        }
        store.active_space = id;
        let remembered = store
            .spaces
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.active)
            .filter(|tid| store.tabs.iter().any(|t| t.id == *tid && t.space == id));
        let active = remembered.or_else(|| store.first_tab_in(id));
        store.set_space_active(id, active);
    });
}

pub fn rename_space(id: SpaceId, name: &str) {
    structural(|store| {
        if let Some(space) = store.spaces.iter_mut().find(|s| s.id == id) {
            space.name = name.to_string();
        }
    });
}

pub fn set_space_icon(id: SpaceId, icon: &str) {
    structural(|store| {
        if let Some(space) = store.spaces.iter_mut().find(|s| s.id == id) {
            space.icon = icon.to_string();
        }
    });
}

/// Delete a space and close all its tabs. Refuses to delete the last space.
pub fn delete_space(id: SpaceId) {
    let browsers = structural(|store| {
        if store.spaces.len() <= 1 || !store.spaces.iter().any(|s| s.id == id) {
            return Vec::new();
        }
        // Suspended tabs have no browser to close — they are simply removed.
        let browsers: Vec<Browser> = store
            .tabs
            .iter()
            .filter(|t| t.space == id)
            .filter_map(|t| t.browser.clone())
            .collect();
        store.tabs.retain(|t| t.space != id);
        store.folders.retain(|f| f.space != id);
        store.spaces.retain(|s| s.id != id);
        if store.active_space == id {
            let new_space = store.spaces[0].id;
            store.active_space = new_space;
            let active = store
                .spaces
                .iter()
                .find(|s| s.id == new_space)
                .and_then(|s| s.active)
                .filter(|tid| store.tabs.iter().any(|t| t.id == *tid))
                .or_else(|| store.first_tab_in(new_space));
            store.set_space_active(new_space, active);
        }
        browsers
    })
    .unwrap_or_default();
    for browser in browsers {
        close_browser(Some(browser));
    }
}

// ─── Import ────────────────────────────────────────────────────────────────

/// Merge a parsed Arc sidebar into the tab store. `replace` wipes the existing
/// session first (closing every live browser). Imported tabs are suspended until
/// the host hydrates them. Returns `None` when the store isn't initialized.
pub fn apply_arc_import(
    import: &crate::import::ArcImport,
    replace: bool,
) -> Option<crate::import::ApplyResult> {
    let outcome = structural(|store| {
        let mut skipped = 0usize;
        let browsers_to_close: Vec<Browser> = if replace {
            store
                .tabs
                .iter()
                .filter_map(|t| t.browser.clone())
                .collect()
        } else {
            Vec::new()
        };

        if replace {
            store.tabs.clear();
            store.folders.clear();
            store.spaces.clear();
            store.closed.clear();
        }

        let previous_active = store.active_space;
        let mut spaces_added = 0usize;
        let mut folders_added = 0usize;
        let mut tabs_added = 0usize;
        let mut favorites_added = 0usize;
        let mut first_imported_space: Option<SpaceId> = None;

        for space in &import.spaces {
            let space_id = store.next_space;
            store.next_space += 1;
            store.spaces.push(Space {
                id: space_id,
                name: space.name.clone(),
                icon: space.icon.clone(),
                active: None,
            });
            spaces_added += 1;
            if first_imported_space.is_none() {
                first_imported_space = Some(space_id);
            }
            let (folders, tabs) = import_arc_space_content(store, space_id, space, &mut skipped);
            folders_added += folders;
            tabs_added += tabs;
        }

        if let Some(fav_space) = first_imported_space {
            for fav in &import.favorites {
                if space_has_url(store, fav_space, &fav.url) {
                    skipped += 1;
                    continue;
                }
                push_imported_tab(store, fav_space, fav, true, false, None);
                favorites_added += 1;
                tabs_added += 1;
            }
            if replace {
                store.active_space = fav_space;
                let active = store.first_tab_in(fav_space);
                store.set_space_active(fav_space, active);
            } else {
                store.active_space = previous_active;
            }
        } else if replace {
            // Import carried no spaces — fall back to a fresh Home space.
            store.spaces.push(Space {
                id: 1,
                name: "Home".into(),
                icon: "🏠".into(),
                active: None,
            });
            store.active_space = 1;
            store.next_space = 2;
        }

        (
            crate::import::ApplyResult {
                spaces: spaces_added,
                folders: folders_added,
                tabs: tabs_added,
                favorites: favorites_added,
                skipped_duplicates: skipped,
            },
            browsers_to_close,
        )
    })?;

    for browser in outcome.1 {
        close_browser(Some(browser));
    }
    Some(outcome.0)
}

fn import_arc_space_content(
    store: &mut TabStore,
    space_id: SpaceId,
    space: &crate::import::ArcSpace,
    skipped: &mut usize,
) -> (usize, usize) {
    let mut folders_added = 0usize;
    let mut tabs_added = 0usize;

    for folder in &space.folders {
        let folder_id = store.next_folder;
        store.next_folder += 1;
        store.folders.push(Folder {
            id: folder_id,
            space: space_id,
            name: folder.name.clone(),
        });
        folders_added += 1;
        for tab in &folder.tabs {
            if space_has_url(store, space_id, &tab.url) {
                *skipped += 1;
                continue;
            }
            push_imported_tab(store, space_id, tab, false, true, Some(folder_id));
            tabs_added += 1;
        }
    }

    for tab in &space.pinned_tabs {
        if space_has_url(store, space_id, &tab.url) {
            *skipped += 1;
            continue;
        }
        push_imported_tab(store, space_id, tab, false, true, None);
        tabs_added += 1;
    }

    (folders_added, tabs_added)
}

fn space_has_url(store: &TabStore, space: SpaceId, url: &str) -> bool {
    store.tabs.iter().any(|t| t.space == space && t.url == url)
}

fn push_imported_tab(
    store: &mut TabStore,
    space: SpaceId,
    tab: &crate::import::ArcTab,
    favorite: bool,
    pinned: bool,
    folder: Option<FolderId>,
) {
    let id = store.next_tab;
    store.next_tab += 1;
    let (favorite, pinned, folder) = if favorite {
        (true, false, None)
    } else {
        (false, pinned || folder.is_some(), folder)
    };
    let index = store
        .tabs
        .iter()
        .rposition(|t| TabStore::same_section(t, space, favorite, pinned, folder))
        .map_or(store.tabs.len(), |p| p + 1);
    store.tabs.insert(
        index,
        Tab {
            id,
            space,
            folder,
            pinned,
            favorite,
            dismissed: false,
            browser: None,
            url: tab.url.clone(),
            title: tab.title.clone(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            last_shown_at: 0,
        },
    );
}

// ─── Folder ↔ space transforms (ADR-0006) ──────────────────────────────────

/// Promote a folder to its own space: its (pinned) tabs become the new space's
/// pinned tabs, the folder is removed, and we switch to the new space.
pub fn folder_to_space(folder_id: FolderId) -> SpaceId {
    structural(|store| {
        let Some(name) = store
            .folders
            .iter()
            .find(|f| f.id == folder_id)
            .map(|f| f.name.clone())
        else {
            return NONE_ID;
        };
        let sid = store.next_space;
        store.next_space += 1;
        store.spaces.push(Space {
            id: sid,
            name,
            icon: DEFAULT_SPACE_ICON.to_string(),
            active: None,
        });
        let mut first = None;
        for tab in store
            .tabs
            .iter_mut()
            .filter(|t| t.folder == Some(folder_id))
        {
            tab.space = sid;
            tab.folder = None;
            tab.pinned = true;
            tab.favorite = false;
            tab.dismissed = false;
            first.get_or_insert(tab.id);
        }
        store.folders.retain(|f| f.id != folder_id);
        store.set_space_active(sid, first);
        store.active_space = sid;
        sid
    })
    .unwrap_or(NONE_ID)
}

/// Demote a space to a folder inside the active space: its **pinned** tabs
/// become a new folder there; its **ephemeral** tabs are closed (only persistent
/// survives); the emptied space is deleted. Only valid for a non-active space
/// (the target is the active space). Returns the new folder id (or -1).
pub fn space_to_folder(space_id: SpaceId) -> FolderId {
    let (folder_id, ephemeral) = structural(|store| {
        if space_id == store.active_space || !store.spaces.iter().any(|s| s.id == space_id) {
            return (NONE_ID, Vec::new());
        }
        let target = store.active_space;
        let name = store
            .spaces
            .iter()
            .find(|s| s.id == space_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let folder_id = store.next_folder;
        store.next_folder += 1;
        store.folders.push(Folder {
            id: folder_id,
            space: target,
            name,
        });
        // Close the source space's ephemeral tabs — only persistent survives.
        // (Suspended ones have no browser; removal below is all they need.)
        let ephemeral: Vec<Browser> = store
            .tabs
            .iter()
            .filter(|t| t.space == space_id && !t.pinned)
            .filter_map(|t| t.browser.clone())
            .collect();
        store.tabs.retain(|t| t.space != space_id || t.pinned);
        // The remaining source tabs are all pinned; flatten them into the new
        // folder in the target space.
        for tab in store.tabs.iter_mut().filter(|t| t.space == space_id) {
            tab.space = target;
            tab.folder = Some(folder_id);
            tab.pinned = true;
            tab.favorite = false;
            tab.dismissed = false;
        }
        store.folders.retain(|f| f.space != space_id);
        store.spaces.retain(|s| s.id != space_id);
        (folder_id, ephemeral)
    })
    .unwrap_or((NONE_ID, Vec::new()));
    for browser in ephemeral {
        close_browser(Some(browser));
    }
    folder_id
}

// ─── Content updates (fine-grained events) ─────────────────────────────────
//
// All addressed by CEF browser identifier (they are driven from CEF handlers);
// a browser that isn't a hydrated tab (popup / DevTools) translates to no tab
// and the update is dropped. Title and URL changes are persisted — they are
// what a restored tab shows and loads (ADR-0013); loading/favicon are
// transient and are not.

/// Update a hydrated tab's field via `mutate` (which returns whether anything
/// changed) and hand back its tab id when it did.
fn note_content(cef_id: i32, mutate: impl FnOnce(&mut Tab) -> bool) -> Option<TabId> {
    with_store_mut(|store| {
        let tab = store
            .tabs
            .iter_mut()
            .find(|t| t.browser.as_ref().is_some_and(|b| b.identifier() == cef_id))?;
        mutate(tab).then_some(tab.id)
    })
}

pub fn note_title(cef_id: i32, title: &str) {
    let changed = note_content(cef_id, |tab| {
        let changed = tab.title != title;
        if changed {
            tab.title = title.to_string();
        }
        changed
    });
    if let Some(id) = changed {
        emit(Event::TabTitleChanged, id, Some(title));
        persist();
    }
}

pub fn note_url(cef_id: i32, url: &str) {
    let changed = note_content(cef_id, |tab| {
        let changed = tab.url != url;
        if changed {
            tab.url = url.to_string();
        }
        changed
    });
    if let Some(id) = changed {
        emit(Event::TabUrlChanged, id, Some(url));
        persist();
    }
}

/// Ingest a sampled page theme/background color for viewport URL-bar tinting.
/// Fire-and-forget — the host stores it per tab; not session-persisted.
pub fn note_theme_color(cef_id: i32, color: &str) {
    let id = note_content(cef_id, |_| true);
    if let Some(id) = id {
        emit(Event::TabThemeColorChanged, id, Some(color));
    }
}

/// Ingest CEF's `on_loading_state_change` for a tab: update the spinner
/// (`loading`) and the back/forward enablement together — they arrive in one
/// callback — emitting each event only when its own value actually changed.
pub fn note_load_state(cef_id: i32, loading: bool, can_go_back: bool, can_go_forward: bool) {
    let result = with_store_mut(|store| {
        let tab = store
            .tabs
            .iter_mut()
            .find(|t| t.browser.as_ref().is_some_and(|b| b.identifier() == cef_id))?;
        let loading_changed = tab.loading != loading;
        let nav_changed = tab.can_go_back != can_go_back || tab.can_go_forward != can_go_forward;
        tab.loading = loading;
        tab.can_go_back = can_go_back;
        tab.can_go_forward = can_go_forward;
        Some((tab.id, loading_changed, nav_changed))
    });
    let Some((id, loading_changed, nav_changed)) = result else {
        return;
    };
    if loading_changed {
        emit(
            if loading {
                Event::TabLoadStarted
            } else {
                Event::TabLoadEnded
            },
            id,
            None,
        );
    }
    if nav_changed {
        let code = format!("{}{}", u8::from(can_go_back), u8::from(can_go_forward));
        emit(Event::TabNavStateChanged, id, Some(&code));
    }
}

pub fn note_favicon(cef_id: i32, host: &str) {
    let id = with_store(|store| store.tab_id_by_cef(cef_id)).flatten();
    if let Some(id) = id {
        emit(Event::TabFaviconChanged, id, Some(host));
    }
}

/// Mutable read for the fine-grained content updates (no visibility / structure
/// reconciliation — those don't change here).
fn with_store_mut<R: Default>(f: impl FnOnce(&mut TabStore) -> R) -> R {
    let mut guard = STORE.lock().expect("tab store lock poisoned");
    guard.as_mut().map(f).unwrap_or_default()
}

// ─── Queries ───────────────────────────────────────────────────────────────

/// The active tab's browser handle, backing the active-tab-targeted FFI verbs.
pub fn active_browser() -> Option<Browser> {
    with_store(|store| store.visible_tab().and_then(|id| store.browser_of(id))).flatten()
}

/// The visible tab's host, if any.
pub fn active_tab_host() -> Option<String> {
    with_store(|store| {
        let id = store.visible_tab()?;
        let url = store.tab(id)?.url.as_str();
        crate::vault::lookup::host_from_uri(url)
    })
    .flatten()
}

/// Close a browser through CEF's (force) close flow, tolerating `None` / an
/// already-invalid host.
fn close_browser(browser: Option<Browser>) {
    if let Some(host) = browser.and_then(|b| b.host()) {
        host.close_browser(1);
    }
}

// ─── Snapshot (ADR-0005) ───────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Snapshot {
    pub active_space: SpaceId,
    pub active_tab: Option<TabId>,
    pub spaces: Vec<SpaceView>,
    pub folders: Vec<FolderView>,
    pub tabs: Vec<TabView>,
}

#[derive(Serialize)]
pub struct SpaceView {
    pub id: SpaceId,
    pub name: String,
    pub icon: String,
}

#[derive(Serialize)]
pub struct FolderView {
    pub id: FolderId,
    pub space: SpaceId,
    pub name: String,
}

#[derive(Serialize)]
pub struct TabView {
    pub id: TabId,
    pub space: SpaceId,
    pub folder: Option<FolderId>,
    pub pinned: bool,
    pub favorite: bool,
    pub dismissed: bool,
    /// `false` when the tab is suspended (`browser: None`) — runtime discard or
    /// lazy session restore (ADR-0013 / ADR-0033).
    pub hydrated: bool,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

/// A full picture of the current structure, for the host to rebuild its tree on
/// `StructureChanged` (ADR-0005).
pub fn snapshot() -> Snapshot {
    with_store(|store| Snapshot {
        active_space: store.active_space,
        active_tab: store.visible_tab(),
        spaces: store
            .spaces
            .iter()
            .map(|s| SpaceView {
                id: s.id,
                name: s.name.clone(),
                icon: s.icon.clone(),
            })
            .collect(),
        folders: store
            .folders
            .iter()
            .map(|f| FolderView {
                id: f.id,
                space: f.space,
                name: f.name.clone(),
            })
            .collect(),
        tabs: store
            .tabs
            .iter()
            .map(|t| TabView {
                id: t.id,
                space: t.space,
                folder: t.folder,
                pinned: t.pinned,
                favorite: t.favorite,
                dismissed: t.dismissed,
                hydrated: t.browser.is_some(),
                url: t.url.clone(),
                title: t.title.clone(),
                loading: t.loading,
                can_go_back: t.can_go_back,
                can_go_forward: t.can_go_forward,
            })
            .collect(),
    })
    .unwrap_or(Snapshot {
        active_space: 0,
        active_tab: None,
        spaces: Vec::new(),
        folders: Vec::new(),
        tabs: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests exercise the persistence/restore logic on `TabStore` values
    // directly — restored tabs are suspended (`browser: None`), so no CEF
    // objects (and none of the global statics) are involved.

    fn saved(
        active_space: SpaceId,
        spaces: Vec<(SpaceId, Option<TabId>)>,
        folders: Vec<(FolderId, SpaceId)>,
        tabs: Vec<(TabId, SpaceId, Option<FolderId>, bool)>,
    ) -> SavedState {
        SavedState {
            active_space,
            spaces: spaces
                .into_iter()
                .map(|(id, active)| SavedSpace {
                    id,
                    name: format!("space{id}"),
                    icon: "✨".into(),
                    active,
                })
                .collect(),
            folders: folders
                .into_iter()
                .map(|(id, space)| SavedFolder {
                    id,
                    space,
                    name: format!("folder{id}"),
                })
                .collect(),
            tabs: tabs
                .into_iter()
                .map(|(id, space, folder, pinned)| SavedTab {
                    id,
                    space,
                    folder,
                    pinned,
                    favorite: false,
                    dismissed: false,
                    url: format!("https://t{id}.example/"),
                    title: format!("t{id}"),
                })
                .collect(),
        }
    }

    #[test]
    fn from_saved_restores_tree_and_resumes_counters() {
        let store = TabStore::from_saved(saved(
            2,
            vec![(1, Some(4)), (2, Some(9))],
            vec![(3, 1)],
            vec![(4, 1, Some(3), true), (9, 2, None, false)],
        ));
        assert_eq!(store.active_space, 2);
        assert_eq!(store.spaces.len(), 2);
        assert_eq!(store.tabs.len(), 2);
        assert_eq!(store.folders.len(), 1);
        // Every restored tab is suspended.
        assert!(store.tabs.iter().all(|t| t.browser.is_none()));
        // Counters resume past the highest persisted ids.
        assert_eq!(store.next_space, 3);
        assert_eq!(store.next_folder, 4);
        assert_eq!(store.next_tab, 10);
        // Active pointers survive.
        assert_eq!(store.visible_tab(), Some(9));
    }

    #[test]
    fn from_saved_empty_spaces_falls_back_to_default() {
        let store = TabStore::from_saved(saved(1, vec![], vec![], vec![]));
        assert_eq!(store.spaces.len(), 1);
        assert_eq!(store.spaces[0].name, "Home");
        assert_eq!(store.active_space, 1);
        assert_eq!(store.next_tab, 1);
    }

    #[test]
    fn from_saved_sanitizes_dangling_references() {
        let store = TabStore::from_saved(saved(
            99,                  // active space doesn't exist
            vec![(1, Some(42))], // active tab doesn't exist
            vec![(3, 8)],        // folder's space doesn't exist
            vec![
                (4, 1, Some(3), false), // folder ref dangles (dropped folder)
                (5, 8, None, false),    // tab's space doesn't exist
                (6, 1, Some(77), true), // folder never existed
            ],
        ));
        assert_eq!(store.active_space, 1); // fell back to the first space
        assert!(store.folders.is_empty()); // dangling folder dropped
        assert_eq!(store.tabs.len(), 2); // orphaned-space tab dropped
        assert!(store.tabs.iter().all(|t| t.folder.is_none())); // refs cleared
                                                                // Space active fell back to its first tab.
        assert_eq!(store.visible_tab(), Some(4));
    }

    #[test]
    fn from_saved_folder_membership_implies_pinned() {
        let store = TabStore::from_saved(saved(
            1,
            vec![(1, None)],
            vec![(2, 1)],
            vec![(3, 1, Some(2), false)], // saved unpinned but in a folder
        ));
        assert!(store.tabs[0].pinned);
        assert_eq!(store.tabs[0].folder, Some(2));
    }

    /// The write-through + shutdown mask, against the real global store: a
    /// structural mutation persists the session; once [`begin_shutdown`] has
    /// run, further mutations (the app-quit browser teardown) do not. The only
    /// test that touches the module globals — everything else works on
    /// `TabStore` values — so it can't race another test over them.
    #[test]
    fn persist_writes_through_until_shutdown() {
        with_global_store_test_lock(|| {
            let dir = std::env::temp_dir().join("browser-tabs-test-persist");
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("session.json");
            let path_str = path.to_str().unwrap();

            test_reset_store(path_str);
            let space = create_space("Work", "💼");
            let state = session::load(path_str).expect("mutation persisted the session");
            assert!(state.spaces.iter().any(|s| s.id == space));

            begin_shutdown();
            rename_space(space, "gone");
            let state = session::load(path_str).expect("file still present");
            assert!(
                state.spaces.iter().all(|s| s.name != "gone"),
                "post-shutdown mutation must not be persisted"
            );
            std::fs::remove_dir_all(&dir).ok();
        });
    }

    #[test]
    fn saved_state_mirrors_store_roundtrip() {
        let original = saved(
            1,
            vec![(1, Some(4)), (2, None)],
            vec![(3, 1)],
            vec![(4, 1, Some(3), true), (5, 2, None, false)],
        );
        let store = TabStore::from_saved(original);
        let out = store.saved_state();
        let rebuilt = TabStore::from_saved(out);
        assert_eq!(rebuilt.active_space, store.active_space);
        assert_eq!(rebuilt.tabs.len(), store.tabs.len());
        assert_eq!(rebuilt.folders.len(), store.folders.len());
        assert_eq!(rebuilt.spaces.len(), store.spaces.len());
        assert_eq!(rebuilt.visible_tab(), store.visible_tab());
    }

    /// A section is the `tabs` order filtered per section; collect the ids of the
    /// today (unpinned, loose) tabs of space 1 in store order.
    fn today_ids(store: &TabStore) -> Vec<TabId> {
        store
            .tabs
            .iter()
            .filter(|t| t.space == 1 && !t.pinned && !t.favorite && t.folder.is_none())
            .map(|t| t.id)
            .collect()
    }

    fn favorite_ids(store: &TabStore) -> Vec<TabId> {
        store
            .tabs
            .iter()
            .filter(|t| t.space == 1 && t.favorite && !t.dismissed)
            .map(|t| t.id)
            .collect()
    }

    #[test]
    fn relocate_reorders_within_a_section() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, None)],
            vec![],
            vec![
                (1, 1, None, false),
                (2, 1, None, false),
                (3, 1, None, false),
            ],
        ));
        // Drag tab 3 above tab 1.
        store.relocate_tab(3, false, false, None, Some(1));
        assert_eq!(today_ids(&store), vec![3, 1, 2]);
        // Drag tab 3 to the section's end (no anchor).
        store.relocate_tab(3, false, false, None, None);
        assert_eq!(today_ids(&store), vec![1, 2, 3]);
    }

    #[test]
    fn relocate_pins_and_files_into_a_folder() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, None)],
            vec![(5, 1)],
            vec![(2, 1, None, false)], // an ephemeral "today" tab
        ));
        // Drag today tab 2 onto folder 5: auto-pins and joins the folder.
        store.relocate_tab(2, false, false, Some(5), None);
        let t = store.tabs.iter().find(|t| t.id == 2).unwrap();
        assert!(t.pinned);
        assert_eq!(t.folder, Some(5));
        assert!(today_ids(&store).is_empty());
    }

    #[test]
    fn relocate_unpins_back_to_today() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, None)],
            vec![(5, 1)],
            vec![
                (2, 1, Some(5), true), // pinned, in folder 5
                (3, 1, None, false),   // a today tab
            ],
        ));
        // Drag the folder tab down into the today section (no anchor → its end).
        store.relocate_tab(2, false, false, None, None);
        let t = store.tabs.iter().find(|t| t.id == 2).unwrap();
        assert!(!t.pinned);
        assert_eq!(t.folder, None);
        assert_eq!(today_ids(&store), vec![3, 2]);
    }

    #[test]
    fn create_folder_with_tabs_moves_in_store_order() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, None)],
            vec![],
            vec![(2, 1, None, false), (3, 1, None, true), (4, 1, None, false)],
        ));
        let folder_id = {
            let id = store.next_folder;
            store.next_folder += 1;
            store.folders.push(Folder {
                id,
                space: 1,
                name: "Batch".to_string(),
            });
            let ordered: Vec<TabId> = store
                .tabs
                .iter()
                .filter(|t| [4, 2].contains(&t.id))
                .map(|t| t.id)
                .collect();
            for tab_id in ordered {
                store.relocate_tab(tab_id, false, true, Some(id), None);
            }
            id
        };
        assert_eq!(folder_id, 1);
        let folder_tabs: Vec<TabId> = store
            .tabs
            .iter()
            .filter(|t| t.folder == Some(folder_id))
            .map(|t| t.id)
            .collect();
        assert_eq!(folder_tabs, vec![2, 4]);
        assert!(store.tabs.iter().find(|t| t.id == 2).unwrap().pinned);
        assert_eq!(today_ids(&store), Vec::<TabId>::new());
    }

    #[test]
    fn clear_ephemeral_tabs_keeps_active_when_keep_empty() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, Some(2))],
            vec![],
            vec![
                (1, 1, None, false),
                (2, 1, None, false),
                (3, 1, None, false),
            ],
        ));
        let keep: std::collections::HashSet<TabId> = store.visible_tab().into_iter().collect();
        let victims: Vec<TabId> = store
            .tabs
            .iter()
            .filter(|t| t.space == 1 && !t.pinned && !keep.contains(&t.id))
            .map(|t| t.id)
            .collect();
        for id in victims {
            store.remove_tab(id);
        }
        assert_eq!(today_ids(&store), vec![2]);
        assert_eq!(store.visible_tab(), Some(2));
    }

    #[test]
    fn clear_ephemeral_tabs_keeps_explicit_selection() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, Some(2))],
            vec![],
            vec![
                (1, 1, None, false),
                (2, 1, None, false),
                (3, 1, None, false),
                (4, 1, None, false),
            ],
        ));
        let keep: std::collections::HashSet<TabId> = [1, 3].into_iter().collect();
        let victims: Vec<TabId> = store
            .tabs
            .iter()
            .filter(|t| t.space == 1 && !t.pinned && !keep.contains(&t.id))
            .map(|t| t.id)
            .collect();
        for id in victims {
            store.remove_tab(id);
        }
        assert_eq!(today_ids(&store), vec![1, 3]);
        // Active tab (2) was not in `keep` — it closes with the rest.
        assert_eq!(store.visible_tab(), Some(1));
    }

    #[test]
    fn relocate_rejects_a_foreign_folder() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, None), (2, None)],
            vec![(5, 2)],              // folder 5 lives in space 2
            vec![(3, 1, None, false)], // tab 3 is in space 1
        ));
        store.relocate_tab(3, false, false, Some(5), None);
        let t = store.tabs.iter().find(|t| t.id == 3).unwrap();
        assert!(!t.pinned); // unchanged — the cross-space folder was rejected
        assert_eq!(t.folder, None);
    }

    // ─── Reopen closed tab (⌘⇧T) ────────────────────────────────────────────

    #[test]
    fn close_then_reopen_restores_tab_in_its_slot() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, Some(2))],
            vec![],
            vec![
                (1, 1, None, false),
                (2, 1, None, false),
                (3, 1, None, false),
            ],
        ));
        // Close the middle today tab (which was active).
        store.remove_tab_recording(2);
        assert_eq!(today_ids(&store), vec![1, 3]);
        // Reopen: back in place (before its old sibling 3), identity intact, and
        // now the active tab.
        assert_eq!(store.reopen_closed(), Some(2));
        assert_eq!(today_ids(&store), vec![1, 2, 3]);
        assert_eq!(store.visible_tab(), Some(2));
        // The stack is a stack: nothing left to reopen.
        assert_eq!(store.reopen_closed(), None);
    }

    #[test]
    fn reopen_restores_pinned_and_folder_membership() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, None)],
            vec![(5, 1)],
            vec![(2, 1, Some(5), true)], // a pinned tab filed in folder 5
        ));
        store.remove_tab_recording(2);
        assert!(store.tabs.is_empty());
        assert_eq!(store.reopen_closed(), Some(2));
        let t = store.tabs.iter().find(|t| t.id == 2).unwrap();
        assert!(t.pinned);
        assert_eq!(t.folder, Some(5));
        assert!(t.browser.is_none()); // comes back suspended, hydrated on show
    }

    #[test]
    fn reopen_heals_when_space_and_folder_are_gone() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, None), (2, None)],
            vec![(5, 2)],
            vec![(3, 2, Some(5), true)], // pinned, in folder 5 of space 2
        ));
        store.remove_tab_recording(3);
        // Space 2 (and its folder) are deleted before the reopen.
        store.tabs.retain(|t| t.space != 2);
        store.folders.retain(|f| f.space != 2);
        store.spaces.retain(|s| s.id != 2);
        assert_eq!(store.reopen_closed(), Some(3));
        let t = store.tabs.iter().find(|t| t.id == 3).unwrap();
        assert_eq!(t.space, store.active_space); // fell back to the active space
        assert_eq!(t.folder, None); // folder gone → dropped to loose pinned
        assert!(t.pinned); // still pinned, just without a folder
        assert_eq!(store.visible_tab(), Some(3));
    }

    #[test]
    fn favorite_dismiss_keeps_tab_and_browser_slot() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, Some(2))],
            vec![],
            vec![(1, 1, None, false), (2, 1, None, false)],
        ));
        store.tabs.iter_mut().find(|t| t.id == 2).unwrap().favorite = true;
        store.dismiss_favorite(2);
        assert_eq!(store.tabs.len(), 2);
        let t = store.tabs.iter().find(|t| t.id == 2).unwrap();
        assert!(t.favorite);
        assert!(t.dismissed);
        assert_eq!(store.visible_tab(), Some(1));
        assert!(favorite_ids(&store).is_empty());
    }

    #[test]
    fn relocate_into_favorite_section() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, None)],
            vec![],
            vec![(2, 1, None, false), (3, 1, None, false)],
        ));
        store.relocate_tab(3, true, false, None, None);
        let t = store.tabs.iter().find(|t| t.id == 3).unwrap();
        assert!(t.favorite);
        assert!(!t.pinned);
        assert_eq!(favorite_ids(&store), vec![3]);
        assert_eq!(today_ids(&store), vec![2]);
    }

    #[test]
    fn closed_stack_is_capped_and_evicts_oldest() {
        let mut store = TabStore::default_store();
        let n = CLOSED_STACK_CAP + 5;
        for id in 1..=n {
            store.tabs.push(Tab {
                id: id as TabId,
                space: 1,
                folder: None,
                pinned: false,
                favorite: false,
                dismissed: false,
                browser: None,
                url: String::new(),
                title: String::new(),
                loading: false,
                can_go_back: false,
                can_go_forward: false,
                last_shown_at: 0,
            });
        }
        for id in 1..=n {
            store.remove_tab_recording(id as TabId);
        }
        assert_eq!(store.closed.len(), CLOSED_STACK_CAP);
        // Newest survives at the top; the oldest 5 closes were evicted.
        assert_eq!(store.reopen_closed(), Some(n as TabId));
    }

    #[test]
    fn sidebar_tab_order_follows_host_layout() {
        let store = TabStore::from_saved(saved(
            1,
            vec![(1, Some(4))],
            vec![(10, 1)],
            vec![
                (1, 1, None, true),     // favorite (pinned flag ignored)
                (2, 1, Some(10), true), // folder tab
                (3, 1, None, true),     // loose saved
                (4, 1, None, false),    // open
            ],
        ));
        // Favorites are modeled with `favorite: true` in from_saved — override.
        let mut store = store;
        store.tabs[0].favorite = true;
        store.tabs[0].pinned = false;
        assert_eq!(store.sidebar_tab_order(1), vec![1, 2, 3, 4]);
    }

    #[test]
    fn move_visible_tab_reorders_in_sidebar_order() {
        let mut store = TabStore::from_saved(saved(
            1,
            vec![(1, Some(2))],
            vec![],
            vec![
                (1, 1, None, false),
                (2, 1, None, false),
                (3, 1, None, false),
            ],
        ));
        store.set_space_active(1, Some(2));
        let order = store.sidebar_tab_order(1);
        assert_eq!(order, vec![1, 2, 3]);
        let id = store.visible_tab().unwrap();
        let from_idx = order.iter().position(|&t| t == id).unwrap();
        let target_idx = from_idx as i32 + 1;
        let tab = store.tab(id).unwrap();
        let before = order.get(target_idx as usize + 1).copied();
        store.relocate_tab(id, tab.favorite, tab.pinned, tab.folder, before);
        assert_eq!(store.sidebar_tab_order(1), vec![1, 3, 2]);
    }
}
