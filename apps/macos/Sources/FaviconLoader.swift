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
    /// Dominant favicon color, cached per host. A fully transparent entry is the
    /// sentinel for "computed, no dominant hue" (a monochrome/gray favicon) so we
    /// don't re-extract on every redraw. Real dominants are always opaque.
    private let colorCache = NSCache<NSString, NSColor>()

    func invalidate(host: String) {
        guard !host.isEmpty else { return }
        cache.removeObject(forKey: cacheKey(host: host, allowWebFallback: false) as NSString)
        cache.removeObject(forKey: cacheKey(host: host, allowWebFallback: true) as NSString)
        colorCache.removeObject(forKey: "\(host)#dom" as NSString)
    }

    /// The favicon's dominant hue, or `nil` when it's essentially monochrome
    /// (black glyph, gray mark, mostly-white) — the caller falls back to a neutral
    /// chip. Extraction is saturation-gated so multi-color marks don't average to
    /// mud and near-neutral marks don't yield a muddy gray tint.
    func dominantColor(for host: String) async -> NSColor? {
        guard !host.isEmpty else { return nil }
        let key = "\(host)#dom" as NSString
        if let cached = colorCache.object(forKey: key) {
            return cached.alphaComponent == 0 ? nil : cached
        }
        let color = await icon(for: host, allowWebFallback: true).flatMap(Self.dominantHue)
        colorCache.setObject(color ?? .clear, forKey: key)
        return color
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

    /// Pick a representative brand color from a favicon.
    ///
    /// Downsamples to a small bitmap, bins opaque pixels into hue buckets weighted
    /// by saturation, and returns the saturation-weighted average of the dominant
    /// bucket. Transparent, near-black, near-white, and low-saturation pixels are
    /// ignored so a black glyph or gray mark reads as "no hue" (`nil`) rather than a
    /// muddy gray, and a multi-color mark resolves to its most vivid family instead
    /// of the gray that a flat average would produce.
    private static func dominantHue(from image: NSImage) -> NSColor? {
        guard let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
            return nil
        }
        let side = 24
        var px = [UInt8](repeating: 0, count: side * side * 4)
        guard
            let ctx = CGContext(
                data: &px, width: side, height: side, bitsPerComponent: 8,
                bytesPerRow: side * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        ctx.draw(cg, in: CGRect(x: 0, y: 0, width: side, height: side))

        let buckets = 12
        var weight = [Double](repeating: 0, count: buckets)
        var rSum = [Double](repeating: 0, count: buckets)
        var gSum = [Double](repeating: 0, count: buckets)
        var bSum = [Double](repeating: 0, count: buckets)
        var opaque = 0
        var colored = 0

        for i in stride(from: 0, to: px.count, by: 4) {
            let a = Double(px[i + 3]) / 255
            if a < 0.5 { continue }
            opaque += 1
            // Un-premultiply so edge pixels aren't darkened toward black.
            let r = Double(px[i]) / 255 / a
            let g = Double(px[i + 1]) / 255 / a
            let b = Double(px[i + 2]) / 255 / a
            let mx = max(r, g, b), mn = min(r, g, b), d = mx - mn
            let sat = mx == 0 ? 0 : d / mx
            // Gate out grays (low sat), near-black (low value), near-white.
            if sat < 0.25 || mx < 0.15 || (mx > 0.98 && sat < 0.12) { continue }
            colored += 1
            var hue = 0.0
            if d != 0 {
                if mx == r {
                    hue = (g - b) / d
                } else if mx == g {
                    hue = 2 + (b - r) / d
                } else {
                    hue = 4 + (r - g) / d
                }
                hue = (hue * 60).truncatingRemainder(dividingBy: 360)
                if hue < 0 { hue += 360 }
            }
            let bin = min(buckets - 1, Int(hue / 30))
            weight[bin] += sat
            rSum[bin] += r * sat
            gSum[bin] += g * sat
            bSum[bin] += b * sat
        }

        // A favicon that's mostly transparent-or-neutral has no brand hue to show.
        guard opaque > 0, Double(colored) / Double(opaque) >= 0.15 else { return nil }
        guard let top = weight.indices.max(by: { weight[$0] < weight[$1] }), weight[top] > 0 else {
            return nil
        }
        let w = weight[top]
        return NSColor(
            srgbRed: rSum[top] / w, green: gSum[top] / w, blue: bSum[top] / w, alpha: 1)
    }
}
