//! Run work on the main thread later, through the libdispatch main queue.
//!
//! WebKit and AppKit objects are main-thread only, and WebKit fires some
//! callbacks (KVO on `URL`, for one) synchronously inside our own calls, while
//! the caller may still hold a core lock. Core reactions to engine callbacks are
//! therefore always posted here, never run inline. The main queue is serviced
//! by the main run loop in every mode, so posted work also runs during menu
//! tracking and window resizes.

use std::ffi::c_void;

#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(
        queue: *const DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn dispatch_after_f(
        when: u64,
        queue: *const DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn dispatch_time(when: u64, delta: i64) -> u64;
}

type Work = Box<dyn FnOnce()>;

extern "C" fn trampoline(context: *mut c_void) {
    // SAFETY: `context` is the `Box<Work>` leaked by `enqueue`, consumed once.
    let work: Box<Work> = unsafe { Box::from_raw(context.cast()) };
    work();
}

fn main_queue() -> *const DispatchQueue {
    // `_dispatch_main_q` is libdispatch's static main queue object.
    std::ptr::addr_of!(_dispatch_main_q)
}

fn enqueue(delay_ms: i64, work: Work) {
    let context = Box::into_raw(Box::new(work)).cast::<c_void>();
    // SAFETY: the main queue is valid for the process; `trampoline` frees `context`.
    unsafe {
        if delay_ms <= 0 {
            dispatch_async_f(main_queue(), context, trampoline);
        } else {
            let when = dispatch_time(0, delay_ms.saturating_mul(1_000_000));
            dispatch_after_f(when, main_queue(), context, trampoline);
        }
    }
}

/// Run `work` on the main thread on a later run-loop turn. Callable from any thread.
pub fn post(work: impl FnOnce() + Send + 'static) {
    enqueue(0, Box::new(work));
}

/// Run `work` on the main thread after `delay_ms` milliseconds.
pub fn post_delayed(delay_ms: i64, work: impl FnOnce() + Send + 'static) {
    enqueue(delay_ms, Box::new(work));
}

/// Like [`post`], for work that holds main-thread-only values (WebKit objects).
/// Must be called on the main thread, which is where the work will run.
pub(crate) fn post_local(work: impl FnOnce() + 'static) {
    debug_assert!(
        objc2::MainThreadMarker::new().is_some(),
        "post_local off the main thread"
    );
    enqueue(0, Box::new(work));
}
