import AppKit
import SwiftUI

/// A tab's favicon, resolved asynchronously through `FaviconLoader` and keyed by
/// host + favicon version so it refreshes on navigation and on capture. Shows a
/// spinner while the page loads and a globe glyph when no icon is available.
struct TabFavicon: View {
    let tab: TabItem
    /// Rendered glyph size. Tab rows use the default 16; the pinned strip renders
    /// a hair larger so the icon fills more of its spacious slot.
    var size: CGFloat = 16
    @State private var image: NSImage?

    var body: some View {
        Group {
            if tab.isLoading {
                // The small native spinner is 16x16 pt; scale it to the icon's
                // size so loading never changes the glyph's footprint.
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(size / 16)
            } else if let image {
                Image(nsImage: image)
                    .resizable()
                    .frame(width: size, height: size)
            } else {
                Image(systemName: "globe")
                    .font(.system(size: size))
                    .foregroundStyle(.secondary)
            }
        }
        .frame(width: size, height: size)
        .task(id: "\(tab.host)#\(tab.faviconVersion)") {
            image =
                tab.host.isEmpty
                ? nil
                : await FaviconLoader.shared.icon(for: tab.host, allowWebFallback: true)
        }
    }
}
