import SwiftUI

/// The unpinned sidebar's content, shown floating over the web view. Solid
/// background on purpose: translucent materials sample their backdrop, and
/// CEF's content is a GPU-hosted remote layer that in-window blur cannot
/// sample reliably.
struct FloatingSidebar: View {
    var isPrivate: Bool = false
    @ObservedObject private var state = BrowserState.shared

    private var cardFill: Color {
        ThemeColor.sidebarTint(
            activeTab: state.activeTab,
            enabled: state.isSidebarPageTintEnabled,
            isPrivate: isPrivate
        ) ?? Color(nsColor: .windowBackgroundColor)
    }

    var body: some View {
        Sidebar(variant: .floating, isPrivate: isPrivate)
            .frame(maxHeight: .infinity)
            .background {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(cardFill)
                    .animation(.easeInOut(duration: 0.25), value: state.activeTab?.themeColor)
                    .shadow(color: .black.opacity(0.25), radius: 16, x: 3, y: 0)
            }
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
            }
    }
}
