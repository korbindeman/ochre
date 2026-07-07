import AppKit
import SwiftUI

/// A tab's favicon, resolved asynchronously through `FaviconLoader` and keyed by
/// host + favicon version so it refreshes on navigation and on capture. Shows a
/// spinner while the page loads and a globe glyph when no icon is available.
struct TabFavicon: View {
    let tab: TabItem
    @State private var image: NSImage?

    var body: some View {
        Group {
            if tab.isLoading {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.6)
            } else if let image {
                Image(nsImage: image)
                    .resizable()
                    .frame(width: 16, height: 16)
            } else {
                Image(systemName: "globe")
                    .foregroundStyle(.secondary)
            }
        }
        .task(id: "\(tab.host)#\(tab.faviconVersion)") {
            image = tab.host.isEmpty
                ? nil
                : await FaviconLoader.shared.icon(for: tab.host, allowWebFallback: false)
        }
    }
}
