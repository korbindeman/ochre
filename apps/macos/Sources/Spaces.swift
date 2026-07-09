import AppKit
import SwiftUI

/// The active space's tree, rendered in the sidebar's scroll region: the saved
/// section (folders, then loose saved tabs) above open tabs, with the New Tab
/// row between them. Membership + order come from the core-owned `BrowserState`
/// (ADR-0006).
struct SpaceContents: View {
    @ObservedObject private var state = BrowserState.shared
    @ObservedObject var drag: TabDragModel
    /// Namespace for the sliding active-tab highlight, shared across saved and
    /// open rows so the selection can animate between any of them.
    @Namespace private var selection

    var body: some View {
        let folders = state.rootFolders
        let loose = state.looseSavedTabs
        let open = state.openTabs
        let hasSaved = !folders.isEmpty || !loose.isEmpty

        VStack(alignment: .leading, spacing: 4) {
            if let space = state.activeSpace {
                TabSectionHeader(title: space.name)
            }
            ForEach(folders) { folder in
                FolderRow(folder: folder, drag: drag, selection: selection)
            }
            sectionRows(loose)
            if hasSaved {
                Divider()
                    .padding(.vertical, 4)
            }
            NewTabRow(looseEmpty: loose.isEmpty, firstOpen: open.first?.id)
            if !open.isEmpty {
                TabSectionHeader(
                    title: "Open",
                    actionTitle: "Clear",
                    actionHelp: "Close open tabs (selected tabs are kept)",
                    onAction: { (NSApp.delegate as? AppDelegate)?.clearOpenTabs() }
                )
            }
            sectionRows(open)
            // Arc-style: the unfilled scroll viewport below the last row drags the
            // window; tab rows above keep their own drag-reorder affordance.
            Spacer(minLength: 0)
                .frame(maxWidth: .infinity)
                .overlay {
                    if !drag.isDragging {
                        SidebarEmptyDragArea {
                            URLBarModel.shared.endEditing()
                            BrowserState.shared.clearTabSelection()
                        }
                    }
                }
        }
        .background(ScrollViewProbe(drag: drag))
    }

    /// A section's tabs in order, each a drag source; the shared resolver derives
    /// insertion points from their reported frames.
    @ViewBuilder
    func sectionRows(_ tabs: [TabItem]) -> some View {
        ForEach(tabs) { tab in
            tabRow(tab)
        }
    }

    @ViewBuilder
    func tabRow(_ tab: TabItem) -> some View {
        let app = NSApp.delegate as? AppDelegate
        let isActive = tab.id == state.activeTabId && !state.isSurfaceOpen
        TabRow(
            tab: tab,
            // A tab and a native surface are mutually exclusive — no tab reads as
            // active while History/Passwords/Settings covers the canvas.
            isActive: isActive,
            isMultiSelected: state.selectedTabIds.contains(tab.id),
            selection: selection,
            onSelect: { state.handleTabClick(tab.id) },
            onClose: { app?.closeTab(tab.id) },
            onHideSaved: tab.pinned && !tab.favorite && isActive
                ? { app?.hideSavedTab(tab.id) } : nil
        )
        .contextMenu { TabMenu(tab: tab) }
        .sidebarTabDrag(tab: tab, drag: drag)
        .reportSidebarDropFrame(.tab(tab.id))
        .transition(SidebarSelection.rowTransition)
    }
}

/// The New Tab row between saved and open — opens the command bar. Styled like a
/// tab row so the list reads as one column. Its reported frame doubles as the
/// drag boundary between saved and open (see `SidebarDropResolver`).
private struct NewTabRow: View {
    let looseEmpty: Bool
    let firstOpen: Int32?

    @State private var hovering = false

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "plus")
                .frame(width: 16, height: 16)
            Text("New Tab")
            Spacer(minLength: 0)
            Text("⌘T")
                .foregroundStyle(.tertiary)
        }
        .font(.system(size: 13, weight: .medium))
        .foregroundStyle(.secondary)
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .background {
            RoundedRectangle(cornerRadius: 8)
                .fill(.quaternary)
                .opacity(hovering ? 0.45 : 0)
                .hoverFade(hovering)
        }
        .contentShape(.rect)
        .onTapGesture { (NSApp.delegate as? AppDelegate)?.toggleCommandBar() }
        .onHover { hovering = $0 }
        .reportSidebarDropFrame(.newTab(looseEmpty: looseEmpty, firstOpen: firstOpen))
    }
}

/// The right-click menu for a tab — save/move to open, folders, close.
private struct TabMenu: View {
    let tab: TabItem
    @ObservedObject private var state = BrowserState.shared

    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }
    private var folders: [FolderItem] { state.activeFolders }

    private var bulkIds: [Int32] {
        state.hasMultiTabSelection && state.selectedTabIds.contains(tab.id)
            ? Array(state.selectedTabIds).sorted()
            : []
    }

    var body: some View {
        if !bulkIds.isEmpty {
            Button("New Folder from \(bulkIds.count) Tabs") {
                app?.createFolderFromTabs(bulkIds)
            }
            Button("Close \(bulkIds.count) Tabs") {
                app?.closeTabs(bulkIds)
            }
            Divider()
        }
        if tab.favorite {
            Button("Unpin Tab") { app?.setTabFavorite(tab.id, false) }
        } else {
            Button("Pin Tab") { app?.setTabFavorite(tab.id, true) }
                .disabled(!state.canPinMoreFavorites)
        }
        if tab.pinned {
            Button("Move to Open") { app?.setTabSaved(tab.id, false) }
            if !folders.isEmpty || tab.folder != nil {
                Menu("Move to Folder") {
                    Button("None") { app?.moveTab(tab.id, toFolder: nil) }
                    ForEach(folders) { folder in
                        Button(folder.name) { app?.moveTab(tab.id, toFolder: folder.id) }
                    }
                }
            }
        } else if !tab.favorite {
            Button("Save Tab") { app?.setTabSaved(tab.id, true) }
        }
        Divider()
        if tab.favorite {
            if tab.id == state.activeTabId && !state.isSurfaceOpen {
                Button("Close Pinned Tab") { app?.closeTab(tab.id) }
            }
            Button("Delete Pinned Tab") { app?.deleteTab(tab.id) }
        } else if tab.pinned {
            if tab.id == state.activeTabId && !state.isSurfaceOpen {
                Button("Close Tab") { app?.hideSavedTab(tab.id) }
            } else {
                Button("Delete Saved Tab") { app?.closeTab(tab.id) }
            }
        } else {
            Button("Close Tab") { app?.closeTab(tab.id) }
        }
    }
}

/// Section label (optional) with an optional trailing action — e.g. Saved, or Clear alone.
private struct TabSectionHeader: View {
    var title: String?
    var actionTitle: String?
    var actionHelp: String?
    var onAction: (() -> Void)?

    var body: some View {
        HStack {
            if let title {
                Text(title)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            Spacer(minLength: 0)
            if let actionTitle, let onAction {
                Button(actionTitle, action: onAction)
                    .buttonStyle(IconButtonStyle(cornerRadius: 5, emphasizesLabel: false))
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .help(actionHelp ?? "")
            }
        }
        .padding(.horizontal, 10)
        .padding(.top, 2)
    }
}

/// A collapsible folder in the saved section, with nested subfolders and saved
/// tabs indented beneath. Collapse state is view-local (UI only, not in the core).
struct FolderRow: View {
    let folder: FolderItem
    var depth: Int = 0
    @ObservedObject private var state = BrowserState.shared
    @ObservedObject var drag: TabDragModel
    /// Shared with the rest of the sidebar so the active-tab highlight can slide
    /// between a folder tab and a loose/open tab.
    let selection: Namespace.ID
    @State private var collapsed = true
    @State private var hovering = false

    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }
    private var indent: CGFloat { CGFloat(16 * (depth + 1)) }

    /// The header is lit while a dragged tab is aimed at this folder.
    private var isDropTarget: Bool { drag.slot == .ontoFolder(folder.id) }

    var body: some View {
        let tabs = state.tabs(inFolder: folder.id)
        let children = state.childFolders(of: folder.id)
        let itemCount = tabs.count + children.count

        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image(systemName: collapsed ? "folder" : "folder.fill")
                    .contentTransition(.symbolEffect(.replace))
                    .foregroundStyle(.secondary)
                Text(folder.name)
                    .lineLimit(1)
                Spacer(minLength: 0)
                Text("\(itemCount)")
                    .font(.system(size: 11))
                    .monospacedDigit()
                    .foregroundStyle(.tertiary)
            }
            .font(.system(size: 13, weight: .medium))
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .padding(.leading, CGFloat(depth) * 16)
            .background {
                RoundedRectangle(cornerRadius: 8)
                    .fill(.quaternary)
                    .opacity(!isDropTarget && hovering ? 0.45 : 0)
                    .hoverFade(hovering)
                RoundedRectangle(cornerRadius: 8)
                    .fill(.tint)
                    .opacity(isDropTarget ? 0.22 : 0)
            }
            .contentShape(.rect)
            .onHover { hovering = $0 }
            .onTapGesture {
                withAnimation(.easeOut(duration: 0.15)) { collapsed.toggle() }
            }
            .contextMenu {
                Button("New Space from Folder") { app?.folderToSpace(folder.id) }
                Button("Rename Folder…") { app?.renameFolder(folder.id) }
                Divider()
                Button("Delete Folder") { app?.deleteFolder(folder.id) }
            }
            .reportSidebarDropFrame(.folder(folder.id))

            if !collapsed {
                ForEach(children) { child in
                    FolderRow(
                        folder: child,
                        depth: depth + 1,
                        drag: drag,
                        selection: selection
                    )
                }
                ForEach(tabs) { tab in
                    let isActive = tab.id == state.activeTabId && !state.isSurfaceOpen
                    TabRow(
                        tab: tab,
                        isActive: isActive,
                        isMultiSelected: state.selectedTabIds.contains(tab.id),
                        selection: selection,
                        onSelect: { state.handleTabClick(tab.id) },
                        onClose: { app?.closeTab(tab.id) },
                        onHideSaved: tab.pinned && !tab.favorite && isActive
                            ? { app?.hideSavedTab(tab.id) } : nil
                    )
                    .contextMenu { TabMenu(tab: tab) }
                    .sidebarTabDrag(tab: tab, drag: drag)
                    // Report before the indent padding so the frame (and the
                    // insertion indicator derived from it) is the visible row.
                    .reportSidebarDropFrame(.tab(tab.id))
                    .padding(.leading, indent)
                    .transition(SidebarSelection.rowTransition)
                }
            }
        }
    }
}

/// The bottom-of-sidebar space switcher: a scrollable row of emoji icons (one
/// per space, Arc-style) plus a "+" to create a new space. The name shows as a
/// tooltip; rename/icon/delete live in the context menu.
struct SpaceSwitcher: View {
    @ObservedObject private var state = BrowserState.shared

    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }

    var body: some View {
        HStack(spacing: 6) {
            // Passwords (vault) — the sidebar entry point to the manager (also in
            // the app menu). Leading utility button; spaces scroll to its right.
            Button {
                app?.showVault()
            } label: {
                Image(systemName: "key.fill")
                    .font(.system(size: 12, weight: .semibold))
                    .frame(width: 26, height: 26)
            }
            .buttonStyle(
                IconButtonStyle(
                    cornerRadius: 7, isActive: state.isVaultOpen, emphasizesLabel: false)
            )
            .foregroundStyle(state.isVaultOpen ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
            .help("Passwords")

            ScrollView(.horizontal) {
                HStack(spacing: 6) {
                    ForEach(state.spaces) { space in
                        SpacePill(
                            space: space,
                            isActive: space.id == state.activeSpaceId,
                            canDemote: state.spaces.count > 1 && space.id != state.activeSpaceId,
                            canDelete: state.spaces.count > 1
                        )
                    }
                }
            }
            .scrollIndicators(.never)

            Button {
                app?.newSpace()
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 12, weight: .semibold))
                    .frame(width: 26, height: 26)
            }
            .buttonStyle(IconButtonStyle(cornerRadius: 7))
            .help("New Space")
        }
    }
}

/// One space in the switcher — its emoji icon, with the active space carried by
/// a pill background. Click selects; right-click manages it (new folder,
/// rename, icon, convert-to-folder, delete). The tooltip names the space.
private struct SpacePill: View {
    let space: SpaceItem
    let isActive: Bool
    let canDemote: Bool
    let canDelete: Bool

    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }

    var body: some View {
        Button {
            browser_select_space(space.id)
        } label: {
            Text(space.icon)
                .font(.system(size: 13))
                .frame(width: 26, height: 26)
        }
        .buttonStyle(IconButtonStyle(cornerRadius: 7, isActive: isActive))
        .help(space.name)
        .contextMenu {
            if isActive {
                Button("New Folder…") { app?.newFolder() }
            }
            Button("Rename Space…") { app?.renameSpace(space.id) }
            Button("Change Icon…") { app?.changeSpaceIcon(space.id) }
            if canDemote {
                Button("Convert to Folder") { app?.spaceToFolder(space.id) }
            }
            if canDelete {
                Divider()
                Button("Delete Space") { app?.deleteSpace(space.id) }
            }
        }
    }
}
