//! The transient overlay state machine (P1-46, ADR-0068): which input surface
//! is up (the command bar, with where its commit lands, or the in-place address
//! editor) and what a commit does.
//!
//! It replaces a host-side global "route" that every surface wrote on open and
//! close. Close notifications arrive late (a panel resigns key after the next
//! surface already opened), so a global reset sent the address editor's commit
//! to a new tab. Here the destination is part of the state, a close names the
//! surface it closes and is ignored when that surface is no longer up, and a
//! commit with no surface up is dropped.
//!
//! [`step`] is pure; the tests walk every reachable state. The host sends
//! inputs through [`send`] and renders the returned state.

use std::sync::Mutex;

use serde::Serialize;

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

/// What is up. At most one surface at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Overlay {
    None,
    CommandBar { dest: Dest },
    AddressEdit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input<'a> {
    /// Show the command bar committing to `dest`; replaces whatever is up.
    OpenCommandBar(Dest),
    /// Edit the visible tab's address in place; replaces whatever is up.
    BeginAddressEdit,
    /// The named surface went away (Escape, focus loss, a panel resigning
    /// key). Ignored unless that surface is the one up.
    Close(Kind),
    /// Return or a chosen suggestion: go to `url` from the surface that is up.
    Commit(&'a str),
}

/// What the host must do after a step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Effect {
    Navigate { dest: Dest, url: String },
}

/// One transition. Pure.
pub fn step(state: Overlay, input: Input<'_>) -> (Overlay, Vec<Effect>) {
    match input {
        Input::OpenCommandBar(dest) => (Overlay::CommandBar { dest }, Vec::new()),
        Input::BeginAddressEdit => (Overlay::AddressEdit, Vec::new()),
        Input::Close(kind) => {
            let up = match state {
                Overlay::CommandBar { .. } => Some(Kind::CommandBar),
                Overlay::AddressEdit => Some(Kind::AddressEdit),
                Overlay::None => None,
            };
            if up == Some(kind) {
                (Overlay::None, Vec::new())
            } else {
                (state, Vec::new())
            }
        }
        Input::Commit(url) => {
            let dest = match state {
                Overlay::CommandBar { dest } => dest,
                Overlay::AddressEdit => Dest::CurrentTab,
                Overlay::None => return (state, Vec::new()),
            };
            let url = url.trim();
            if url.is_empty() {
                return (state, Vec::new());
            }
            let effect = Effect::Navigate {
                dest,
                url: url.to_string(),
            };
            (Overlay::None, vec![effect])
        }
    }
}

static STATE: Mutex<Overlay> = Mutex::new(Overlay::None);

/// Apply `input` to the app's overlay state. Returns the new state and the
/// effects the host must run. Main thread only.
pub fn send(input: Input<'_>) -> (Overlay, Vec<Effect>) {
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let (next, effects) = step(*state, input);
    if next != *state {
        eprintln!("[overlay] {:?} -> {:?}", *state, next);
    }
    *state = next;
    (next, effects)
}

/// The current overlay state.
pub fn current() -> Overlay {
    *STATE.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use std::collections::{HashSet, VecDeque};

    use super::*;

    const DESTS: [Dest; 3] = [Dest::NewTab, Dest::CurrentTab, Dest::LittleWindow];

    fn inputs() -> Vec<Input<'static>> {
        let mut all: Vec<Input<'static>> =
            DESTS.iter().map(|d| Input::OpenCommandBar(*d)).collect();
        all.extend([
            Input::BeginAddressEdit,
            Input::Close(Kind::CommandBar),
            Input::Close(Kind::AddressEdit),
            Input::Commit("https://example.com/"),
            Input::Commit("   "),
        ]);
        all
    }

    /// Every state reachable from `None` by any input sequence.
    fn reachable() -> Vec<Overlay> {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([Overlay::None]);
        let mut out = Vec::new();
        while let Some(state) = queue.pop_front() {
            if !seen.insert(format!("{state:?}")) {
                continue;
            }
            out.push(state);
            for input in inputs() {
                queue.push_back(step(state, input).0);
            }
        }
        out
    }

    #[test]
    fn every_surface_and_destination_is_reachable() {
        let states = reachable();
        assert_eq!(states.len(), 5, "{states:?}");
    }

    #[test]
    fn opening_a_surface_always_shows_exactly_that_surface() {
        for state in reachable() {
            for dest in DESTS {
                assert_eq!(
                    step(state, Input::OpenCommandBar(dest)).0,
                    Overlay::CommandBar { dest }
                );
            }
            assert_eq!(step(state, Input::BeginAddressEdit).0, Overlay::AddressEdit);
        }
    }

    #[test]
    fn a_close_only_closes_the_surface_it_names() {
        for state in reachable() {
            let (after_bar, _) = step(state, Input::Close(Kind::CommandBar));
            let (after_edit, _) = step(state, Input::Close(Kind::AddressEdit));
            match state {
                Overlay::CommandBar { .. } => {
                    assert_eq!(after_bar, Overlay::None);
                    assert_eq!(after_edit, state);
                }
                Overlay::AddressEdit => {
                    assert_eq!(after_edit, Overlay::None);
                    assert_eq!(after_bar, state);
                }
                Overlay::None => {
                    assert_eq!(after_bar, state);
                    assert_eq!(after_edit, state);
                }
            }
        }
    }

    #[test]
    fn a_commit_lands_where_the_visible_surface_points() {
        for state in reachable() {
            let (next, effects) = step(state, Input::Commit("https://example.com/"));
            let expected = match state {
                Overlay::CommandBar { dest } => Some(dest),
                Overlay::AddressEdit => Some(Dest::CurrentTab),
                Overlay::None => None,
            };
            match expected {
                Some(dest) => {
                    assert_eq!(next, Overlay::None);
                    assert_eq!(
                        effects,
                        vec![Effect::Navigate {
                            dest,
                            url: "https://example.com/".into()
                        }]
                    );
                }
                None => assert!(effects.is_empty(), "stale commit from {state:?} ran"),
            }
        }
    }

    #[test]
    fn blank_commits_do_nothing() {
        for state in reachable() {
            assert_eq!(step(state, Input::Commit("  ")), (state, Vec::new()));
        }
    }

    /// The reported bug: open the command bar, click the address chip, then
    /// the bar's panel resigns key and reports its close. The address edit
    /// must survive, and Return must navigate the current tab.
    #[test]
    fn a_late_command_bar_close_does_not_redirect_the_address_edit() {
        let mut state = Overlay::None;
        for input in [
            Input::OpenCommandBar(Dest::NewTab),
            Input::BeginAddressEdit,
            Input::Close(Kind::CommandBar),
        ] {
            state = step(state, input).0;
        }
        assert_eq!(state, Overlay::AddressEdit);
        let (_, effects) = step(state, Input::Commit("example.com"));
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
        let json = serde_json::to_string(&Overlay::CommandBar {
            dest: Dest::LittleWindow,
        })
        .unwrap();
        assert_eq!(json, r#"{"kind":"command_bar","dest":"little_window"}"#);
        assert_eq!(
            serde_json::to_string(&Overlay::None).unwrap(),
            r#"{"kind":"none"}"#
        );
    }
}
