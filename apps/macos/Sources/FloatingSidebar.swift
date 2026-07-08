import SwiftUI

/// The unpinned sidebar's content, shown floating over the web view. Solid
/// background on purpose: translucent materials sample their backdrop, and
/// CEF's content is a GPU-hosted remote layer that in-window blur cannot
/// sample reliably.
struct FloatingSidebar: View {
    var isPrivate: Bool = false

    var body: some View {
        Sidebar(variant: .floating, isPrivate: isPrivate)
            .frame(maxHeight: .infinity)
            .background {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(Color(nsColor: .windowBackgroundColor))
                    .shadow(color: .black.opacity(0.25), radius: 16, x: 3, y: 0)
            }
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
            }
    }
}
