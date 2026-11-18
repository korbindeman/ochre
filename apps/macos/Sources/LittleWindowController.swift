import AppKit
import SwiftUI

/// A little window (P1-45, ADR-0067): one normal-profile tab in a small window
/// with a slim toolbar, outside the sidebar. ⌘O moves the live page into the
/// main window; closing the window closes the tab.
@MainActor
final class LittleWindowController: NSObject, NSWindowDelegate {
    static let defaultSize = NSSize(width: 960, height: 680)
    static let toolbarHeight: CGFloat = 38

    private(set) var tabId: Int32 = -1
    private(set) var window: NSWindow?
    private let container = CEFContainerView()
    /// Set once the tab moved into the main window, so closing the little
    /// window does not close the tab with it.
    private var promoted = false
    private let onClose: (LittleWindowController) -> Void

    init(onClose: @escaping (LittleWindowController) -> Void) {
        self.onClose = onClose
        super.init()
    }

    /// Build the window over `anchor` (the main window, when there is one) and
    /// create the tab. Returns false if the engine could not create it.
    func open(url: String, over anchor: NSWindow?) -> Bool {
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: Self.defaultSize),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.isReleasedWhenClosed = false
        window.minSize = NSSize(width: 420, height: 320)
        window.delegate = self
        place(window, over: anchor)

        let content = NSView(frame: window.contentRect(forFrameRect: window.frame))
        let bar = NSHostingView(rootView: LittleToolbar(controller: self))
        bar.frame = NSRect(
            x: 0, y: content.bounds.height - Self.toolbarHeight,
            width: content.bounds.width, height: Self.toolbarHeight)
        bar.autoresizingMask = [.width, .minYMargin]
        container.frame = NSRect(
            x: 0, y: 0, width: content.bounds.width,
            height: content.bounds.height - Self.toolbarHeight)
        container.autoresizingMask = [.width, .height]
        content.addSubview(container)
        content.addSubview(bar)
        window.contentView = content
        self.window = window

        let size = container.bounds.size
        tabId = url.withCString {
            browser_little_tab_create(
                Unmanaged.passUnretained(container).toOpaque(),
                Int32(size.width), Int32(size.height), $0)
        }
        guard tabId >= 0 else {
            NSLog("[little] browser_little_tab_create failed")
            window.delegate = nil
            window.close()
            return false
        }
        BrowserState.normal.reloadStructure()
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
        return true
    }

    /// The tab as the core last described it.
    var tab: TabItem? { BrowserState.normal.littleTabs.first { $0.id == tabId } }

    /// Move the page into the main window as the active open tab (⌘O).
    func promote() {
        guard !promoted, let main = (NSApp.delegate as? AppDelegate)?.mainWindowController else {
            return
        }
        promoted = true
        main.rootView.adoptTabView(tabId, container: container)
        guard browser_little_tab_promote(tabId) else { return }
        window?.close()
        main.window?.makeKeyAndOrderFront(nil)
        main.activate()
    }

    func close() { window?.close() }

    /// Cascade from the main window's center, like a new document window.
    private func place(_ window: NSWindow, over anchor: NSWindow?) {
        guard let anchor else {
            window.center()
            return
        }
        let frame = anchor.frame
        let size = window.frame.size
        let origin = NSPoint(
            x: frame.midX - size.width / 2 + 24,
            y: frame.midY - size.height / 2 - 24)
        window.setFrameOrigin(origin)
    }

    // MARK: - NSWindowDelegate

    func windowDidBecomeKey(_ notification: Notification) {
        BrowserState.current = BrowserState.normal
        browser_set_focused_little(tabId)
    }

    func windowWillClose(_ notification: Notification) {
        if !promoted, tabId >= 0 {
            browser_little_tab_close(tabId)
            container.detachCEFView()
        }
        browser_set_focused_little(-1)
        OverlayMachine.windowClosed(OverlayMachine.littleWindowId(tab: tabId))
        window?.delegate = nil
        onClose(self)
    }
}

/// The little window's toolbar: page identity (click to edit the address) and
/// the move-to-main-window button. Traffic lights sit to its left.
private struct LittleToolbar: View {
    let controller: LittleWindowController
    @ObservedObject private var state = BrowserState.normal

    var body: some View {
        let tab = controller.tab
        HStack(spacing: 8) {
            Button {
                (NSApp.delegate as? AppDelegate)?.editCurrentURL()
            } label: {
                HStack(spacing: 8) {
                    if let tab {
                        TabFavicon(tab: tab)
                    }
                    Text(title(tab))
                        .font(.system(size: 13, weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Spacer(minLength: 0)
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .help("Edit address (⌘L)")

            Button {
                controller.promote()
            } label: {
                Image(systemName: "arrow.up.forward.app")
                    .font(.system(size: 14, weight: .regular))
                    .frame(width: 26, height: 26)
            }
            .buttonStyle(IconButtonStyle())
            .help("Open in Main Window (⌘O)")
            .accessibilityLabel("Open in Main Window")
        }
        .padding(.leading, 78)
        .padding(.trailing, 8)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.bar)
    }

    private func title(_ tab: TabItem?) -> String {
        guard let tab else { return "" }
        if !tab.title.isEmpty { return tab.title }
        return URL(string: tab.url)?.host() ?? tab.url
    }
}
