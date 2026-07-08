import AppKit
import SwiftUI

/// The History page — the first native app-UI surface, and the reference the
/// other internal pages (settings, bookmarks, downloads) follow (ADR-0010,
/// style §15).
///
/// It is a *content-area surface*, not a tab: `RootView` hosts it in an
/// `NSHostingView` shown over the web content and dismissed back to the active
/// tab (like the empty state). Glass lives on the functional top bar only — the
/// content list uses native materials and rows (style §3, §13). All data comes
/// from the core over the history FFI; this view owns none of it.
struct HistoryView: View {
    @ObservedObject private var model = HistoryModel.shared

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .safeAreaBar(edge: .top, spacing: 0) { Toolbar(model: model) }
            .background(Color(nsColor: .windowBackgroundColor))
            .onChange(of: model.query) { _, _ in model.reload() }
            .onExitCommand { BrowserState.shared.closeHistory() }
    }

    @ViewBuilder private var content: some View {
        if model.entries.isEmpty {
            EmptyState(query: model.query)
        } else {
            // Day titles are plain rows, not `Section` headers — pinned headers
            // draw a system hairline under themselves in the plain style, which
            // reads as a heavy border against this airy layout.
            List {
                ForEach(model.sections) { section in
                    Text(section.title)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .listRowInsets(EdgeInsets(top: 14, leading: 24, bottom: 6, trailing: 24))
                        .listRowSeparator(.hidden)
                    ForEach(section.entries) { entry in
                        HistoryRow(
                            entry: entry,
                            onOpen: { open(entry) },
                            onDelete: { model.delete(entry) }
                        )
                        .listRowInsets(EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16))
                        .listRowSeparator(.hidden)
                    }
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            // The floating glass toolbar (`safeAreaBar`) has no bar material
            // behind it by design (style §247), so content is fed to the scroll
            // edge effect to fade it out under the title instead of letting rows
            // read through the header. The bottom inset lets the last row scroll
            // clear of the window edge instead of hugging it — note
            // `contentMargins(.scrollContent)` is a no-op on `List`, so this uses
            // `safeAreaPadding`, which reserves the bottom scroll inset.
            .scrollEdgeEffectStyle(.soft, for: .top)
            .safeAreaPadding(.bottom, 20)
        }
    }

    private func open(_ entry: HistoryEntry) {
        (NSApp.delegate as? AppDelegate)?.openTab(url: entry.url, windowId: 0)
        BrowserState.shared.closeHistory()
    }
}

// ─── Top bar (the glass functional layer) ──────────────────────────────────

/// Title + live Liquid Glass search + a Clear menu, floating over the content
/// as a `safeAreaBar` (the list scrolls under it with the system's scroll
/// edge effect). Glass sits on the capsule controls themselves — the macOS 26
/// toolbar idiom (Notes, Safari) — not on a stacked bar material (style §3,
/// §13); everything below is native content.
private struct Toolbar: View {
    @ObservedObject var model: HistoryModel

    var body: some View {
        // Group the glass controls in one container so they share a sampling
        // region and blend as a single material (never stack glass on glass).
        GlassEffectContainer {
            HStack(spacing: 12) {
                // Leading dismiss — a surface is exclusive with the tabs (ADR-0010),
                // so ✕ returns to the page (Escape also works). Kept clear of the
                // trailing Clear control so it isn't fired by accident.
                Button { BrowserState.shared.closeHistory() } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 12, weight: .semibold))
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .help("Close")
                Text("History")
                    .font(.system(size: 24, weight: .bold))
                Spacer(minLength: 16)
                SearchField(text: $model.query)
                    .frame(maxWidth: 280)
                Menu("Clear") {
                    Button("Clear Last Hour") { model.clear(withinLast: 3600) }
                    Button("Clear Today") { model.clearToday() }
                    Divider()
                    Button("Clear All History", role: .destructive) { model.clear(since: 0) }
                }
                .menuStyle(.button)
                .buttonStyle(.glass)
                .fixedSize()
                .disabled(model.entries.isEmpty && model.query.isEmpty)
            }
            .padding(.horizontal, 24)
            .padding(.top, 20)
            .padding(.bottom, 14)
        }
    }
}

/// The standard Liquid Glass search capsule (the command bar owns full search;
/// this only filters the visible list).
private struct SearchField: View {
    @Binding var text: String

    var body: some View {
        HStack(spacing: 7) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
            TextField("Search history", text: $text)
                .textFieldStyle(.plain)
            if !text.isEmpty {
                Button { text = "" } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
            }
        }
        .font(.system(size: 13))
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .glassEffect(.regular, in: .capsule)
    }
}

// ─── Content ────────────────────────────────────────────────────────────────

/// One history row: favicon · title / URL · trailing time, with a hover-revealed
/// remove button (the time yields to it on hover). Click opens the page in a new
/// tab; the context menu adds Copy Link and Remove.
private struct HistoryRow: View {
    let entry: HistoryEntry
    let onOpen: () -> Void
    let onDelete: () -> Void

    @State private var hovering = false

    var body: some View {
        HStack(spacing: 12) {
            HostFavicon(host: entry.host)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.displayTitle)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                Text(prettyURL(entry.url))
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 12)
            if hovering {
                Button(action: onDelete) {
                    Image(systemName: "xmark")
                        .font(.system(size: 10, weight: .bold))
                        .frame(width: 22, height: 22)
                }
                .buttonStyle(IconButtonStyle(cornerRadius: 6))
                .help("Remove from History")
            } else {
                Text(timeLabel(entry.lastVisit))
                    .font(.system(size: 11))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(.quaternary)
                .opacity(hovering ? 0.6 : 0)
                .hoverFade(hovering)
        )
        .contentShape(.rect)
        .onTapGesture(perform: onOpen)
        .onHover { hovering = $0 }
        .contextMenu {
            Button("Open", action: onOpen)
            Button("Copy Link") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(entry.url, forType: .string)
            }
            Divider()
            Button("Remove", role: .destructive, action: onDelete)
        }
    }
}

/// Empty state — distinct copy for "no history at all" vs. "search found
/// nothing" (style §10).
private struct EmptyState: View {
    let query: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: query.isEmpty ? "clock" : "magnifyingglass")
                .font(.system(size: 32, weight: .light))
                .foregroundStyle(.tertiary)
            Text(query.isEmpty ? "No history yet" : "No results for “\(query)”")
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(.secondary)
            if query.isEmpty {
                Text("Pages you visit will appear here.")
                    .font(.system(size: 12))
                    .foregroundStyle(.tertiary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// ─── Model ──────────────────────────────────────────────────────────────────

/// One history-page row, decoded from `browser_history_list`. Keys arrive
/// snake_case and decode via `.convertFromSnakeCase`.
struct HistoryEntry: Decodable, Identifiable {
    let url: String
    let title: String
    let host: String
    /// Unix seconds of the most recent visit.
    let lastVisit: Int64
    let visitCount: Int64

    var id: String { url }
    var displayTitle: String { title.isEmpty ? (host.isEmpty ? url : host) : title }
}

/// A day's worth of history rows, for the list's section grouping.
struct HistorySection: Identifiable {
    let id: String
    let title: String
    let entries: [HistoryEntry]

    /// Group recency-ordered `entries` into day sections (Today / Yesterday /
    /// date), preserving the incoming order within and across days.
    static func group(_ entries: [HistoryEntry]) -> [HistorySection] {
        let calendar = Calendar.current
        var order: [Date] = []
        var byDay: [Date: [HistoryEntry]] = [:]
        for entry in entries {
            let day = calendar.startOfDay(
                for: Date(timeIntervalSince1970: TimeInterval(entry.lastVisit)))
            if byDay[day] == nil { order.append(day) }
            byDay[day, default: []].append(entry)
        }
        return order.map { day in
            HistorySection(id: day.ISO8601Format(), title: dayTitle(day, calendar), entries: byDay[day] ?? [])
        }
    }
}

/// Observable state for the History surface — a thin projection of the core's
/// store. Reloaded when the surface opens (`RootView`) and on every query change.
@MainActor
final class HistoryModel: ObservableObject {
    static let shared = HistoryModel()

    @Published var query = ""
    @Published private(set) var entries: [HistoryEntry] = []

    /// Cap the page — a first cut lists the most recent `pageLimit` visits;
    /// infinite scroll (offset paging) is a follow-up if it's ever needed.
    private static let pageLimit: UInt32 = 500

    var sections: [HistorySection] { HistorySection.group(entries) }

    /// Reload from the core for the current query.
    func reload() {
        entries = Self.fetch(query: query, limit: Self.pageLimit, offset: 0)
    }

    /// Remove one entry — optimistically drop it locally, then tell the core.
    func delete(_ entry: HistoryEntry) {
        _ = entry.url.withCString { browser_history_delete($0) }
        entries.removeAll { $0.url == entry.url }
    }

    /// Clear entries last visited at or after `since` (Unix seconds); 0 clears all.
    func clear(since: Int64) {
        _ = browser_history_clear(since)
        reload()
    }

    /// Clear entries visited within the last `seconds`.
    func clear(withinLast seconds: Int64) {
        clear(since: Int64(Date().timeIntervalSince1970) - seconds)
    }

    /// Clear everything from local midnight onward.
    func clearToday() {
        clear(since: Int64(Calendar.current.startOfDay(for: Date()).timeIntervalSince1970))
    }

    private static func fetch(query: String, limit: UInt32, offset: UInt32) -> [HistoryEntry] {
        let raw = query.withCString { browser_history_list($0, limit, offset) }
        guard let raw else { return [] }
        defer { browser_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        return (try? decoder.decode([HistoryEntry].self, from: data)) ?? []
    }

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()
}

// ─── Date formatting ──────────────────────────────────────────────────────

/// Section header for a day: "Today" / "Yesterday" / "Wednesday, July 2".
private func dayTitle(_ day: Date, _ calendar: Calendar) -> String {
    if calendar.isDateInToday(day) { return "Today" }
    if calendar.isDateInYesterday(day) { return "Yesterday" }
    return day.formatted(.dateTime.weekday(.wide).month(.wide).day())
}

/// Trailing per-row time of day (e.g. "3:45 PM").
private func timeLabel(_ unix: Int64) -> String {
    Date(timeIntervalSince1970: TimeInterval(unix)).formatted(date: .omitted, time: .shortened)
}
