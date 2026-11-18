//! Property tests for the tab store (P1-43, ADR-0065).
//!
//! Random sequences of the real public mutators run against the module-global
//! store (all tabs suspended, so no CEF objects are involved), and the store's
//! invariants are checked after every step. The invariants are the rules the
//! tab model otherwise keeps only in comments: what a favorite, a pinned tab,
//! and an open tab may hold, and that every reference points at something real.

use std::collections::HashSet;

use proptest::prelude::*;

use super::*;

/// Every rule the store must hold between operations. Returns the first broken
/// rule, worded so a failing case says what went wrong.
fn check_invariants(store: &TabStore) -> Result<(), String> {
    let mut ids = HashSet::new();
    for t in &store.tabs {
        if !ids.insert(t.id) {
            return Err(format!("duplicate tab id {}", t.id));
        }
        if t.id >= store.next_tab {
            return Err(format!(
                "tab id {} not below next_tab {}",
                t.id, store.next_tab
            ));
        }
        if !store.spaces.iter().any(|s| s.id == t.space) {
            return Err(format!("tab {} in missing space {}", t.id, t.space));
        }
        // Kind exclusivity (favorite vs pinned, folder implies pinned, saved URL
        // iff saved) is now enforced by `Placement`; only cross-references remain.
        if let Some(fid) = t.folder() {
            match store.folders.iter().find(|f| f.id == fid) {
                None => return Err(format!("tab {} in missing folder {fid}", t.id)),
                Some(f) if f.space != t.space => {
                    return Err(format!(
                        "tab {} (space {}) in folder {fid} of space {}",
                        t.id, t.space, f.space
                    ))
                }
                Some(_) => {}
            }
        }
    }
    if store.favorite_count() > MAX_VISIBLE_FAVORITES {
        return Err(format!(
            "{} favorites, cap {MAX_VISIBLE_FAVORITES}",
            store.favorite_count()
        ));
    }

    let mut folder_ids = HashSet::new();
    for f in &store.folders {
        if !folder_ids.insert(f.id) {
            return Err(format!("duplicate folder id {}", f.id));
        }
        if f.id >= store.next_folder {
            return Err(format!("folder id {} not below next_folder", f.id));
        }
        if !store.spaces.iter().any(|s| s.id == f.space) {
            return Err(format!("folder {} in missing space {}", f.id, f.space));
        }
        // Parent chain: exists, same space, no cycle.
        let mut seen = HashSet::from([f.id]);
        let mut cur = f.parent;
        while let Some(pid) = cur {
            let Some(p) = store.folders.iter().find(|x| x.id == pid) else {
                return Err(format!("folder {} has missing parent {pid}", f.id));
            };
            if p.space != f.space {
                return Err(format!("folder {} parent {pid} in another space", f.id));
            }
            if !seen.insert(pid) {
                return Err(format!("folder cycle through {pid}"));
            }
            cur = p.parent;
        }
    }

    let mut space_ids = HashSet::new();
    for s in &store.spaces {
        if !space_ids.insert(s.id) {
            return Err(format!("duplicate space id {}", s.id));
        }
        if s.id >= store.next_space {
            return Err(format!("space id {} not below next_space", s.id));
        }
        if let Some(active) = s.active {
            if !store.can_be_active_in(s.id, active) {
                return Err(format!(
                    "space {} active tab {active} cannot be active there",
                    s.id
                ));
            }
        }
    }
    if store.spaces.is_empty() {
        return Err("no spaces".into());
    }
    if !store.spaces.iter().any(|s| s.id == store.active_space) {
        return Err(format!("active space {} missing", store.active_space));
    }
    if store.closed.len() > CLOSED_STACK_CAP {
        return Err("closed stack over cap".into());
    }

    // Session round-trip: what we write is what we read back.
    let saved = store.saved_state();
    let restored = TabStore::from_saved(saved_clone(&saved)).saved_state();
    let (a, b) = (json(&saved), json(&restored));
    if a != b {
        return Err(format!(
            "session round-trip changed state\nsaved:    {a}\nrestored: {b}"
        ));
    }
    Ok(())
}

fn json(state: &SavedState) -> String {
    serde_json::to_string(state).unwrap_or_default()
}

fn saved_clone(state: &SavedState) -> SavedState {
    serde_json::from_str(&json(state)).expect("SavedState round-trips through JSON")
}

#[derive(Clone, Debug)]
enum Op {
    Select(usize),
    SelectIndex(u8),
    AdjacentTab(bool),
    AdjacentSpace(bool),
    MoveVisible(bool),
    HideSaved(usize),
    Delete(usize),
    Close(usize),
    Reopen,
    SetPinned(usize, bool),
    SetFavorite(usize, bool),
    ReplacePinnedUrl(usize),
    MoveToFolder(usize, Option<usize>),
    MoveTab(usize, bool, bool, Option<usize>, Option<usize>),
    CreateFolder,
    CreateFolderWithTabs(Vec<usize>),
    ClearEphemeral(Vec<usize>),
    DeleteFolder(usize),
    CreateSpace,
    SelectSpace(usize),
    DeleteSpace(usize),
    FolderToSpace(usize),
    SpaceToFolder(usize),
    Navigate(usize, u8),
}

fn op() -> impl Strategy<Value = Op> {
    let i = || 0usize..16;
    prop_oneof![
        i().prop_map(Op::Select),
        (0u8..10).prop_map(Op::SelectIndex),
        any::<bool>().prop_map(Op::AdjacentTab),
        any::<bool>().prop_map(Op::AdjacentSpace),
        any::<bool>().prop_map(Op::MoveVisible),
        i().prop_map(Op::HideSaved),
        i().prop_map(Op::Delete),
        i().prop_map(Op::Close),
        Just(Op::Reopen),
        (i(), any::<bool>()).prop_map(|(a, b)| Op::SetPinned(a, b)),
        (i(), any::<bool>()).prop_map(|(a, b)| Op::SetFavorite(a, b)),
        i().prop_map(Op::ReplacePinnedUrl),
        (i(), proptest::option::of(i())).prop_map(|(a, b)| Op::MoveToFolder(a, b)),
        (
            i(),
            any::<bool>(),
            any::<bool>(),
            proptest::option::of(i()),
            proptest::option::of(i())
        )
            .prop_map(|(a, f, p, d, b)| Op::MoveTab(a, f, p, d, b)),
        Just(Op::CreateFolder),
        proptest::collection::vec(i(), 0..4).prop_map(Op::CreateFolderWithTabs),
        proptest::collection::vec(i(), 0..3).prop_map(Op::ClearEphemeral),
        i().prop_map(Op::DeleteFolder),
        Just(Op::CreateSpace),
        i().prop_map(Op::SelectSpace),
        i().prop_map(Op::DeleteSpace),
        i().prop_map(Op::FolderToSpace),
        i().prop_map(Op::SpaceToFolder),
        (i(), 0u8..4).prop_map(|(a, n)| Op::Navigate(a, n)),
    ]
}

/// A starting session: 1-3 spaces, a few folders, and tabs of every kind.
fn initial() -> impl Strategy<Value = SavedState> {
    (
        1i32..=3,
        proptest::collection::vec(0usize..3, 0..4),
        proptest::collection::vec((0usize..3, 0u8..4, any::<bool>()), 0..12),
    )
        .prop_map(|(n_spaces, folder_spaces, tabs)| {
            let space_id = |k: usize| 1 + (k as i32 % n_spaces);
            let spaces = (1..=n_spaces)
                .map(|id| SavedSpace {
                    id,
                    name: format!("s{id}"),
                    icon: "✨".into(),
                    pinned_expanded: true,
                    active: None,
                })
                .collect();
            let folders: Vec<SavedFolder> = folder_spaces
                .iter()
                .enumerate()
                .map(|(k, &sp)| SavedFolder {
                    id: 100 + k as i32,
                    space: space_id(sp),
                    name: format!("f{k}"),
                    parent: None,
                    expanded: k % 2 == 0,
                })
                .collect();
            let mut favorites = 0;
            let tabs = tabs
                .iter()
                .enumerate()
                .map(|(k, &(sp, kind, use_folder))| {
                    let space = space_id(sp);
                    let folder = folders
                        .iter()
                        .find(|f| f.space == space)
                        .map(|f| f.id)
                        .filter(|_| use_folder && kind == 2);
                    let favorite = kind == 3 && favorites < MAX_VISIBLE_FAVORITES;
                    favorites += usize::from(favorite);
                    SavedTab {
                        id: 1000 + k as i32,
                        space,
                        folder,
                        pinned: !favorite && (kind == 1 || kind == 2),
                        favorite,
                        url: format!("https://t{k}.example/"),
                        title: format!("t{k}"),
                    }
                })
                .collect();
            SavedState {
                active_space: 1,
                spaces,
                folders,
                tabs,
            }
        })
}

fn pick<T: Copy>(items: &[T], i: usize) -> Option<T> {
    (!items.is_empty()).then(|| items[i % items.len()])
}

fn tab_ids() -> Vec<TabId> {
    with_store(|s| s.tabs.iter().map(|t| t.id).collect()).unwrap_or_default()
}
fn folder_ids() -> Vec<FolderId> {
    with_store(|s| s.folders.iter().map(|f| f.id).collect()).unwrap_or_default()
}
fn space_ids() -> Vec<SpaceId> {
    with_store(|s| s.spaces.iter().map(|x| x.id).collect()).unwrap_or_default()
}

fn apply(op: &Op) {
    let tab = |i| pick(&tab_ids(), i);
    let folder = |i| pick(&folder_ids(), i);
    let space = |i| pick(&space_ids(), i);
    let step = |up: bool| if up { 1 } else { -1 };
    match op {
        Op::Select(i) => tab(*i).into_iter().for_each(select_tab),
        Op::SelectIndex(n) => {
            select_tab_by_index(*n);
        }
        Op::AdjacentTab(up) => {
            select_adjacent_tab(step(*up));
        }
        Op::AdjacentSpace(up) => {
            select_adjacent_space(step(*up));
        }
        Op::MoveVisible(up) => {
            move_visible_tab(step(*up));
        }
        Op::HideSaved(i) => tab(*i).into_iter().for_each(hide_saved_tab),
        Op::Delete(i) => tab(*i).into_iter().for_each(delete_tab),
        Op::Close(i) => tab(*i).into_iter().for_each(close_tab),
        Op::Reopen => {
            reopen_closed_tab();
        }
        Op::SetPinned(i, on) => tab(*i).into_iter().for_each(|t| set_pinned(t, *on)),
        Op::SetFavorite(i, on) => tab(*i).into_iter().for_each(|t| set_favorite(t, *on)),
        Op::ReplacePinnedUrl(i) => {
            if let Some(t) = tab(*i) {
                replace_pinned_url(t);
            }
        }
        Op::MoveToFolder(i, f) => {
            if let Some(t) = tab(*i) {
                move_to_folder(t, f.and_then(folder));
            }
        }
        Op::MoveTab(i, fav, pinned, f, before) => {
            if let Some(t) = tab(*i) {
                move_tab(t, *fav, *pinned, f.and_then(folder), before.and_then(tab));
            }
        }
        Op::CreateFolder => {
            create_folder("new");
        }
        Op::CreateFolderWithTabs(idx) => {
            let ids: Vec<TabId> = idx.iter().filter_map(|&i| tab(i)).collect();
            create_folder_with_tabs("group", &ids);
        }
        Op::ClearEphemeral(idx) => {
            let keep: Vec<TabId> = idx.iter().filter_map(|&i| tab(i)).collect();
            clear_ephemeral_tabs(&keep);
        }
        Op::DeleteFolder(i) => folder(*i).into_iter().for_each(delete_folder),
        Op::CreateSpace => {
            create_space("new", "🧪");
        }
        Op::SelectSpace(i) => space(*i).into_iter().for_each(select_space),
        Op::DeleteSpace(i) => space(*i).into_iter().for_each(delete_space),
        Op::FolderToSpace(i) => {
            if let Some(f) = folder(*i) {
                folder_to_space(f);
            }
        }
        Op::SpaceToFolder(i) => {
            if let Some(s) = space(*i) {
                space_to_folder(s);
            }
        }
        // What `note_url` does once CEF reports a navigation.
        Op::Navigate(i, n) => {
            if let Some(t) = tab(*i) {
                with_store_mut(|s| {
                    if let Some(tab) = s.tabs.iter_mut().find(|x| x.id == t) {
                        tab.url = format!("https://nav{n}.example/");
                    }
                });
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn store_invariants_hold_across_random_operations(
        start in initial(),
        ops in proptest::collection::vec(op(), 1..40),
    ) {
        with_global_store_test_lock(|| -> Result<(), TestCaseError> {
            let dir = std::env::temp_dir().join(format!("ochre-tabs-prop-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            test_reset_store(&dir.join("session.json").to_string_lossy());
            *STORE.lock().expect("store lock") = Some(TabStore::from_saved(start));
            let first = with_store(check_invariants).expect("store set");
            prop_assert!(first.is_ok(), "initial store: {}", first.unwrap_err());
            for (n, op) in ops.iter().enumerate() {
                apply(op);
                let result = with_store(check_invariants).expect("store set");
                prop_assert!(result.is_ok(), "after step {n} {op:?}: {}", result.unwrap_err());
            }
            Ok(())
        })?;
    }
}

/// A URL typed while the visible tab is suspended must not be dropped: the
/// tab records it, and hydration (which reads `suspended_url`) loads it.
#[test]
fn navigating_a_suspended_visible_tab_keeps_the_url() {
    with_global_store_test_lock(|| {
        let dir = std::env::temp_dir().join(format!("ochre-tabs-nav-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        test_reset_store(&dir.join("session.json").to_string_lossy());
        let state = SavedState {
            active_space: 1,
            spaces: vec![SavedSpace {
                id: 1,
                name: "s".into(),
                icon: "✨".into(),
                pinned_expanded: true,
                active: Some(7),
            }],
            folders: vec![],
            tabs: vec![SavedTab {
                id: 7,
                space: 1,
                folder: None,
                pinned: true,
                favorite: false,
                url: "https://home.example/".into(),
                title: "home".into(),
            }],
        };
        *STORE.lock().expect("store lock") = Some(TabStore::from_saved(state));

        assert!(navigate_suspended_visible("https://typed.example/"));
        assert_eq!(suspended_url(7).as_deref(), Some("https://typed.example/"));
        let tab_ok = with_store(|s| {
            let t = s.tab(7).expect("tab");
            !t.nav_pending && t.saved_url() == Some("https://home.example/")
        });
        assert_eq!(tab_ok, Some(true), "pending served; pinned URL unchanged");
    });
}
