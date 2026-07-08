import SwiftUI

/// Presentation state for the command bar's glass morph — owned by
/// `CommandBarController` so AppKit dismiss can drive the morph-out before the
/// panel tears down.
@MainActor
final class CommandBarPresentation: ObservableObject {
    @Published var isPresented = false
}

/// The Liquid Glass search field plus its favicon autocomplete, drawn as one
/// continuous surface — the field flows straight into the results (with a
/// hairline divider between them) like Spotlight, rather than floating as two
/// detached bubbles.
///
/// The field text and inline-completion behaviour live in the shared
/// `AutocompleteField`; keyboard navigation (arrows/return/escape) is handled by
/// the host's key monitor in `CommandBarController`. The expand/collapse uses the
/// platform's `glassEffectID` morph inside a `GlassEffectContainer` (mvp M3).
struct CommandBarView: View {
    @ObservedObject var presentation: CommandBarPresentation
    @ObservedObject var controller: SuggestionController
    /// Called when a row is clicked — the host runs the suggestion and dismisses.
    var onChoose: (Suggestion) -> Void
    /// Transparent margin the host reserves around the panel. Padding the content
    /// by the same amount keeps the glass's own soft drop shadow *inside* the
    /// panel instead of hard-clipped at its rectangular edge.
    var shadowInset: CGFloat
    /// Visible width of the glass surface (matches `CommandBarController.barWidth`).
    var barWidth: CGFloat

    @Namespace private var glassNamespace
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private let cornerRadius: CGFloat = 24
    private var morphAnimation: Animation? {
        reduceMotion ? nil : .spring(response: 0.32, dampingFraction: 0.78)
    }

    var body: some View {
        GlassEffectContainer {
            Group {
                if presentation.isPresented {
                    expandedSurface
                        .glassEffect(.regular.interactive(), in: .rect(cornerRadius: cornerRadius))
                } else {
                    // Collapsed morph origin — a compact capsule the glass expands
                    // from on present (Spotlight-style).
                    Color.clear
                        .frame(height: 28)
                        .glassEffect(.regular.interactive(), in: .capsule)
                }
            }
            .frame(width: barWidth)
            .glassEffectID("command-bar", in: glassNamespace)
        }
        .padding(shadowInset)
        .animation(morphAnimation, value: presentation.isPresented)
        .animation(morphAnimation, value: controller.suggestions.isEmpty)
        .onAppear {
            withAnimation(morphAnimation) {
                presentation.isPresented = true
            }
        }
    }

    private var expandedSurface: some View {
        VStack(spacing: 0) {
            fieldRow

            if !controller.suggestions.isEmpty {
                Divider()
                    .padding(.horizontal, 16)
                AutocompleteList(controller: controller, onChoose: onChoose)
                    .padding(8)
            }
        }
    }

    private var fieldRow: some View {
        HStack(spacing: 12) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 20, weight: .regular))
                .foregroundStyle(.secondary)
            AutocompleteField(controller: controller, font: .system(size: 22))
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, minHeight: 60)
    }
}
