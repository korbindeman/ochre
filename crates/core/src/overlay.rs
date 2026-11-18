//! The UI surface state machine (P1-46, ADR-0068): which input surface is up
//! (the command bar, with where its commit lands, or the in-place address
//! editor), which window shows the find bar, and which page (History,
//! Passwords, Settings) covers each window's web view.
//!
//! It replaces host-side flags every surface wrote on open and close. Close
//! notifications arrive late (a panel resigns key after the next surface
//! already opened), so a global reset once sent the address editor's commit to
//! a new tab. Here the destination is part of the state, a close names the
//! surface it closes and is ignored when that surface is no longer up, and a
//! commit with no surface up is dropped. The cross-surface rules live here
//! too: a page closes the input surfaces and its window's find bar; find and
//! address editing wait for the web view.
//!
//! [`step`] is pure; the tests walk every reachable state. The host sends
//! inputs through [`send`] and renders the returned [`Ui`].

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;

/// A host window: `0` normal, `>= 1` private, `< 0` a little window.
pub type WindowId = i32;

/// Where a command-bar commit lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Dest {
    NewTab,
    CurrentTab,
    LittleWindow,
}

/// An input surface, as named by a close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    CommandBar,
    AddressEdit,
}

/// The input surface that is up. At most one at a time, app-wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Overlay {
    None,
    CommandBar { dest: Dest },
    AddressEdit,
}

/// An app page covering a window's web view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    History,
    Passwords,
    Settings,
}

/// The whole surface state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Ui {
    pub overlay: Overlay,
    /// The window whose find bar is up, if any (one find bar app-wide).
    pub find: Option<WindowId>,
    /// Windows showing a page; absent means the web view.
    pub pages: BTreeMap<WindowId, Page>,
}

impl Ui {
    pub const fn new() -> Self {
        Self {
            overlay: Overlay::None,
            find: None,
            pages: BTreeMap::new(),
        }
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input<'a> {
    /// Show the command bar committing to `dest`; replaces whatever is up.
    OpenCommandBar(Dest),
    /// Edit the visible tab's address in window `w`; replaces whatever is up.
    /// Ignored while a page covers that window's web view.
    BeginAddressEdit(WindowId),
    /// The named surface went away (Escape, focus loss, a panel resigning
    /// key). Ignored unless that surface is the one up.
    Close(Kind),
    /// Return or a chosen suggestion: go to `url` from the surface that is up.
    Commit(&'a str),
    /// Show `page` in window `w`, replacing another page. Closes the input
    /// surfaces and that window's find bar.
    OpenPage(WindowId, Page),
    /// Close `page` in window `w`. Ignored unless it is the page shown.
    ClosePage(WindowId, Page),
    /// Back to the web view in `w` (a tab was selected or opened).
    ShowWeb(WindowId),
    /// Show the find bar in `w`. Ignored while a page covers the web view.
    OpenFind(WindowId),
    CloseFind,
    /// Window `w` closed: forget its page and find bar.
    WindowClosed(WindowId),
}

/// What the host must do after a step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    Navigate { dest: Dest, url: String },
}

/// One transition. Pure.
pub fn step(ui: &Ui, input: Input<'_>) -> (Ui, Vec<Effect>) {
    let mut next = ui.clone();
    let mut effects = Vec::new();
    match input {
        Input::OpenCommandBar(dest) => next.overlay = Overlay::CommandBar { dest },
        Input::BeginAddressEdit(w) => {
            if !next.pages.contains_key(&w) {
                next.overlay = Overlay::AddressEdit;
            }
        }
        Input::Close(kind) => {
            let up = match next.overlay {
                Overlay::CommandBar { .. } => Some(Kind::CommandBar),
                Overlay::AddressEdit => Some(Kind::AddressEdit),
                Overlay::None => None,
            };
            if up == Some(kind) {
                next.overlay = Overlay::None;
            }
        }
        Input::Commit(url) => {
            let dest = match next.overlay {
                Overlay::CommandBar { dest } => Some(dest),
                Overlay::AddressEdit => Some(Dest::CurrentTab),
                Overlay::None => None,
            };
            let url = url.trim();
            if let Some(dest) = dest.filter(|_| !url.is_empty()) {
                next.overlay = Overlay::None;
                effects.push(Effect::Navigate {
                    dest,
                    url: url.to_string(),
                });
            }
        }
        Input::OpenPage(w, page) => {
            next.pages.insert(w, page);
            next.overlay = Overlay::None;
            if next.find == Some(w) {
                next.find = None;
            }
        }
        Input::ClosePage(w, page) => {
            if next.pages.get(&w) == Some(&page) {
                next.pages.remove(&w);
            }
        }
        Input::ShowWeb(w) => {
            next.pages.remove(&w);
        }
        Input::OpenFind(w) => {
            if !next.pages.contains_key(&w) {
                next.find = Some(w);
            }
        }
        Input::CloseFind => next.find = None,
        Input::WindowClosed(w) => {
            next.pages.remove(&w);
            if next.find == Some(w) {
                next.find = None;
            }
        }
    }
    (next, effects)
}

static STATE: Mutex<Ui> = Mutex::new(Ui::new());

/// Apply `input` to the app's surface state. Returns the new state and the
/// effects the host must run. Main thread only.
pub fn send(input: Input<'_>) -> (Ui, Vec<Effect>) {
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let (next, effects) = step(&state, input);
    if next != *state {
        eprintln!("[surface] {input:?}: {:?} -> {:?}", *state, next);
    }
    *state = next.clone();
    (next, effects)
}

/// The current surface state.
pub fn current() -> Ui {
    STATE.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use std::collections::{HashSet, VecDeque};

    use super::*;

    const DESTS: [Dest; 3] = [Dest::NewTab, Dest::CurrentTab, Dest::LittleWindow];
    const PAGES: [Page; 3] = [Page::History, Page::Passwords, Page::Settings];
    const WINDOWS: [WindowId; 2] = [0, 1];

    fn inputs() -> Vec<Input<'static>> {
        let mut all: Vec<Input<'static>> =
            DESTS.iter().map(|d| Input::OpenCommandBar(*d)).collect();
        all.extend([
            Input::Close(Kind::CommandBar),
            Input::Close(Kind::AddressEdit),
            Input::Commit("https://example.com/"),
            Input::Commit("   "),
            Input::CloseFind,
        ]);
        for w in WINDOWS {
            all.extend([
                Input::BeginAddressEdit(w),
                Input::ShowWeb(w),
                Input::OpenFind(w),
                Input::WindowClosed(w),
            ]);
            for p in PAGES {
                all.extend([Input::OpenPage(w, p), Input::ClosePage(w, p)]);
            }
        }
        all
    }

    /// Every state reachable from the initial one by any input sequence.
    fn reachable() -> Vec<Ui> {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([Ui::new()]);
        let mut out = Vec::new();
        while let Some(ui) = queue.pop_front() {
            if !seen.insert(format!("{ui:?}")) {
                continue;
            }
            for input in inputs() {
                queue.push_back(step(&ui, input).0);
            }
            out.push(ui);
        }
        out
    }

    #[test]
    fn the_state_space_is_what_the_model_says() {
        // overlay (5) x find (none, w0, w1) x page per window (4 x 4), minus
        // find in a window showing a page (find needs the web view), minus an
        // address edit while every window shows a page (3 x 3 page pairs).
        let states = reachable();
        for ui in &states {
            assert!(ui.find.is_none_or(|w| !ui.pages.contains_key(&w)), "{ui:?}");
            if ui.overlay == Overlay::AddressEdit {
                assert!(WINDOWS.iter().any(|w| !ui.pages.contains_key(w)), "{ui:?}");
            }
        }
        assert_eq!(states.len(), 5 * (16 + 2 * 4) - 9);
    }

    #[test]
    fn opening_a_surface_always_shows_exactly_that_surface() {
        for ui in reachable() {
            for dest in DESTS {
                assert_eq!(
                    step(&ui, Input::OpenCommandBar(dest)).0.overlay,
                    Overlay::CommandBar { dest }
                );
            }
            for w in WINDOWS {
                let edit = step(&ui, Input::BeginAddressEdit(w)).0.overlay;
                if ui.pages.contains_key(&w) {
                    assert_eq!(edit, ui.overlay, "address edit under a page");
                } else {
                    assert_eq!(edit, Overlay::AddressEdit);
                }
                for page in PAGES {
                    let next = step(&ui, Input::OpenPage(w, page)).0;
                    assert_eq!(next.pages.get(&w), Some(&page));
                    assert_eq!(next.overlay, Overlay::None);
                    assert_ne!(next.find, Some(w));
                }
            }
        }
    }

    #[test]
    fn a_close_only_closes_the_surface_it_names() {
        for ui in reachable() {
            let after_bar = step(&ui, Input::Close(Kind::CommandBar)).0.overlay;
            let after_edit = step(&ui, Input::Close(Kind::AddressEdit)).0.overlay;
            match ui.overlay {
                Overlay::CommandBar { .. } => {
                    assert_eq!(after_bar, Overlay::None);
                    assert_eq!(after_edit, ui.overlay);
                }
                Overlay::AddressEdit => {
                    assert_eq!(after_edit, Overlay::None);
                    assert_eq!(after_bar, ui.overlay);
                }
                Overlay::None => {
                    assert_eq!(after_bar, ui.overlay);
                    assert_eq!(after_edit, ui.overlay);
                }
            }
            for w in WINDOWS {
                for page in PAGES {
                    let next = step(&ui, Input::ClosePage(w, page)).0;
                    if ui.pages.get(&w) == Some(&page) {
                        assert!(!next.pages.contains_key(&w));
                    } else {
                        assert_eq!(next, ui, "closing a page that is not shown changed state");
                    }
                }
            }
        }
    }

    #[test]
    fn a_commit_lands_where_the_visible_surface_points() {
        for ui in reachable() {
            let (next, effects) = step(&ui, Input::Commit("https://example.com/"));
            let expected = match ui.overlay {
                Overlay::CommandBar { dest } => Some(dest),
                Overlay::AddressEdit => Some(Dest::CurrentTab),
                Overlay::None => None,
            };
            match expected {
                Some(dest) => {
                    assert_eq!(next.overlay, Overlay::None);
                    assert_eq!(
                        effects,
                        vec![Effect::Navigate {
                            dest,
                            url: "https://example.com/".into()
                        }]
                    );
                }
                None => assert!(effects.is_empty(), "stale commit from {ui:?} ran"),
            }
        }
    }

    #[test]
    fn blank_commits_do_nothing() {
        for ui in reachable() {
            assert_eq!(step(&ui, Input::Commit("  ")), (ui.clone(), Vec::new()));
        }
    }

    #[test]
    fn pages_are_per_window_and_closing_a_window_forgets_it() {
        for ui in reachable() {
            for w in WINDOWS {
                let other = 1 - w;
                for page in PAGES {
                    let next = step(&ui, Input::OpenPage(w, page)).0;
                    assert_eq!(next.pages.get(&other), ui.pages.get(&other));
                }
                let closed = step(&ui, Input::WindowClosed(w)).0;
                assert!(!closed.pages.contains_key(&w));
                assert_ne!(closed.find, Some(w));
            }
        }
    }

    #[test]
    fn every_state_returns_to_the_web_view() {
        for ui in reachable() {
            let mut next = ui;
            for input in [
                Input::Close(Kind::CommandBar),
                Input::Close(Kind::AddressEdit),
                Input::CloseFind,
                Input::ShowWeb(0),
                Input::ShowWeb(1),
            ] {
                next = step(&next, input).0;
            }
            assert_eq!(next, Ui::new());
        }
    }

    /// The reported bug: open the command bar, click the address chip, then
    /// the bar's panel resigns key and reports its close. The address edit
    /// must survive, and Return must navigate the current tab.
    #[test]
    fn a_late_command_bar_close_does_not_redirect_the_address_edit() {
        let mut ui = Ui::new();
        for input in [
            Input::OpenCommandBar(Dest::NewTab),
            Input::BeginAddressEdit(0),
            Input::Close(Kind::CommandBar),
        ] {
            ui = step(&ui, input).0;
        }
        assert_eq!(ui.overlay, Overlay::AddressEdit);
        let (_, effects) = step(&ui, Input::Commit("example.com"));
        assert_eq!(
            effects,
            vec![Effect::Navigate {
                dest: Dest::CurrentTab,
                url: "example.com".into()
            }]
        );
    }

    #[test]
    fn states_serialize_for_the_host() {
        let mut ui = Ui::new();
        ui.overlay = Overlay::CommandBar {
            dest: Dest::LittleWindow,
        };
        ui.pages.insert(0, Page::History);
        ui.find = Some(-3);
        assert_eq!(
            serde_json::to_string(&ui).unwrap(),
            r#"{"overlay":{"kind":"command_bar","dest":"little_window"},"find":-3,"pages":{"0":"history"}}"#
        );
    }
}
