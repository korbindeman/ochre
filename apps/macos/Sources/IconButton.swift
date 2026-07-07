import SwiftUI

/// The house style for chrome icon buttons (style §17): a quiet rounded fill that
/// appears on hover and *deepens on press* — the two-stage feedback native macOS /
/// Arc toolbar buttons give. Our custom `.plain` buttons previously showed hover at
/// best (via a hand-rolled `@State hovering` + `.background`) and never a pressed
/// state; this folds that boilerplate into one place and adds the press.
///
/// Animates with the shared hover curve (`Animation.hover`, `Motion.swift`) so it
/// moves like the rest of the chrome, and honors Reduce Motion.
///
/// - `cornerRadius` matches the button's container (the chip's corner, a pill).
/// - `isActive` keeps the fill lit at rest for a persistently-selected control
///   (the active space, the open Passwords toggle).
/// - `emphasizesLabel` brightens a plain glyph secondary → primary while hovered or
///   pressed and dims it to quaternary when disabled. Turn it *off* for buttons
///   whose color already encodes state (the copy button → green, the shield →
///   tint); those keep their own `foregroundStyle` and only take the fill.
struct IconButtonStyle: ButtonStyle {
    var cornerRadius: CGFloat = 6
    var isActive = false
    var emphasizesLabel = true

    func makeBody(configuration: Configuration) -> some View {
        Feedback(
            configuration: configuration,
            cornerRadius: cornerRadius,
            isActive: isActive,
            emphasizesLabel: emphasizesLabel
        )
    }

    /// A nested `View` (not a bare modifier) so the hover highlight can hold `@State`
    /// and read the environment's enabled / Reduce-Motion flags — a `ButtonStyle`
    /// body can't do either directly.
    private struct Feedback: View {
        let configuration: Configuration
        let cornerRadius: CGFloat
        let isActive: Bool
        let emphasizesLabel: Bool
        @Environment(\.isEnabled) private var isEnabled
        @Environment(\.accessibilityReduceMotion) private var reduceMotion
        @State private var hovering = false

        /// off → hover → press, each deeper; a selected control stays lit. `.quaternary`
        /// is faint, so the steps ride high. Disabled shows nothing (the glyph carries
        /// the unavailable read instead).
        private var fillOpacity: Double {
            guard isEnabled else { return 0 }
            if isActive || configuration.isPressed { return 1 }
            return hovering ? 0.6 : 0
        }

        /// Plain glyphs read secondary, brightening to primary while hovered or
        /// pressed; a disabled glyph dims to quaternary so it reads as unavailable.
        private var labelForeground: AnyShapeStyle {
            if !isEnabled { return AnyShapeStyle(.quaternary) }
            return (hovering || configuration.isPressed)
                ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary)
        }

        var body: some View {
            label
                .background(
                    RoundedRectangle(cornerRadius: cornerRadius)
                        .fill(.quaternary)
                        .opacity(fillOpacity)
                )
                .contentShape(.rect(cornerRadius: cornerRadius))
                .onHover { hovering = $0 }
                .animation(reduceMotion ? nil : .hover, value: hovering)
                .animation(reduceMotion ? nil : .hover, value: configuration.isPressed)
                .animation(reduceMotion ? nil : .hover, value: isActive)
        }

        /// Own the glyph color only when asked (`emphasizesLabel`); otherwise leave the
        /// caller's `foregroundStyle` untouched so state colors survive.
        @ViewBuilder private var label: some View {
            if emphasizesLabel {
                configuration.label.foregroundStyle(labelForeground)
            } else {
                configuration.label
            }
        }
    }
}
