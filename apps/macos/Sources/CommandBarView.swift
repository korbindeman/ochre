import SwiftUI

/// The Liquid Glass search field plus its favicon autocomplete dropdown.
///
/// The field text and inline-completion behaviour live in the shared
/// `AutocompleteField`; keyboard navigation (arrows/return/escape) is handled by
/// the host's key monitor in `CommandBarController`. This view just lays the two
/// out and plays the spring scale-in.
struct CommandBarView: View {
    @ObservedObject var controller: SuggestionController
    /// Called when a row is clicked — the host runs the suggestion and dismisses.
    var onChoose: (Suggestion) -> Void

    @State private var appeared = false

    var body: some View {
        // The field and its results are two glass surfaces stacked together, so
        // group them in one container: they share a sampling region and blend as
        // one material (Apple: never stack glass on glass).
        GlassEffectContainer {
            VStack(spacing: 8) {
                HStack(spacing: 12) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 20, weight: .regular))
                        .foregroundStyle(.secondary)
                    // Larger than a default field so the placeholder / typed text
                    // reads at Spotlight's scale, and matched to the icon weight so
                    // the whole search line is one consistent size.
                    AutocompleteField(controller: controller, font: .system(size: 22))
                }
                .padding(.horizontal, 24)
                .frame(maxWidth: .infinity, minHeight: 60)
                .glassEffect(.regular, in: .capsule)

                if !controller.suggestions.isEmpty {
                    AutocompleteList(controller: controller, onChoose: onChoose)
                        .padding(6)
                        .glassEffect(.regular, in: .rect(cornerRadius: 16))
                }
            }
        }
        .scaleEffect(appeared ? 1 : 0.94, anchor: .top)
        .onAppear {
            withAnimation(.spring(response: 0.28, dampingFraction: 0.72)) {
                appeared = true
            }
        }
    }
}
