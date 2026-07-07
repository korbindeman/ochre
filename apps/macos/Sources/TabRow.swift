import SwiftUI

/// A single vertical tab: favicon (or spinner while loading), title, and a
/// hover-revealed close button. The whole row selects; only the ✕ closes.
/// (Non-private so the spaces/folders tree in `Spaces.swift` reuses it.)
struct TabRow: View {
    let tab: TabItem
    let isActive: Bool
    /// Shared namespace for the sliding active-tab highlight (`matchedGeometryEffect`):
    /// the selection pill is a single element that animates from the old active row
    /// to the new one on tab switch, rather than cross-fading in place.
    let selection: Namespace.ID
    let onSelect: () -> Void
    let onClose: () -> Void

    @State private var hovering = false

    var body: some View {
        HStack(spacing: 8) {
            TabFavicon(tab: tab)
                .frame(width: 16, height: 16)
            Text(tab.displayTitle)
                .lineLimit(1)
            Spacer(minLength: 0)
            if hovering {
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(.system(size: 10, weight: .bold))
                        // Match the 16pt favicon height so the button never becomes
                        // the tallest child and grows the row on hover; the 18pt
                        // width keeps a comfortable horizontal hit target.
                        .frame(width: 18, height: 16)
                }
                .buttonStyle(IconButtonStyle(cornerRadius: 5))
                .help("Close Tab")
            }
        }
        .font(.system(size: 13, weight: .medium))
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .background {
            // The active pill is the single matched element (slides between rows).
            // The hover highlight is a separate, always-present fill (opacity 0 when
            // idle) so it fades rather than snaps and never joins the slide. Only one
            // row is active at a time, so the pill has one source.
            RoundedRectangle(cornerRadius: 8)
                .fill(.quaternary)
                .opacity(!isActive && hovering ? 0.45 : 0)
                .hoverFade(hovering)
            if isActive {
                RoundedRectangle(cornerRadius: 8)
                    .fill(.quaternary)
                    .matchedGeometryEffect(id: SidebarSelection.pill, in: selection)
            }
        }
        .contentShape(.rect(cornerRadius: 8))
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
    }
}

/// Shared identifiers/curves for the sidebar's tab animations, so the tab list and
/// the folder tree animate identically (they render `TabRow` from two files).
enum SidebarSelection {
    /// `matchedGeometryEffect` id for the sliding active-tab highlight.
    static let pill = "sidebar.activeTab"

    /// Enter/exit for a tab row: grow in from the favicon edge + fade; fade out on
    /// close (the surrounding rows spring to close the gap via the list animation).
    static let rowTransition: AnyTransition = .asymmetric(
        insertion: .scale(scale: 0.96, anchor: .leading).combined(with: .opacity),
        removal: .opacity
    )
}
