//! Runtime tab suspension policy - when to discard a background tab's web view.
//!
//! ADR-0013's lazy hydration handles *restore-at-launch*; this module handles
//! *discard-while-running*: free renderer memory for tabs the user is not on,
//! reusing the same `browser: None` + `restore_tab` re-entry path.
//!
//! v2 policy (budget + recency): keep a warm pool of background tabs and only
//! discard the least-recently-shown ones when the pool is full. A two-tab session
//! stays fully warm; large sessions shed LRU tabs instead of suspending everything
//! on a fixed timer.

use std::sync::{LazyLock, Mutex};

use crate::tabs::{self, TabId};

/// Hydrated background tabs we try to keep warm before discarding LRU ones.
const MAX_WARM_BACKGROUND: usize = 8;

/// Minimum idle time after a tab switch before budget reconciliation runs.
const RECONCILE_GRACE_MS: i64 = 180_000;

static PENDING_RECONCILE: LazyLock<Mutex<u64>> = LazyLock::new(|| Mutex::new(0));

/// Called from [`tabs::structural`] after a visibility / space change.
pub fn on_visibility_changed(
    old_visible: Option<TabId>,
    new_visible: Option<TabId>,
    old_space: tabs::SpaceId,
    new_space: tabs::SpaceId,
) {
    if let Some(new_id) = new_visible {
        tabs::note_tab_shown(new_id);
    }
    if new_space != old_space {
        reconcile_warm_budget();
    } else if old_visible != new_visible {
        schedule_reconcile();
    }
}

/// Returns ids of hydrated background tabs that exceed the warm budget.
/// Candidates are ordered least-recently-shown first (`last_shown_at` ascending).
fn excess_warm_tab_ids(candidates: &[(TabId, i64)], budget: usize) -> Vec<TabId> {
    if candidates.len() <= budget {
        return Vec::new();
    }
    let mut sorted = candidates.to_vec();
    sorted.sort_by_key(|(_, shown)| *shown);
    let excess = sorted.len() - budget;
    sorted.into_iter().take(excess).map(|(id, _)| id).collect()
}

/// Suspend hydrated background tabs until the warm pool fits the budget.
fn reconcile_warm_budget() {
    let candidates = tabs::hydrated_background_candidates(None, None);
    for id in excess_warm_tab_ids(&candidates, MAX_WARM_BACKGROUND) {
        tabs::suspend_tab(id);
    }
}

fn schedule_reconcile() {
    // Unit tests drive the tab store with no main run loop to service posts.
    if cfg!(test) {
        return;
    }
    let generation = {
        let mut guard = PENDING_RECONCILE
            .lock()
            .expect("suspend reconcile lock poisoned");
        *guard += 1;
        *guard
    };
    crate::main_thread::post_delayed(RECONCILE_GRACE_MS, move || {
        let still_valid = {
            let guard = PENDING_RECONCILE
                .lock()
                .expect("suspend reconcile lock poisoned");
            *guard == generation
        };
        if still_valid {
            reconcile_warm_budget();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excess_warm_tab_ids_keeps_small_sessions_warm() {
        let candidates = vec![(1, 100), (2, 200)];
        assert!(excess_warm_tab_ids(&candidates, MAX_WARM_BACKGROUND).is_empty());
    }

    #[test]
    fn excess_warm_tab_ids_discards_least_recent_first() {
        let candidates: Vec<_> = (1..=10).map(|id| (id, i64::from(id) * 1000)).collect();
        let excess = excess_warm_tab_ids(&candidates, 8);
        assert_eq!(excess, vec![1, 2]);
    }

    #[test]
    fn excess_warm_tab_ids_never_shown_are_first_out() {
        let candidates = vec![(1, 0), (2, 5000), (3, 0), (4, 9000)];
        let excess = excess_warm_tab_ids(&candidates, 2);
        assert_eq!(excess.len(), 2);
        assert!(excess.contains(&1));
        assert!(excess.contains(&3));
    }
}
