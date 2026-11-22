//! Imported cookies into the normal profile's persistent WebKit store.
//!
//! WebKit's cookie store is asynchronous: read the cookies it already has,
//! drop imports that would overwrite one (`import::cookies::without_existing`),
//! then set the rest and report once the last completion fires. Every block
//! here runs on the main thread, so `Rc`/`RefCell` state is enough.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_foundation::{
    NSArray, NSDate, NSHTTPCookie, NSHTTPCookieDomain, NSHTTPCookieExpires, NSHTTPCookieName,
    NSHTTPCookiePath, NSHTTPCookieSameSiteLax, NSHTTPCookieSameSitePolicy,
    NSHTTPCookieSameSiteStrict, NSHTTPCookieSecure, NSHTTPCookieValue, NSMutableDictionary,
    NSString,
};

use crate::import::cookies::{without_existing, CookieKey, ImportedCookie, SameSite};

type Done = Box<dyn FnOnce(usize, usize)>;

/// Set `cookies` in the normal profile, skipping any Ochre already holds.
/// `done(added, kept)` runs on the main thread once WebKit has stored them;
/// `kept` counts imports skipped because Ochre had that cookie already.
pub(crate) fn import(cookies: Vec<ImportedCookie>, done: impl FnOnce(usize, usize) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let (store, _) = super::profile_ctx(super::Profile::Normal, mtm);
    // SAFETY: main thread; the data store outlives this call.
    let cookie_store = unsafe { store.httpCookieStore() };
    let pending = RefCell::new(Some((cookies, Box::new(done) as Done)));
    let target = cookie_store.clone();
    let listed = RcBlock::new(move |all: NonNull<NSArray<NSHTTPCookie>>| {
        let Some((cookies, done)) = pending.borrow_mut().take() else {
            return;
        };
        // SAFETY: WebKit hands a valid array for the duration of the block.
        let all = unsafe { all.as_ref() };
        let existing: HashSet<CookieKey> = all
            .iter()
            .map(|c| {
                (
                    c.name().to_string(),
                    c.domain().to_string(),
                    c.path().to_string(),
                )
            })
            .collect();
        let total = cookies.len();
        let fresh = without_existing(cookies, &existing);
        set_all(&target, fresh, total, done);
    });
    // SAFETY: main thread; the block is retained by WebKit until it runs.
    unsafe { cookie_store.getAllCookies(&listed) };
}

fn set_all(
    store: &objc2_web_kit::WKHTTPCookieStore,
    fresh: Vec<ImportedCookie>,
    total: usize,
    done: Done,
) {
    let kept = total - fresh.len();
    let built: Vec<Retained<NSHTTPCookie>> = fresh.iter().filter_map(ns_cookie).collect();
    if built.is_empty() {
        done(0, kept);
        return;
    }
    let added = built.len();
    let remaining = Rc::new(Cell::new(added));
    let done = Rc::new(RefCell::new(Some(done)));
    for cookie in &built {
        let remaining = remaining.clone();
        let done = done.clone();
        let stored = RcBlock::new(move || {
            remaining.set(remaining.get() - 1);
            if remaining.get() == 0 {
                if let Some(done) = done.borrow_mut().take() {
                    done(added, kept);
                }
            }
        });
        // SAFETY: main thread; WebKit copies the cookie and retains the block.
        unsafe { store.setCookie_completionHandler(cookie, Some(&stored)) };
    }
}

fn ns_cookie(cookie: &ImportedCookie) -> Option<Retained<NSHTTPCookie>> {
    let props: Retained<NSMutableDictionary<NSString, AnyObject>> = NSMutableDictionary::new();
    let text = |key: &NSString, value: &str| props.insert(key, NSString::from_str(value).as_ref());
    // SAFETY: the property-key statics are immutable Foundation constants.
    unsafe {
        text(NSHTTPCookieDomain, &cookie.domain);
        text(NSHTTPCookieName, &cookie.name);
        text(NSHTTPCookieValue, &cookie.value);
        text(NSHTTPCookiePath, &cookie.path);
        if cookie.secure {
            text(NSHTTPCookieSecure, "TRUE");
        }
        match cookie.same_site {
            SameSite::Lax => {
                props.insert(NSHTTPCookieSameSitePolicy, NSHTTPCookieSameSiteLax.as_ref())
            }
            SameSite::Strict => {
                props.insert(
                    NSHTTPCookieSameSitePolicy,
                    NSHTTPCookieSameSiteStrict.as_ref(),
                );
            }
            SameSite::None | SameSite::Unspecified => {}
        }
    }
    if cookie.http_only {
        text(&NSString::from_str("HttpOnly"), "TRUE");
    }
    if let Some(expires) = cookie.expires {
        let date = NSDate::dateWithTimeIntervalSince1970(expires as f64);
        // SAFETY: immutable Foundation constant.
        props.insert(unsafe { NSHTTPCookieExpires }, date.as_ref());
    }
    // SAFETY: every key is an NSHTTPCookie property key with a value of the
    // documented type; an invalid set returns nil rather than raising.
    unsafe { NSHTTPCookie::cookieWithProperties(&props) }
}
