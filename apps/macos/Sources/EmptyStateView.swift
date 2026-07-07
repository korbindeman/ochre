import AppKit
import SwiftUI

/// Shown in the content region when there are no tabs — the launch state and
/// the state after the last tab closes. Fully clean by design: a single, quiet
/// New-Tab hint on an otherwise empty canvas (a new-tab *page* with suggestions
/// is Phase 2).
struct EmptyStateView: View {
    var body: some View {
        Button {
            (NSApp.delegate as? AppDelegate)?.toggleCommandBar()
        } label: {
            HStack(spacing: 8) {
                Text("New Tab")
                Text("⌘T")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.tertiary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(RoundedRectangle(cornerRadius: 5).fill(.quaternary).opacity(0.7))
            }
            .font(.system(size: 14, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
        }
        .buttonStyle(IconButtonStyle(cornerRadius: 11, emphasizesLabel: false))
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
    }
}
