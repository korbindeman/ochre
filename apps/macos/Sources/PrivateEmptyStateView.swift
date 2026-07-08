import SwiftUI

/// Empty canvas for a private window — explains what's protected.
struct PrivateEmptyStateView: View {
    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "hand.raised.fill")
                .font(.system(size: 28))
                .foregroundStyle(.secondary)
            Text("Private Browsing")
                .font(.system(size: 18, weight: .semibold))
            Text(
                "This window won't save your history, cookies, or site data. " +
                "Downloads are still saved to your Mac. Ad blocking and cookie " +
                "notice rejection follow your current settings."
            )
            .font(.system(size: 13))
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .frame(maxWidth: 360)
            Button {
                (NSApp.delegate as? AppDelegate)?.toggleCommandBar()
            } label: {
                Text("New Private Tab")
                    .font(.system(size: 14, weight: .medium))
            }
            .buttonStyle(.glass)
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
    }
}
