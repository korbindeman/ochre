import AppKit
import SwiftUI

/// Shared UI-motion helpers for the chrome.
///
/// The native macOS idiom for a hover highlight is a short cross-fade of the
/// background (Finder/Safari sidebars, toolbar buttons), not an instant snap —
/// `onHover` driving an animated state change. Everything here honors Reduce
/// Motion (style §7): under it the animation is dropped so nothing moves, matching
/// the Reduce-Motion gate the structure-reload spring already uses in `BrowserState`.

enum MotionPolicy {
    /// Whether the user has Reduce Motion enabled at the OS level.
    static var reduceMotion: Bool {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }
}

extension Animation {
    /// The chrome's hover / press cross-fade — a short (~120ms) ease-out, the native
    /// toolbar-button feel. Shared so `hoverFade` and `IconButtonStyle` move alike.
    /// Callers gate it on Reduce Motion (pass `nil`) rather than baking the gate in,
    /// since an `Animation` can't be conditionally absent.
    static let hover: Animation = .easeOut(duration: 0.12)
}

extension View {
    /// Fade a hover-driven background instead of snapping it. Apply to the highlight
    /// *shape* (inside `.background`), so only its opacity animates — never the row's
    /// content or a hover-revealed control. The shared `.hover` curve; instant under
    /// Reduce Motion. (Icon *buttons* use `IconButtonStyle` instead, which folds this
    /// in and adds the pressed state; this is for tap-gesture rows and the chip.)
    func hoverFade(_ hovering: Bool) -> some View {
        modifier(HoverFade(hovering: hovering))
    }
}

private struct HoverFade: ViewModifier {
    let hovering: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        content.animation(reduceMotion ? nil : .hover, value: hovering)
    }
}
