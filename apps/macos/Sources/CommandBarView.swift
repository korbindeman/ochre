import SwiftUI

/// Presentation state for the command bar's glass morph - owned by
/// `CommandBarController` so AppKit dismiss can drive the morph-out before the
/// panel tears down.
@MainActor
final class CommandBarPresentation: ObservableObject {
    @Published var isPresented = false
}

/// The Liquid Glass search field plus its favicon autocomplete, drawn as one
/// continuous surface - the field flows straight into the results (with a
/// hairline divider between them) like Spotlight, rather than floating as two
/// detached bubbles.
///
/// The field text and inline-completion behaviour live in the shared
/// `AutocompleteField`; keyboard navigation (arrows/return/escape) is handled by
/// the host's key monitor in `CommandBarController`. Open and close are one
/// fade-and-settle driven by `CommandBarPresentation`.
struct CommandBarView: View {
    @ObservedObject var presentation: CommandBarPresentation
    @ObservedObject var controller: SuggestionController
    /// Called when a row is clicked - the host runs the suggestion and dismisses.
    var onChoose: (Suggestion) -> Void
    /// Transparent margin the host reserves around the panel. Padding the content
    /// by the same amount keeps the glass's own soft drop shadow *inside* the
    /// panel instead of hard-clipped at its rectangular edge.
    var shadowInset: CGFloat
    /// Visible width of the glass surface (matches `CommandBarController.barWidth`).
    var barWidth: CGFloat
    /// Prefilled field text (selected on focus), e.g. the URL being edited.
    var initialQuery: String = ""

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private let cornerRadius: CGFloat = 24

    /// One motion for open and close: the whole surface fades and settles from
    /// a hair smaller, anchored at the top like Spotlight. No capsule morph and
    /// no window-alpha fade: Liquid Glass does not reliably follow either, which
    /// left an empty glass bubble on screen after close. Result rows change
    /// instantly, matching the panel's instant resize.
    private var presentAnimation: Animation {
        reduceMotion ? .linear(duration: 0.06) : .easeOut(duration: 0.14)
    }

    var body: some View {
        expandedSurface
            .glassEffect(.regular.interactive(), in: .rect(cornerRadius: cornerRadius))
            .frame(width: barWidth)
            .opacity(presentation.isPresented ? 1 : 0)
            .scaleEffect(
                presentation.isPresented || reduceMotion ? 1 : 0.97, anchor: .top)
            .padding(shadowInset)
            .animation(presentAnimation, value: presentation.isPresented)
            .onAppear { presentation.isPresented = true }
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
            AutocompleteField(
                controller: controller,
                initialQuery: initialQuery,
                font: .system(size: 22),
                selectAllOnFocus: !initialQuery.isEmpty
            )
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, minHeight: 60)
    }
}
