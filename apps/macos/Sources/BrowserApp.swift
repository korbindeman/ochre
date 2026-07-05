import SwiftUI

@main
struct BrowserApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
                .frame(minWidth: 800, minHeight: 600)
        }
        .windowStyle(.hiddenTitleBar)
    }
}

struct ContentView: View {
    @State private var title: String = "Browser"

    var body: some View {
        VStack(spacing: 0) {
            // Slim top toolbar — minimal controls (back/forward/reload)
            toolbar

            Divider()

            // Web content region — CEF NSView will be hosted here via NSViewRepresentable.
            // During the spike this is a placeholder; real CEF embedding lands in M2.
            ZStack {
                Color(nsColor: .windowBackgroundColor)
                Text("CEF WebView Placeholder")
                    .font(.headline)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var toolbar: some View {
        HStack(spacing: 12) {
            Button(action: {}) {
                Image(systemName: "chevron.left")
            }
            .disabled(true)

            Button(action: {}) {
                Image(systemName: "chevron.right")
            }
            .disabled(true)

            Button(action: {}) {
                Image(systemName: "arrow.clockwise")
            }
            .disabled(true)

            Spacer()

            Text(title)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)

            Spacer()

            Button(action: {}) {
                Image(systemName: "sidebar.left")
            }
            .disabled(true)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}

#Preview {
    ContentView()
}
