import AppKit
import SwiftUI

/// The active space's tree, rendered in the sidebar's scroll region: the pinned
/// section (folders, then loose pinned tabs) above the "today" (ephemeral) tabs,
/// with the New Tab row heading the today section (Arc placement).
/// Membership + order come from the core-owned `BrowserState` (ADR-0006).
struct SpaceContents: View {
    @ObservedObject private var state = BrowserState.shared
    @StateObject private var drag = TabDragModel()
    /// Namespace for the sliding active-tab highlight, shared across the loose,
    /// today, and folder rows so the selection can animate between any of them.
    @Namespace private var selection

    var body: some View {
        let folders = state.activeFolders
        let loose = state.loosePinnedTabs
        let today = state.todayTabs

        VStack(alignment: .leading, spacing: 4) {
            ForEach(folders) { folder in
                FolderRow(folder: folder, drag: drag, selection: selection)
            }
            sectionRows(loose, section: .loosePinned)
            // Separate the persistent section from the today section.
            if !folders.isEmpty || !loose.isEmpty {
                Divider()
                    .padding(.vertical, 4)
            }
            NewTabRow(looseEmpty: loose.isEmpty, firstToday: today.first?.id, drag: drag)
            sectionRows(today, section: .today)
        }
    }

    /// A section's tabs in order, each wired for drag reordering with a pointer
    /// to its next sibling (nil when last) so a below-the-midpoint drop knows
    /// whether to insert before it or append to the section.
    @ViewBuilder
    func sectionRows(_ tabs: [TabItem], section: TabSection) -> some View {
        ForEach(Array(tabs.enumerated()), id: \.element.id) { index, tab in
            tabRow(
                tab,
                section: section,
                next: index + 1 < tabs.count ? tabs[index + 1].id : nil
            )
        }
    }

    @ViewBuilder
    func tabRow(_ tab: TabItem, section: TabSection, next: Int32?) -> some View {
        TabRow(
            tab: tab,
            // A tab and a native surface are mutually exclusive — no tab reads as
            // active while History/Passwords covers the canvas.
            isActive: tab.id == state.activeTabId && !state.isSurfaceOpen,
            selection: selection,
            onSelect: { state.selectTab(tab.id) },
            onClose: { (NSApp.delegate as? AppDelegate)?.closeTab(tab.id) }
        )
        .contextMenu { TabMenu(tab: tab, folders: state.activeFolders) }
        .tabRowDrag(tab: tab, section: section, next: next, drag: drag)
        .transition(SidebarSelection.rowTransition)
    }
}

/// The New Tab row at the head of the today section — opens the command bar.
/// Styled like a tab row so the list reads as one column. It also doubles as the
/// drag boundary between the pinned and today sections (see `BoundaryDropDelegate`):
/// its indicators only appear for the section a tab row can't cover (an empty
/// loose-pinned or today section).
private struct NewTabRow: View {
    let looseEmpty: Bool
    let firstToday: Int32?
    @ObservedObject var drag: TabDragModel

    @State private var height: CGFloat = 36

    var body: some View {
        Button {
            (NSApp.delegate as? AppDelegate)?.toggleCommandBar()
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "plus")
                    .frame(width: 16, height: 16)
                Text("New Tab")
                Spacer(minLength: 0)
                Text("⌘T")
                    .foregroundStyle(.tertiary)
            }
            .font(.system(size: 13, weight: .medium))
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
        }
        .buttonStyle(IconButtonStyle(cornerRadius: 8, emphasizesLabel: false))
        .foregroundStyle(.secondary)
        .background(
            GeometryReader { geo in
                Color.clear.preference(key: RowHeightKey.self, value: geo.size.height)
            }
        )
        .onPreferenceChange(RowHeightKey.self) { height = $0 }
        .overlay(alignment: .top) {
            InsertionIndicator().opacity(looseEmpty && drag.slot == .endOf(.loosePinned) ? 1 : 0)
        }
        .overlay(alignment: .bottom) {
            InsertionIndicator().opacity(firstToday == nil && drag.slot == .endOf(.today) ? 1 : 0)
        }
        .onDrop(
            of: [.text],
            delegate: BoundaryDropDelegate(
                looseEmpty: looseEmpty,
                firstToday: firstToday,
                rowHeight: { height },
                drag: drag
            )
        )
    }
}

/// The right-click menu for a tab — pin/unpin, move between folders, close.
private struct TabMenu: View {
    let tab: TabItem
    let folders: [FolderItem]

    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }

    var body: some View {
        if tab.pinned {
            Button("Unpin Tab") { app?.setTabPinned(tab.id, false) }
            if !folders.isEmpty || tab.folder != nil {
                Menu("Move to Folder") {
                    Button("None") { app?.moveTab(tab.id, toFolder: nil) }
                    ForEach(folders) { folder in
                        Button(folder.name) { app?.moveTab(tab.id, toFolder: folder.id) }
                    }
                }
            }
        } else {
            Button("Pin Tab") { app?.setTabPinned(tab.id, true) }
        }
        Divider()
        Button("Close Tab") { app?.closeTab(tab.id) }
    }
}

/// A collapsible folder in the pinned section, with its pinned tabs indented
/// beneath. Collapse state is view-local (UI only, not in the core).
struct FolderRow: View {
    let folder: FolderItem
    @ObservedObject private var state = BrowserState.shared
    @ObservedObject var drag: TabDragModel
    /// Shared with the rest of the sidebar so the active-tab highlight can slide
    /// between a folder tab and a loose/today tab.
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
            .contentShape(.rect(cornerRadius: 8))
            .onTapGesture {
                withAnimation(.easeOut(duration: 0.15)) { collapsed.toggle() }
            }
            .contextMenu {
                Button("New Space from Folder") { app?.folderToSpace(folder.id) }
                Button("Rename Folder…") { app?.renameFolder(folder.id) }
                Divider()
                Button("Delete Folder") { app?.deleteFolder(folder.id) }
            }
            .onDrop(of: [.text], delegate: FolderDropDelegate(folderId: folder.id, drag: drag))

            if !collapsed {
                ForEach(Array(tabs.enumerated()), id: \.element.id) { index, tab in
                    TabRow(
                        tab: tab,
                        isActive: tab.id == state.activeTabId && !state.isSurfaceOpen,
                        selection: selection,
                        onSelect: { state.selectTab(tab.id) },
                        onClose: { app?.closeTab(tab.id) }
                    )
                    .contextMenu { TabMenu(tab: tab, folders: state.activeFolders) }
                    .tabRowDrag(
                        tab: tab,
                        section: .folder(folder.id),
                        next: index + 1 < tabs.count ? tabs[index + 1].id : nil,
                        drag: drag
                    )
                    .padding(.leading, 16)
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
            Button { app?.showVault() } label: {
                Image(systemName: "key.fill")
                    .font(.system(size: 12, weight: .semibold))
                    .frame(width: 26, height: 26)
            }
            .buttonStyle(IconButtonStyle(cornerRadius: 7, isActive: state.isVaultOpen, emphasizesLabel: false))
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

            Button { app?.newSpace() } label: {
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
        Button { browser_select_space(space.id) } label: {
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
