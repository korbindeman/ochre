import SwiftUI

/// The Liquid Glass search field plus its favicon autocomplete, drawn as one
/// continuous surface — the field flows straight into the results (with a
/// hairline divider between them) like Spotlight, rather than floating as two
/// detached bubbles.
///
/// The field text and inline-completion behaviour live in the shared
/// `AutocompleteField`; keyboard navigation (arrows/return/escape) is handled by
/// the host's key monitor in `CommandBarController`. This view just lays the two
/// out and plays the spring scale-in.
struct CommandBarView: View {
    @ObservedObject var controller: SuggestionController
    /// Called when a row is clicked — the host runs the suggestion and dismisses.
    var onChoose: (Suggestion) -> Void
    /// Transparent margin the host reserves around the panel. Padding the content
    /// by the same amount keeps the glass's own soft drop shadow *inside* the
    /// panel instead of hard-clipped at its rectangular edge.
    var shadowInset: CGFloat

    @State private var appeared = false

    private let cornerRadius: CGFloat = 24

    var body: some View {
        // Field and results are a single glass surface and shape: they share one
        // material and blend as a unit (Apple: never stack glass on glass).
        VStack(spacing: 0) {
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

            if !controller.suggestions.isEmpty {
                Divider()
                    .padding(.horizontal, 16)
                AutocompleteList(controller: controller, onChoose: onChoose)
                    .padding(8)
            }
        }
        .glassEffect(.regular, in: .rect(cornerRadius: cornerRadius))
        .scaleEffect(appeared ? 1 : 0.94, anchor: .top)
        .padding(shadowInset)
        .onAppear {
            withAnimation(.spring(response: 0.28, dampingFraction: 0.72)) {
                appeared = true
            }
        }
    }
}
