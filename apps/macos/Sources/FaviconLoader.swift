import AppKit

/// Resolves a host's favicon, trying sources in order:
///   1. **Captured** — the favicon the engine saved while browsing, via the
///      Rust store (`browser_favicon_png`). Instant, offline, exact.
///   2. **Web fallback** (optional) — `https://<host>/favicon.ico`, then
///      DuckDuckGo's icon proxy. Used only for offline surfaces (history, vault,
///      autocomplete) where the host may never have been visited.
///   3. **None** — the caller falls back to a globe glyph.
///
/// Tab rows pass `allowWebFallback: false` and show the globe until capture
/// lands, so third-party compositors never paint a white plate behind icons.
@MainActor
final class FaviconLoader: ObservableObject {
    static let shared = FaviconLoader()

    private let cache = NSCache<NSString, NSImage>()

    /// Drop any cached icon for `host` so the next resolve re-reads the store.
    /// Called when the engine captures a favicon (`TabFaviconChanged`).
    func invalidate(host: String) {
        guard !host.isEmpty else { return }
        cache.removeObject(forKey: cacheKey(host: host, allowWebFallback: false) as NSString)
        cache.removeObject(forKey: cacheKey(host: host, allowWebFallback: true) as NSString)
    }

    /// The favicon for `host`, or nil if no source yields one.
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

    /// Favicon PNG the engine captured for this host while browsing.
    private func capturedFavicon(_ host: String) -> NSImage? {
        var length = 0
        guard let raw = host.withCString({ browser_favicon_png($0, &length) }), length > 0 else {
            return nil
        }
        defer { browser_free_bytes(raw, length) }
        return NSImage(data: Data(bytes: raw, count: length))
    }

    /// Direct site ICO, then DuckDuckGo's transparent proxy.
    private func fetchWebFallback(_ host: String) async -> NSImage? {
        if let image = await fetchImage(from: URL(string: "https://\(host)/favicon.ico")) {
            return image
        }
        return await fetchImage(from: URL(string: "https://icons.duckduckgo.com/ip3/\(host).ico"))
    }

    private func fetchImage(from url: URL?) async -> NSImage? {
        guard let url,
              let (data, response) = try? await URLSession.shared.data(from: url),
              let http = response as? HTTPURLResponse,
              (200 ..< 300).contains(http.statusCode),
              let image = NSImage(data: data),
              image.isValid
        else {
            return nil
        }
        return image
    }
}
