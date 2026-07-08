//! Runtime tab suspension policy — when to discard a background tab's CEF browser.
//!
//! ADR-0013's lazy hydration handles *restore-at-launch*; this module handles
//! *discard-while-running*: free renderer memory for tabs the user is not on,
//! reusing the same `browser: None` + `restore_tab` re-entry path.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use cef::*;

use crate::tabs::{self, SpaceId, TabId};

/// Grace before a tab left by tab-switch is suspended. Cancelled if the user
/// returns before this fires.
const SUSPEND_GRACE_MS: i64 = 30_000;

static PENDING_GENERATION: LazyLock<Mutex<HashMap<TabId, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Called from [`tabs::structural`] after a visibility / space change.
pub fn on_visibility_changed(
    old_visible: Option<TabId>,
    new_visible: Option<TabId>,
    old_space: SpaceId,
    new_space: SpaceId,
) {
    if new_space != old_space {
        suspend_hydrated_in_space(old_space, new_visible);
    }
    if let Some(new_id) = new_visible {
        cancel_pending(new_id);
    }
    if let Some(old_id) = old_visible {
        if Some(old_id) != new_visible && !tabs::is_favorite(old_id) {
            schedule_suspend(old_id);
        }
    }
}

/// Immediately suspend every hydrated, non-favorite background tab in `space`.
fn suspend_hydrated_in_space(space: SpaceId, keep_visible: Option<TabId>) {
    let ids = tabs::hydrated_background_in_space(space, keep_visible);
    for id in ids {
        cancel_pending(id);
        tabs::suspend_tab(id);
    }
}

fn schedule_suspend(id: TabId) {
    let generation = {
        let mut guard = PENDING_GENERATION
            .lock()
            .expect("suspend generation lock poisoned");
        let entry = guard.entry(id).or_insert(0);
        *entry += 1;
        *entry
    };
    let mut task = SuspendTabTask::new(id, generation);
    post_delayed_task(ThreadId::UI, Some(&mut task), SUSPEND_GRACE_MS);
}

fn cancel_pending(id: TabId) {
    let mut guard = PENDING_GENERATION
        .lock()
        .expect("suspend generation lock poisoned");
    let entry = guard.entry(id).or_insert(0);
    *entry += 1;
}

wrap_task! {
    struct SuspendTabTask {
        id: TabId,
        generation: u64,
    }

    impl Task {
        fn execute(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);
            let still_valid = {
                let guard = PENDING_GENERATION
                    .lock()
                    .expect("suspend generation lock poisoned");
                guard.get(&self.id) == Some(&self.generation)
            };
            if still_valid {
                tabs::suspend_tab(self.id);
            }
        }
    }
}
