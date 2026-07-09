import AppKit

// Entry point. CEF must own `NSApp` — it installs a `CefAppProtocol`-conformant
// NSApplication subclass inside `browser_initialize` — before any AppKit code
// touches the shared application, so bootstrapping CEF is the very first thing
// that runs, ahead of building any window.
guard browser_initialize() else {
    // Initialization failed (framework load / CEF init). Nothing more to do.
    exit(1)
}

// Uncaught ObjC exceptions (common in AppKit callbacks) bypass Rust's panic hook.
NSSetUncaughtExceptionHandler { exception in
    browser_crash_report_swift_exception(
        exception.name.rawValue,
        exception.reason ?? ""
    )
}

// Top-level code in `main.swift` is not statically MainActor-isolated, but a
// process entry point always runs on the main thread — assert that to call
// the MainActor-bound app delegate. The returned reference is stored in a
// global: `NSApplication.delegate` is weak, so someone must own the delegate
// for the process lifetime.
let delegate = MainActor.assumeIsolated { () -> AppDelegate in
    let delegate = AppDelegate()
    NSApplication.shared.delegate = delegate
    delegate.makeWindow()
    return delegate
}

// P0-1 focus-routing regression (ADR-0018): `--focus-selftest` drives a scripted
// set of focus transitions with synthesized input and exits non-zero on any
// failure. Retained in a global — like the delegate — for the run's lifetime.
var focusSelfTest: FocusSelfTest?
if CommandLine.arguments.contains("--focus-selftest") {
    focusSelfTest = MainActor.assumeIsolated {
        let test = FocusSelfTest(app: delegate)
        // Kick off once the message loop is pumping the main queue.
        DispatchQueue.main.async { MainActor.assumeIsolated { test.run() } }
        return test
    }
}

// P1-27 sidebar drag regression (ADR-0035): `--drag-selftest` drives the drag
// pipeline at its gesture entry points against live row frames and exits
// non-zero on any failure.
var dragSelfTest: DragSelfTest?
if CommandLine.arguments.contains("--drag-selftest") {
    dragSelfTest = MainActor.assumeIsolated {
        let test = DragSelfTest(app: delegate)
        DispatchQueue.main.async { MainActor.assumeIsolated { test.run() } }
        return test
    }
}

// P1-30 ⌘W close-path regression (ADR-0037): `--close-selftest` drives ⌘W
// through the menu against ordinary / saved / no tabs and asserts the window
// only closes from the empty canvas. Exits non-zero on any failure.
var closeSelfTest: CloseSelfTest?
if CommandLine.arguments.contains("--close-selftest") {
    closeSelfTest = MainActor.assumeIsolated {
        let test = CloseSelfTest(app: delegate)
        DispatchQueue.main.async { MainActor.assumeIsolated { test.run() } }
        return test
    }
}

// Hand the run loop to CEF; it drives the AppKit event loop internally until
// the window closes (which calls `browser_quit`).
browser_run_message_loop()

// Loop returned — tear CEF down and exit cleanly.
NSLog("[main] message loop returned — shutting down")
browser_shutdown()
