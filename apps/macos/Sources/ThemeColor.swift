import SwiftUI

/// Parses CSS color strings streamed from `theme_color.rs` (`TabThemeColorChanged`).
enum ThemeColor {
    /// Sampled page color for the active tab, when sidebar tinting is enabled.
    static func sidebarTint(activeTab: TabItem?, enabled: Bool, isPrivate: Bool) -> Color? {
        guard enabled, !isPrivate else { return nil }
        guard let css = activeTab?.themeColor, !css.isEmpty else { return nil }
        return parse(css)
    }

    /// Parse `meta[name="theme-color"]` content or `getComputedStyle` output.
    static func parse(_ css: String) -> Color? {
        let trimmed = css.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if trimmed.hasPrefix("#") { return parseHex(trimmed) }
        let lower = trimmed.lowercased()
        if lower.hasPrefix("rgb") { return parseRGB(trimmed) }
        return nil
    }

    private static func parseHex(_ css: String) -> Color? {
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
        return Color(red: r, green: g, blue: b, opacity: a)
    }

    private static func parseRGB(_ css: String) -> Color? {
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
        return Color(red: r, green: g, blue: b, opacity: a)
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
