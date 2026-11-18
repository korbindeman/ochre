//! Private (incognito) windows - ephemeral tab sets in isolated WebKit profiles.
//!
//! Each private window owns a flat list of tabs backed by its own
//! non-persistent `WKWebsiteDataStore` ([`crate::engine::Profile::Private`]):
//! cookies and storage live in memory and vanish with the window.
//! Nothing is written to history, session restore, or the persisted adblock
//! allowlist. Per-site shield overrides live only in the window's RAM.

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

use crate::engine::{Browser, Profile};
use crate::tabs::{Event, EventCallback, TabId, NONE_ID};

pub type WindowId = i32;

/// Normal browsing uses window id `0`; private windows start at `1`.
pub const NORMAL_WINDOW_ID: WindowId = 0;

/// Tab ids for private windows start here so they never collide with the main
/// store's counter on the wire.
const PRIVATE_TAB_ID_BASE: TabId = 1_000_000;

static CALLBACK: OnceLock<Mutex<Option<EventCallback>>> = OnceLock::new();
static WINDOWS: OnceLock<Mutex<HashMap<WindowId, PrivateWindow>>> = OnceLock::new();

fn windows() -> &'static Mutex<HashMap<WindowId, PrivateWindow>> {
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}
static NEXT_WINDOW: Mutex<WindowId> = Mutex::new(1);
static NEXT_TAB: Mutex<TabId> = Mutex::new(PRIVATE_TAB_ID_BASE);
static FOCUSED: Mutex<WindowId> = Mutex::new(NORMAL_WINDOW_ID);

struct PrivateTab {
    id: TabId,
    browser: Option<Browser>,
    url: String,
    title: String,
    loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
}

pub struct PrivateWindow {
    id: WindowId,
    tabs: Vec<PrivateTab>,
    active_tab: Option<TabId>,
    /// Ephemeral per-site adblock allowlist (registrable domains).
    allowlist: Vec<String>,
}

#[derive(Serialize)]
pub struct PrivateSnapshot {
    pub window_id: WindowId,
    pub active_tab: Option<TabId>,
    pub tabs: Vec<PrivateTabView>,
}

#[derive(Serialize)]
pub struct PrivateTabView {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

fn callback_slot() -> &'static Mutex<Option<EventCallback>> {
    CALLBACK.get_or_init(|| Mutex::new(None))
}

/// Register the host event callback (shared with [`crate::tabs`]).
pub fn set_event_callback(callback: Option<EventCallback>) {
    *callback_slot().lock().unwrap_or_else(|p| p.into_inner()) = callback;
}

fn emit(window_id: WindowId, event: Event, tab_id: i32, payload: Option<&str>) {
    let callback = *callback_slot().lock().unwrap_or_else(|p| p.into_inner());
    let Some(callback) = callback else {
        return;
    };
    match payload {
        None => callback(event as i32, window_id, tab_id, std::ptr::null()),
        Some(text) => match CString::new(text) {
            Ok(c) => callback(event as i32, window_id, tab_id, c.as_ptr()),
            Err(_) => callback(event as i32, window_id, tab_id, std::ptr::null()),
        },
    }
}

/// Allocate a new private window (incognito profile + empty tab list). Returns its
/// id (`>= 1`). Does not create a tab yet.
pub fn create_window() -> WindowId {
    let id = {
        let mut next = NEXT_WINDOW.lock().unwrap_or_else(|p| p.into_inner());
        let id = *next;
        *next += 1;
        id
    };
    let window = PrivateWindow {
        id,
        tabs: Vec::new(),
        active_tab: None,
        allowlist: Vec::new(),
    };
    windows()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(id, window);
    id
}

pub fn window_count() -> usize {
    windows().lock().unwrap_or_else(|p| p.into_inner()).len()
}

pub fn all_window_ids() -> Vec<WindowId> {
    windows()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .keys()
        .copied()
        .collect()
}

pub fn is_private_window(window_id: WindowId) -> bool {
    window_id != NORMAL_WINDOW_ID
}

pub fn set_focused_window(window_id: WindowId) {
    *FOCUSED.lock().unwrap_or_else(|p| p.into_inner()) = window_id;
}

pub fn focused_window() -> WindowId {
    *FOCUSED.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn focused_is_private() -> bool {
    focused_window() != NORMAL_WINDOW_ID
}

fn with_window<R>(id: WindowId, f: impl FnOnce(&PrivateWindow) -> R) -> Option<R> {
    windows()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&id)
        .map(f)
}

fn with_window_mut<R>(id: WindowId, f: impl FnOnce(&mut PrivateWindow) -> R) -> Option<R> {
    windows()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_mut(&id)
        .map(f)
}

fn allowlist_key(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    match psl::domain(host.as_bytes()) {
        Some(domain) => String::from_utf8_lossy(domain.as_bytes()).into_owned(),
        None => host,
    }
}

/// Ephemeral allowlist for a web view id, if it belongs to a private tab.
pub fn ephemeral_allowlist_for_cef(cef_id: i32) -> Option<Vec<String>> {
    let guard = windows().lock().unwrap_or_else(|p| p.into_inner());
    for window in guard.values() {
        if window
            .tabs
            .iter()
            .any(|t| t.browser.as_ref().is_some_and(|b| b.identifier() == cef_id))
        {
            return Some(window.allowlist.clone());
        }
    }
    None
}

pub fn is_private_cef(cef_id: i32) -> bool {
    ephemeral_allowlist_for_cef(cef_id).is_some()
}

fn window_id_for_cef(cef_id: i32) -> Option<WindowId> {
    let guard = windows().lock().unwrap_or_else(|p| p.into_inner());
    for window in guard.values() {
        if window
            .tabs
            .iter()
            .any(|t| t.browser.as_ref().is_some_and(|b| b.identifier() == cef_id))
        {
            return Some(window.id);
        }
    }
    None
}

/// Notify the host that a private-window tab entered/left browser fullscreen.
/// Returns `true` when `cef_id` belongs to a private window.
pub fn note_fullscreen(cef_id: i32, fullscreen: bool) -> bool {
    let Some(window_id) = window_id_for_cef(cef_id) else {
        return false;
    };
    emit(
        window_id,
        Event::FullscreenChanged,
        i32::from(fullscreen),
        None,
    );
    true
}

/// Create a tab in private window `window_id`. Returns the tab id or [`NONE_ID`].
///
/// # Safety
/// `parent_view` must be a valid `NSView*` that outlives the tab.
pub unsafe fn create_tab(
    window_id: WindowId,
    parent_view: *mut std::ffi::c_void,
    width: i32,
    height: i32,
    url: &str,
) -> TabId {
    let Some(browser) = create_tab_browser(window_id, parent_view, width, height, url) else {
        return NONE_ID;
    };
    register_tab(window_id, browser, url)
}

unsafe fn create_tab_browser(
    window_id: WindowId,
    parent_view: *mut std::ffi::c_void,
    width: i32,
    height: i32,
    url: &str,
) -> Option<Browser> {
    with_window(window_id, |_| ())?;
    crate::engine::create(parent_view, width, height, url, Profile::Private(window_id))
}

fn register_tab(window_id: WindowId, browser: Browser, url: &str) -> TabId {
    let id = {
        let mut next = NEXT_TAB.lock().unwrap_or_else(|p| p.into_inner());
        let id = *next;
        *next += 1;
        id
    };
    with_window_mut(window_id, |window| {
        window.tabs.push(PrivateTab {
            id,
            browser: Some(browser),
            url: url.to_string(),
            title: String::new(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        });
        window.active_tab = Some(id);
    });
    emit(window_id, Event::StructureChanged, 0, None);
    id
}

pub fn select_tab(window_id: WindowId, id: TabId) {
    let changed = with_window_mut(window_id, |window| {
        if !window.tabs.iter().any(|t| t.id == id) {
            return false;
        }
        window.active_tab = Some(id);
        true
    })
    .unwrap_or(false);
    if changed {
        reconcile_visibility(window_id);
        emit(window_id, Event::StructureChanged, 0, None);
    }
}

/// Select a tab by its 1-based position (`1`…`8`, or `9` = last) in a private
/// window's flat tab list.
pub fn select_tab_by_index(window_id: WindowId, index: u8) -> bool {
    let id = with_window(window_id, |window| {
        if window.tabs.is_empty() {
            return None;
        }
        let idx = if index == 9 {
            window.tabs.len() - 1
        } else if (1..=8).contains(&index) {
            index as usize - 1
        } else {
            return None;
        };
        if idx >= window.tabs.len() {
            return None;
        }
        Some(window.tabs[idx].id)
    })
    .flatten();
    let Some(id) = id else {
        return false;
    };
    select_tab(window_id, id);
    true
}

/// Select the next (`delta = 1`) or previous (`delta = -1`) tab in a private
/// window, wrapping at the ends.
pub fn select_adjacent_tab(window_id: WindowId, delta: i32) -> bool {
    if !matches!(delta, -1 | 1) {
        return false;
    }
    let id = with_window(window_id, |window| {
        if window.tabs.is_empty() {
            return None;
        }
        let order: Vec<TabId> = window.tabs.iter().map(|t| t.id).collect();
        let next = match window
            .active_tab
            .and_then(|id| order.iter().position(|&t| t == id))
        {
            None => order[0],
            Some(pos) => {
                if order.len() < 2 {
                    return None;
                }
                let len = order.len() as i32;
                let next_pos = (pos as i32 + delta).rem_euclid(len) as usize;
                order[next_pos]
            }
        };
        Some(next)
    })
    .flatten();
    let Some(id) = id else {
        return false;
    };
    select_tab(window_id, id);
    true
}

fn reconcile_visibility(window_id: WindowId) {
    let visible = with_window(window_id, |w| w.active_tab).flatten();
    with_window_mut(window_id, |window| {
        for tab in &window.tabs {
            let Some(browser) = tab.browser.as_ref() else {
                continue;
            };
            let show = visible == Some(tab.id);
            set_visible(browser, show);
        }
    });
}

fn set_visible(browser: &Browser, visible: bool) {
    browser.set_visible(visible);
}

pub fn close_tab(window_id: WindowId, id: TabId) {
    let browser = with_window_mut(window_id, |window| {
        let idx = window.tabs.iter().position(|t| t.id == id)?;
        let tab = window.tabs.remove(idx);
        if window.active_tab == Some(id) {
            window.active_tab = window.tabs.last().map(|t| t.id);
        }
        tab.browser
    })
    .flatten();
    if let Some(browser) = browser {
        browser.close();
    }
    emit(window_id, Event::StructureChanged, 0, None);
}

/// Close every tab in a private window and remove the window record.
pub fn destroy_window(window_id: WindowId) {
    let tabs: Vec<Option<Browser>> = with_window_mut(window_id, |window| {
        window.tabs.drain(..).map(|t| t.browser).collect()
    })
    .unwrap_or_default();
    for browser in tabs.into_iter().flatten() {
        browser.close();
    }
    windows()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&window_id);
    crate::engine::drop_private_profile(window_id);
    if focused_window() == window_id {
        set_focused_window(NORMAL_WINDOW_ID);
    }
}

pub fn active_browser_for_focused() -> Option<Browser> {
    let id = focused_window();
    if id == NORMAL_WINDOW_ID {
        return None;
    }
    active_browser(id)
}

pub fn active_tab_host(window_id: WindowId) -> Option<String> {
    with_window(window_id, |window| {
        let active = window.active_tab?;
        window
            .tabs
            .iter()
            .find(|t| t.id == active)
            .and_then(|t| crate::vault::lookup::host_from_uri(&t.url))
    })
    .flatten()
}

pub fn active_browser(window_id: WindowId) -> Option<Browser> {
    with_window(window_id, |window| {
        let active = window.active_tab?;
        window
            .tabs
            .iter()
            .find(|t| t.id == active)
            .and_then(|t| t.browser.clone())
    })
    .flatten()
}

pub fn snapshot(window_id: WindowId) -> PrivateSnapshot {
    with_window(window_id, |window| PrivateSnapshot {
        window_id: window.id,
        active_tab: window.active_tab,
        tabs: window
            .tabs
            .iter()
            .map(|t| PrivateTabView {
                id: t.id,
                url: t.url.clone(),
                title: t.title.clone(),
                loading: t.loading,
                can_go_back: t.can_go_back,
                can_go_forward: t.can_go_forward,
            })
            .collect(),
    })
    .unwrap_or(PrivateSnapshot {
        window_id,
        active_tab: None,
        tabs: Vec::new(),
    })
}

/// Route a target="_blank" open to the private window that owns `cef_id`.
pub fn request_open_tab(cef_id: i32, url: &str, foreground: bool) {
    let Some(window_id) = window_id_for_cef(cef_id) else {
        return;
    };
    emit(
        window_id,
        Event::OpenTabRequested,
        i32::from(foreground),
        Some(url),
    );
}

pub fn note_closed(cef_id: i32) {
    let Some(window_id) = window_id_for_cef(cef_id) else {
        return;
    };
    with_window_mut(window_id, |window| {
        if let Some(idx) = window
            .tabs
            .iter()
            .position(|t| t.browser.as_ref().is_some_and(|b| b.identifier() == cef_id))
        {
            window.tabs.remove(idx);
            if window
                .active_tab
                .is_some_and(|id| !window.tabs.iter().any(|t| t.id == id))
            {
                window.active_tab = window.tabs.last().map(|t| t.id);
            }
        }
    });
    crate::adblock::forget_tab(cef_id);
    emit(window_id, Event::StructureChanged, 0, None);
}

fn note_content(
    window_id: WindowId,
    cef_id: i32,
    mutate: impl FnOnce(&mut PrivateTab) -> bool,
) -> Option<TabId> {
    with_window_mut(window_id, |window| {
        let tab = window
            .tabs
            .iter_mut()
            .find(|t| t.browser.as_ref().is_some_and(|b| b.identifier() == cef_id))?;
        mutate(tab).then_some(tab.id)
    })
    .flatten()
}

pub fn note_title(cef_id: i32, title: &str) -> bool {
    let Some(window_id) = window_id_for_cef(cef_id) else {
        return false;
    };
    let changed = note_content(window_id, cef_id, |tab| {
        let changed = tab.title != title;
        if changed {
            tab.title = title.to_string();
        }
        changed
    });
    if let Some(id) = changed {
        emit(window_id, Event::TabTitleChanged, id, Some(title));
    }
    true
}

pub fn note_url(cef_id: i32, url: &str) -> bool {
    let Some(window_id) = window_id_for_cef(cef_id) else {
        return false;
    };
    let changed = note_content(window_id, cef_id, |tab| {
        let changed = tab.url != url;
        if changed {
            tab.url = url.to_string();
        }
        changed
    });
    if let Some(id) = changed {
        emit(window_id, Event::TabUrlChanged, id, Some(url));
    }
    true
}

pub fn note_load_state(
    cef_id: i32,
    loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
) -> bool {
    let Some(window_id) = window_id_for_cef(cef_id) else {
        return false;
    };
    let result = with_window_mut(window_id, |window| {
        let tab = window
            .tabs
            .iter_mut()
            .find(|t| t.browser.as_ref().is_some_and(|b| b.identifier() == cef_id))?;
        let loading_changed = tab.loading != loading;
        let nav_changed = tab.can_go_back != can_go_back || tab.can_go_forward != can_go_forward;
        tab.loading = loading;
        tab.can_go_back = can_go_back;
        tab.can_go_forward = can_go_forward;
        Some((tab.id, loading_changed, nav_changed))
    })
    .flatten();
    let Some((id, loading_changed, nav_changed)) = result else {
        return true;
    };
    if loading_changed {
        emit(
            window_id,
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
        emit(window_id, Event::TabNavStateChanged, id, Some(&code));
    }
    true
}

/// Ingest a sampled page theme/background color. Returns true when the browser
/// belonged to a private window (handled here); false for the normal window.
pub fn note_theme_color(cef_id: i32, color: &str) -> bool {
    let Some(window_id) = window_id_for_cef(cef_id) else {
        return false;
    };
    let id = note_content(window_id, cef_id, |_| true);
    if let Some(id) = id {
        emit(window_id, Event::TabThemeColorChanged, id, Some(color));
    }
    true
}

pub fn note_favicon(cef_id: i32, host: &str) -> bool {
    let Some(window_id) = window_id_for_cef(cef_id) else {
        return false;
    };
    let id = with_window(window_id, |window| {
        window.tab_id_by_cef(cef_id).or_else(|| {
            window.tabs.iter().find_map(|t| {
                t.browser
                    .as_ref()
                    .filter(|b| b.identifier() == cef_id)
                    .map(|_| t.id)
            })
        })
    })
    .flatten();
    if let Some(id) = id {
        emit(window_id, Event::TabFaviconChanged, id, Some(host));
    }
    true
}

impl PrivateWindow {
    fn tab_id_by_cef(&self, cef_id: i32) -> Option<TabId> {
        self.tabs
            .iter()
            .find(|t| t.browser.as_ref().is_some_and(|b| b.identifier() == cef_id))
            .map(|t| t.id)
    }
}

pub fn is_site_allowlisted(window_id: WindowId, host: &str) -> bool {
    with_window(window_id, |w| {
        w.allowlist.iter().any(|h| h == &allowlist_key(host))
    })
    .unwrap_or(false)
}

pub fn set_site_allowlisted(window_id: WindowId, host: &str, allowlisted: bool) {
    let key = allowlist_key(host);
    with_window_mut(window_id, |window| {
        match (allowlisted, window.allowlist.iter().position(|h| *h == key)) {
            (true, None) => window.allowlist.push(key),
            (false, Some(i)) => {
                window.allowlist.remove(i);
            }
            _ => {}
        }
    });
    crate::main_thread::post(crate::engine::content_rules::refresh);
}

/// A private window's ephemeral allowlist (registrable domains).
pub fn ephemeral_allowlist(window_id: WindowId) -> Vec<String> {
    with_window(window_id, |w| w.allowlist.clone()).unwrap_or_default()
}

/// Live browsers in a private window - for close-on-window-exit.
pub fn browsers_in_window(window_id: WindowId) -> Vec<Browser> {
    with_window(window_id, |window| {
        window
            .tabs
            .iter()
            .filter_map(|t| t.browser.clone())
            .collect()
    })
    .unwrap_or_default()
}
