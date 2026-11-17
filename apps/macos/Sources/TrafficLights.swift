import AppKit
import SwiftUI

/// Custom-drawn macOS traffic lights, rendered as ordinary sidebar content
/// (ADR-0012). The real `NSWindow` buttons are hidden while windowed
/// (`AppDelegate`) and these stand in for them.
///
/// Why we draw our own instead of placing the system buttons: those buttons are
/// AppKit-owned titlebar chrome, and AppKit re-lays them out (position *and*
/// visibility) on its own schedule - focus changes, resize, miniaturize,
/// fullscreen, internal titlebar rebuilds. On a beta OS that churn left the real
/// buttons stranded and mispositioned no matter how we chased it. Drawing the
/// lights as SwiftUI views sidesteps the whole problem: they live in the
/// sidebar's view tree, so they slide/fade with it and are always positioned
/// correctly, and we own their look, hover, and actions outright.
///
/// Fidelity: colors, the hover-reveal glyphs, and the window-inactive gray state
/// match native. Fullscreen defers to the OS (its auto-hiding titlebar shows the
/// real buttons); the custom set steps aside there via `BrowserState.isFullscreen`.
struct TrafficLights: View {
    @ObservedObject private var state = BrowserState.shared
    @Environment(\.controlActiveState) private var activeState
    @State private var hovering = false

    /// The window these buttons drive. Single-window app, so the main/first
    /// window is unambiguous; close routes through `performClose` so it still
    /// hits `AppDelegate.windowShouldClose` (the graceful CEF teardown).
    private var window: NSWindow? { NSApp.mainWindow ?? NSApp.windows.first }

    /// A background window shows gray lights until hovered - matching native,
    /// where you can still reveal and click a background window's buttons.
    private var windowActive: Bool { activeState != .inactive }

    var body: some View {
        // Native (macOS Tahoe) metrics, measured against the real buttons:
        // 14pt circles, 9pt apart (23pt on center). Leading 5pt lands the first
        // circle's center ~24pt from the window edge, matching native.
        HStack(spacing: 9) {
            if !state.isFullscreen {
                Light(kind: .close, hovering: hovering, windowActive: windowActive) {
                    window?.performClose(nil)
                }
                Light(kind: .minimize, hovering: hovering, windowActive: windowActive) {
                    window?.miniaturize(nil)
                }
                Light(kind: .zoom, hovering: hovering, windowActive: windowActive) {
                    window?.toggleFullScreen(nil)
                }
            }
        }
        .padding(.leading, 5)
        // 70pt reserves room for the wider group before the sidebar toggle;
        // 26pt tall centers the circles on the nav-button row.
        .frame(width: 70, height: 26, alignment: .leading)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
    }
}

/// One traffic light: a colored circle that grays out when the window is
/// inactive and reveals its glyph while the cluster is hovered.
private struct Light: View {
    enum Kind { case close, minimize, zoom }

    let kind: Kind
    /// Hover is a property of the *cluster*, not the button - native reveals all
    /// three glyphs together whenever the pointer is over any of them.
    let hovering: Bool
    let windowActive: Bool
    let action: () -> Void

    /// Colored while the window is active, or on hover (so a background window's
    /// lights still light up under the pointer). Gray otherwise.
    private var lit: Bool { windowActive || hovering }

    var body: some View {
        Button(action: action) {
            ZStack {
                // Flat fill, no border - native (Tahoe) draws the circles borderless.
                Circle().fill(lit ? fill : Self.inactiveFill)
                if hovering { glyph }
            }
            .frame(width: 14, height: 14)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }

    @ViewBuilder private var glyph: some View {
        switch kind {
        case .close:
            Image(systemName: "xmark")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(glyphColor)
        case .minimize:
            Image(systemName: "minus")
                .font(.system(size: 10, weight: .heavy))
                .foregroundStyle(glyphColor)
        case .zoom:
            // Two triangles pointing to opposite corners - the native green
            // "enter fullscreen" glyph (green click toggles fullscreen).
            FullscreenGlyph()
                .fill(glyphColor)
                .frame(width: 8, height: 8)
        }
    }

    private var fill: Color {
        switch kind {
        case .close: Color(red: 1.0, green: 0.373, blue: 0.341)      // #FF5F57
        case .minimize: Color(red: 0.996, green: 0.737, blue: 0.180) // #FEBC2E
        case .zoom: Color(red: 0.157, green: 0.784, blue: 0.251)     // #28C840
        }
    }

    private var glyphColor: Color {
        switch kind {
        case .close: Color(red: 0.30, green: 0.0, blue: 0.0).opacity(0.85)
        case .minimize: Color(red: 0.40, green: 0.28, blue: 0.0).opacity(0.9)
        case .zoom: Color(red: 0.0, green: 0.32, blue: 0.05).opacity(0.85)
        }
    }

    private var label: String {
        switch kind {
        case .close: "Close"
        case .minimize: "Minimize"
        case .zoom: "Full Screen"
        }
    }

    /// Inactive gray, adapting to light/dark like the native buttons do.
    private static let inactiveFill = Color(nsColor: NSColor(name: nil) { appearance in
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? NSColor(white: 0.33, alpha: 1) : NSColor(white: 0.81, alpha: 1)
    })
}

/// The green button's hover glyph: filled triangles in the top-left and
/// bottom-right corners, with a diagonal channel between - the native "enter
/// full screen" mark.
private struct FullscreenGlyph: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX * 0.62, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.maxY * 0.62))
        path.closeSubpath()
        path.move(to: CGPoint(x: rect.maxX, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.maxX * 0.38, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY * 0.38))
        path.closeSubpath()
        return path
    }
}
