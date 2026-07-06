import AppKit

/// Resolves a host's favicon, trying sources in order and caching the result:
///   1. **Captured** — the favicon the engine saved while browsing, via the
///      Rust store (`browser_favicon_png`). Instant, offline, exact.
///   2. **Web service** — a favicon service keyed by host. Default is Google's
///      (`s2/favicons`); swap `serviceURL` to use another (e.g. DuckDuckGo:
///      `https://icons.duckduckgo.com/ip3/<host>.ico`).
///   3. **None** — the caller falls back to a globe glyph.
///
/// The pluggable `serviceURL` is what keeps the web fallback engine-agnostic
/// while defaulting to Google, matching the search default.
@MainActor
final class FaviconLoader: ObservableObject {
    static let shared = FaviconLoader()

    /// Maps a host to the favicon-service URL for the web fallback. Replace to
    /// change providers globally.
    var serviceURL: (String) -> URL? = { host in
        URL(string: "https://www.google.com/s2/favicons?domain=\(host)&sz=32")
    }

    private let cache = NSCache<NSString, NSImage>()

    /// Drop any cached icon for `host` so the next `icon(for:)` re-resolves it.
    /// Called when the engine captures a favicon (`TabFaviconChanged`), so the
    /// exact captured PNG replaces any web-service fallback already cached.
    func invalidate(host: String) {
        guard !host.isEmpty else { return }
        cache.removeObject(forKey: host as NSString)
    }

    /// The favicon for `host`, or nil if no source yields one.
    func icon(for host: String) async -> NSImage? {
        guard !host.isEmpty else { return nil }
        if let cached = cache.object(forKey: host as NSString) {
            return cached
        }
        // Captured favicon first (instant), then the web service.
        var image = capturedFavicon(host)
        if image == nil {
            image = await fetchFavicon(host)
        }
        if let image {
            cache.setObject(image, forKey: host as NSString)
        }
        return image
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

    /// Favicon fetched from the configured web service.
    private func fetchFavicon(_ host: String) async -> NSImage? {
        guard let url = serviceURL(host),
              let (data, _) = try? await URLSession.shared.data(from: url)
        else {
            return nil
        }
        return NSImage(data: data)
    }
}
