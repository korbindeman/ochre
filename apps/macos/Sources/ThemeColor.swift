import SwiftUI

/// Parses CSS color strings streamed from `theme_color.rs` (`TabThemeColorChanged`).
enum ThemeColor {
    private struct RGBA {
        let r: Double
        let g: Double
        let b: Double
        let a: Double

        /// WCAG relative luminance — drives light vs. dark sidebar chrome.
        var relativeLuminance: Double {
            func linearize(_ channel: Double) -> Double {
                channel <= 0.03928 ? channel / 12.92 : pow((channel + 0.055) / 1.055, 2.4)
            }
            let red = linearize(r)
            let green = linearize(g)
            let blue = linearize(b)
            return 0.2126 * red + 0.7152 * green + 0.0722 * blue
        }

        /// Background reads as dark → sidebar should use light labels (`.dark` scheme).
        var isDarkBackground: Bool { relativeLuminance < 0.5 }
    }

    /// Sampled page color for the active tab, when sidebar tinting is enabled.
    static func sidebarTint(activeTab: TabItem?, enabled: Bool, isPrivate: Bool) -> Color? {
        guard enabled, !isPrivate else { return nil }
        guard let css = activeTab?.themeColor, !css.isEmpty else { return nil }
        return parse(css)
    }

    /// Sidebar label scheme that contrasts with the sampled tint, when tinting is on.
    static func sidebarColorScheme(activeTab: TabItem?, enabled: Bool, isPrivate: Bool)
        -> ColorScheme?
    {
        guard enabled, !isPrivate else { return nil }
        guard let css = activeTab?.themeColor, !css.isEmpty,
            let rgba = parseRGBA(css)
        else { return nil }
        return rgba.isDarkBackground ? .dark : .light
    }

    /// Parse `meta[name="theme-color"]` content or `getComputedStyle` output.
    static func parse(_ css: String) -> Color? {
        guard let rgba = parseRGBA(css) else { return nil }
        return Color(red: rgba.r, green: rgba.g, blue: rgba.b, opacity: rgba.a)
    }

    private static func parseRGBA(_ css: String) -> RGBA? {
        let trimmed = css.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if trimmed.hasPrefix("#") { return parseHexRGBA(trimmed) }
        let lower = trimmed.lowercased()
        if lower.hasPrefix("rgb") { return parseRGBRGBA(trimmed) }
        return nil
    }

    private static func parseHexRGBA(_ css: String) -> RGBA? {
        var hex = String(css.dropFirst())
        if hex.count == 3 || hex.count == 4 {
            hex = hex.map { String($0) + String($0) }.joined()
        }
        guard hex.count == 6 || hex.count == 8 else { return nil }
        var value: UInt64 = 0
        guard Scanner(string: hex).scanHexInt64(&value) else { return nil }
        let r: Double
        let g: Double
        let b: Double
        let a: Double
        if hex.count == 6 {
            r = Double((value >> 16) & 0xFF) / 255
            g = Double((value >> 8) & 0xFF) / 255
            b = Double(value & 0xFF) / 255
            a = 1
        } else {
            r = Double((value >> 24) & 0xFF) / 255
            g = Double((value >> 16) & 0xFF) / 255
            b = Double((value >> 8) & 0xFF) / 255
            a = Double(value & 0xFF) / 255
        }
        return RGBA(r: r, g: g, b: b, a: a)
    }

    private static func parseRGBRGBA(_ css: String) -> RGBA? {
        guard
            let open = css.firstIndex(of: "("),
            let close = css.lastIndex(of: ")"),
            open < close
        else { return nil }
        let body = css[css.index(after: open)..<close]
        let parts = body.split(separator: ",").map {
            $0.trimmingCharacters(in: .whitespaces)
        }
        guard parts.count == 3 || parts.count == 4 else { return nil }
        guard
            let r = parseRGBComponent(parts[0]),
            let g = parseRGBComponent(parts[1]),
            let b = parseRGBComponent(parts[2])
        else { return nil }
        let a = parts.count == 4 ? parseAlpha(parts[3]) ?? 1 : 1
        return RGBA(r: r, g: g, b: b, a: a)
    }

    private static func parseRGBComponent(_ part: String) -> Double? {
        if part.hasSuffix("%") {
            let num = Double(part.dropLast()) ?? -1
            guard num >= 0, num <= 100 else { return nil }
            return num / 100
        }
        let num = Double(part) ?? -1
        guard num >= 0, num <= 255 else { return nil }
        return num / 255
    }

    private static func parseAlpha(_ part: String) -> Double? {
        if part.hasSuffix("%") {
            let num = Double(part.dropLast()) ?? -1
            guard num >= 0, num <= 100 else { return nil }
            return num / 100
        }
        let num = Double(part) ?? -1
        guard num >= 0, num <= 1 else { return nil }
        return num
    }
}

/// Forces light or dark sidebar labels to contrast with a page-tinted background.
struct SidebarPageTintColorScheme: ViewModifier {
    var isPrivate: Bool = false
    @ObservedObject private var state = BrowserState.shared

    func body(content: Content) -> some View {
        if let scheme = ThemeColor.sidebarColorScheme(
            activeTab: state.activeTab,
            enabled: state.isSidebarPageTintEnabled,
            isPrivate: isPrivate
        ) {
            content
                .preferredColorScheme(scheme)
                .animation(.easeInOut(duration: 0.25), value: state.activeTab?.themeColor)
        } else {
            content
        }
    }
}

extension View {
    func sidebarPageTintColorScheme(isPrivate: Bool = false) -> some View {
        modifier(SidebarPageTintColorScheme(isPrivate: isPrivate))
    }
}

/// Sidebar background driven by the active tab's sampled page color.
struct SidebarPageTintBackground: View {
    var isPrivate: Bool = false
    @ObservedObject private var state = BrowserState.shared

    var body: some View {
        if let color = ThemeColor.sidebarTint(
            activeTab: state.activeTab,
            enabled: state.isSidebarPageTintEnabled,
            isPrivate: isPrivate
        ) {
            color
                .animation(.easeInOut(duration: 0.25), value: state.activeTab?.themeColor)
        }
    }
}
