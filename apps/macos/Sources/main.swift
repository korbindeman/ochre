import AppKit

// Entry point. CEF must own `NSApp` — it installs a `CefAppProtocol`-conformant
// NSApplication subclass inside `browser_initialize` — before any AppKit code
// touches the shared application, so bootstrapping CEF is the very first thing
// that runs, ahead of building any window.
guard browser_initialize() else {
    // Initialization failed (framework load / CEF init). Nothing more to do.
    exit(1)
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
// Hand the run loop to CEF; it drives the AppKit event loop internally until
// the window closes (which calls `browser_quit`).
browser_run_message_loop()

// Loop returned — tear CEF down and exit cleanly.
browser_shutdown()
