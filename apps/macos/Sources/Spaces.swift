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
    @State private var sidebarDropFrames: [SidebarDropFrame] = []

    var body: some View {
        let folders = state.activeFolders
        let loose = state.looseSavedTabs
        let open = state.openTabs
        let hasSaved = !folders.isEmpty || !loose.isEmpty

        VStack(alignment: .leading, spacing: 4) {
            ForEach(folders) { folder in
                FolderRow(folder: folder, drag: drag, selection: selection)
            }
            sectionRows(loose, section: .looseSaved)
            if hasSaved {
                Divider()
                    .padding(.vertical, 4)
            }
            if loose.isEmpty, drag.slot == .endOf(.looseSaved) {
                TabDropPlaceholder()
            }
            NewTabRow(looseEmpty: loose.isEmpty, firstOpen: open.first?.id, drag: drag)
            if open.isEmpty, drag.slot == .endOf(.open) {
                TabDropPlaceholder()
            }
            if !open.isEmpty {
                TabSectionHeader(
                    title: "Open",
                    actionTitle: "Clear",
                    actionHelp: "Close open tabs (selected tabs are kept)",
                    onAction: { (NSApp.delegate as? AppDelegate)?.clearOpenTabs() }
                )
            }
            sectionRows(open, section: .open)
            // Arc-style: the unfilled scroll viewport below the last row drags the
            // window; tab rows above keep their own `onDrag` reorder affordance.
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
        .coordinateSpace(.named(SidebarDropSpace.name))
        .onPreferenceChange(SidebarDropFramesKey.self) { sidebarDropFrames = $0 }
        .overlay {
            if isDraggingFavorite {
                FavoriteSidebarDropLayer(drag: drag, frames: sidebarDropFrames)
            }
        }
    }

    private var isDraggingFavorite: Bool {
        guard let id = drag.draggingId else { return false }
        return state.tabs.first { $0.id == id }?.favorite ?? false
    }

    /// A section's tabs in order, each wired for drag reordering with a pointer
    /// to its next sibling (nil when last) so a below-the-midpoint drop knows
    /// whether to insert before it or append to the section.
    @ViewBuilder
    func sectionRows(_ tabs: [TabItem], section: TabSection) -> some View {
        ForEach(Array(tabs.enumerated()), id: \.element.id) { index, tab in
            if drag.slot == .before(tab.id) {
                TabDropPlaceholder()
            }
            tabRow(
                tab,
                section: section,
                next: index + 1 < tabs.count ? tabs[index + 1].id : nil
            )
        }
        if drag.slot == .endOf(section) {
            TabDropPlaceholder()
        }
    }

    @ViewBuilder
    func tabRow(_ tab: TabItem, section: TabSection, next: Int32?) -> some View {
        TabRow(
            tab: tab,
            // A tab and a native surface are mutually exclusive — no tab reads as
            // active while History/Passwords/Settings covers the canvas.
            isActive: tab.id == state.activeTabId && !state.isSurfaceOpen,
            isMultiSelected: state.selectedTabIds.contains(tab.id),
            selection: selection,
            onSelect: { state.handleTabClick(tab.id) },
            onClose: { (NSApp.delegate as? AppDelegate)?.closeTab(tab.id) }
        )
        .contextMenu { TabMenu(tab: tab) }
        .tabRowDrag(tab: tab, section: section, next: next, drag: drag)
        .reportSidebarDropFrame(.tab(tab.id))
        .transition(SidebarSelection.rowTransition)
    }
}

/// The New Tab row between saved and open — opens the command bar. Styled like a
/// tab row so the list reads as one column. It also doubles as the drag boundary
/// between saved and open (see `BoundaryDropDelegate`).
private struct NewTabRow: View {
    let looseEmpty: Bool
    let firstOpen: Int32?
    @ObservedObject var drag: TabDragModel

    @State private var height: CGFloat = 36
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
        .background(
            GeometryReader { geo in
                Color.clear.preference(key: RowHeightKey.self, value: geo.size.height)
            }
        )
        .onPreferenceChange(RowHeightKey.self) { height = $0 }
        .reportSidebarDropFrame(.newTab(looseEmpty: looseEmpty, firstOpen: firstOpen))
        .onDrop(
            of: [.text],
            delegate: BoundaryDropDelegate(
                looseEmpty: looseEmpty,
                firstOpen: firstOpen,
                rowHeight: { height },
                drag: drag
            )
        )
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
        Button(tab.favorite ? "Close Pinned Tab" : "Close Tab") { app?.closeTab(tab.id) }
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

/// A collapsible folder in the saved section, with its saved tabs indented
/// beneath. Collapse state is view-local (UI only, not in the core).
struct FolderRow: View {
    let folder: FolderItem
    @ObservedObject private var state = BrowserState.shared
    @ObservedObject var drag: TabDragModel
    /// Shared with the rest of the sidebar so the active-tab highlight can slide
    /// between a folder tab and a loose/open tab.
    let selection: Namespace.ID
    @State private var collapsed = false

    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }

    /// The header is lit while a dragged tab is aimed at this folder.
    private var isDropTarget: Bool { drag.slot == .ontoFolder(folder.id) }

    var body: some View {
        let tabs = state.tabs(inFolder: folder.id)

        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image(systemName: collapsed ? "chevron.right" : "chevron.down")
                    .font(.system(size: 10, weight: .semibold))
                    .contentTransition(.symbolEffect(.replace))
                    .foregroundStyle(.tertiary)
                    .frame(width: 12)
                Image(systemName: "folder")
                    .foregroundStyle(.secondary)
                Text(folder.name)
                    .lineLimit(1)
                Spacer(minLength: 0)
                Text("\(tabs.count)")
                    .font(.system(size: 11))
                    .monospacedDigit()
                    .foregroundStyle(.tertiary)
            }
            .font(.system(size: 13, weight: .medium))
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill(.tint)
                    .opacity(isDropTarget ? 0.22 : 0)
            )
            .contentShape(.rect)
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
            .onDrop(of: [.text], delegate: FolderDropDelegate(folderId: folder.id, drag: drag))

            if !collapsed {
                ForEach(Array(tabs.enumerated()), id: \.element.id) { index, tab in
                    if drag.slot == .before(tab.id) {
                        TabDropPlaceholder()
                            .padding(.leading, 16)
                    }
                    TabRow(
                        tab: tab,
                        isActive: tab.id == state.activeTabId && !state.isSurfaceOpen,
                        isMultiSelected: state.selectedTabIds.contains(tab.id),
                        selection: selection,
                        onSelect: { state.handleTabClick(tab.id) },
                        onClose: { app?.closeTab(tab.id) }
                    )
                    .contextMenu { TabMenu(tab: tab) }
                    .tabRowDrag(
                        tab: tab,
                        section: .folder(folder.id),
                        next: index + 1 < tabs.count ? tabs[index + 1].id : nil,
                        drag: drag
                    )
                    .padding(.leading, 16)
                    .reportSidebarDropFrame(.tab(tab.id))
                    .transition(SidebarSelection.rowTransition)
                }
                if drag.slot == .endOf(.folder(folder.id)) {
                    TabDropPlaceholder()
                        .padding(.leading, 16)
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
