import AppKit

/// Resolves a host's favicon, trying sources in order:
///   1. **Captured** — PNG/ICO bytes the engine fetched from the page's declared
///      icon URLs (via CEF's `on_favicon_urlchange` + direct HTTP fetch).
///   2. **Web fallback** (optional) — `https://<host>/favicon.ico`, then
///      DuckDuckGo's transparent proxy. Offline surfaces only (history, vault,
///      autocomplete).
///   3. **None** — globe glyph.
///
/// Tab rows prefer captured icons; web fallback covers sites the engine has not
/// cached yet (or SVG-only hosts where capture is still in flight).
@MainActor
final class FaviconLoader: ObservableObject {
    static let shared = FaviconLoader()

    private let cache = NSCache<NSString, NSImage>()

    func invalidate(host: String) {
        guard !host.isEmpty else { return }
        cache.removeObject(forKey: cacheKey(host: host, allowWebFallback: false) as NSString)
        cache.removeObject(forKey: cacheKey(host: host, allowWebFallback: true) as NSString)
    }

    func icon(for host: String, allowWebFallback: Bool = true) async -> NSImage? {
        guard !host.isEmpty else { return nil }
        let key = cacheKey(host: host, allowWebFallback: allowWebFallback) as NSString
        if let cached = cache.object(forKey: key) {
            return cached
        }
        var image = capturedFavicon(host)
        if image == nil, allowWebFallback {
            image = await fetchWebFallback(host)
        }
        if let image {
            cache.setObject(image, forKey: key)
        }
        return image
    }

    private func cacheKey(host: String, allowWebFallback: Bool) -> String {
        "\(host)#\(allowWebFallback ? "web" : "tab")"
    }

    private func capturedFavicon(_ host: String) -> NSImage? {
        var length = 0
        guard let raw = host.withCString({ browser_favicon_png($0, &length) }), length > 0 else {
            return nil
        }
        defer { browser_free_bytes(raw, length) }
        return imageFromData(Data(bytes: raw, count: length))
    }

    private func fetchWebFallback(_ host: String) async -> NSImage? {
        let paths = [
            "https://\(host)/favicon.ico",
            "https://\(host)/apple-touch-icon.png",
            "https://\(host)/apple-touch-icon-precomposed.png",
            "https://icons.duckduckgo.com/ip3/\(host).ico",
        ]
        for path in paths {
            if let image = await fetchImage(from: URL(string: path)) {
                return image
            }
        }
        return nil
    }

    private func fetchImage(from url: URL?) async -> NSImage? {
        guard let url,
            let (data, response) = try? await URLSession.shared.data(from: url),
            let http = response as? HTTPURLResponse,
            (200..<300).contains(http.statusCode)
        else {
            return nil
        }
        return imageFromData(data)
    }

    /// NSImage rejects some small ICO/PNG payloads unless given an explicit size.
    private func imageFromData(_ data: Data) -> NSImage? {
        guard let image = NSImage(data: data), !data.isEmpty else { return nil }
        image.size = NSSize(width: 16, height: 16)
        return image
    }
}
