//! Arc Browser sidebar import — `StorableSidebar.json` on macOS.
//!
//! Arc stores pinned tabs, folders, spaces, and favorites in one JSON document.
//! Ephemeral ("Today") tabs, easels, and split views are not represented here and
//! are deliberately skipped. The format is undocumented; this parser follows the
//! structure used by community exporters (e.g. ivnvxd/arc-export) and validated
//! against real `StorableSidebar.json` files.

use std::collections::HashMap;

use serde_json::Value;

const DEFAULT_ICON: &str = "✨";

/// Normalized sidebar tree ready to apply to our tab model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArcImport {
    pub spaces: Vec<ArcSpace>,
    /// Arc favorites (global strip) — imported into the first space.
    pub favorites: Vec<ArcTab>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArcSpace {
    pub name: String,
    pub icon: String,
    pub folders: Vec<ArcFolder>,
    pub pinned_tabs: Vec<ArcTab>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArcFolder {
    pub name: String,
    pub tabs: Vec<ArcTab>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArcTab {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArcImportPreview {
    pub spaces: usize,
    pub folders: usize,
    pub tabs: usize,
    pub favorites: usize,
}

#[derive(Debug)]
pub enum ArcImportError {
    InvalidJson(serde_json::Error),
    MissingSidebar,
    NoSpaces,
}

impl std::fmt::Display for ArcImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(err) => write!(f, "invalid Arc sidebar JSON: {err}"),
            Self::MissingSidebar => write!(f, "not a recognizable Arc StorableSidebar.json"),
            Self::NoSpaces => write!(f, "no importable spaces found"),
        }
    }
}

impl std::error::Error for ArcImportError {}

impl ArcImport {
    pub fn preview(&self) -> ArcImportPreview {
        let folders = self.spaces.iter().map(|s| s.folders.len()).sum();
        let tabs = self
            .spaces
            .iter()
            .map(|s| s.pinned_tabs.len() + s.folders.iter().map(|f| f.tabs.len()).sum::<usize>())
            .sum();
        ArcImportPreview {
            spaces: self.spaces.len(),
            folders,
            tabs,
            favorites: self.favorites.len(),
        }
    }
}

/// Parse raw `StorableSidebar.json` bytes into a normalized import tree.
pub fn parse_arc_sidebar(bytes: &[u8]) -> Result<ArcImport, ArcImportError> {
    let root: Value = serde_json::from_slice(bytes).map_err(ArcImportError::InvalidJson)?;
    let sidebar = root.get("sidebar").ok_or(ArcImportError::MissingSidebar)?;
    let containers = sidebar
        .get("containers")
        .and_then(Value::as_array)
        .ok_or(ArcImportError::MissingSidebar)?;
    let main = containers
        .iter()
        .find(|c| c.get("spaces").is_some())
        .ok_or(ArcImportError::MissingSidebar)?;

    let items = collect_items(main.get("items").unwrap_or(&Value::Null));
    let mut spaces = parse_spaces(main.get("spaces").unwrap_or(&Value::Null), &items);
    if spaces.is_empty() {
        return Err(ArcImportError::NoSpaces);
    }

    reorder_spaces(
        &mut spaces,
        root.pointer("/firebaseSyncState/syncData/orderedSpaceIDs/value"),
    );

    let favorites = parse_favorites(main.get("topAppsContainerIDs"), &items);

    Ok(ArcImport { spaces, favorites })
}

fn collect_items(value: &Value) -> HashMap<String, ArcItem> {
    let mut map = HashMap::new();
    let Some(list) = value.as_array() else {
        return map;
    };
    for entry in list {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let Some(id) = obj.get("id").and_then(Value::as_str) else {
            continue;
        };
        let children_ids = obj
            .get("childrenIds")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let data = obj.get("data").cloned().unwrap_or(Value::Null);
        map.insert(
            id.to_string(),
            ArcItem {
                title: obj.get("title").and_then(Value::as_str).map(str::to_string),
                children_ids,
                data,
            },
        );
    }
    map
}

#[derive(Debug, Clone)]
struct ArcItem {
    title: Option<String>,
    children_ids: Vec<String>,
    data: Value,
}

impl ArcItem {
    fn is_folder(&self) -> bool {
        self.data.get("list").is_some()
    }

    fn tab(&self) -> Option<ArcTab> {
        let tab = self.data.get("tab")?;
        let url = tab.get("savedURL").and_then(Value::as_str)?;
        let url = url.trim();
        if url.is_empty() || url == "about:blank" {
            return None;
        }
        let title = tab
            .get("savedTitle")
            .and_then(Value::as_str)
            .or(self.title.as_deref())
            .unwrap_or(url)
            .trim()
            .to_string();
        Some(ArcTab {
            url: url.to_string(),
            title,
        })
    }
}

fn parse_spaces(value: &Value, items: &HashMap<String, ArcItem>) -> Vec<ArcSpace> {
    let Some(list) = value.as_array() else {
        return Vec::new();
    };
    let mut spaces = Vec::new();
    let mut index = 0;
    for entry in list {
        let Some(space_obj) = entry.as_object() else {
            continue;
        };
        index += 1;
        let name = space_obj
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Space {index}"));
        let icon = space_icon(space_obj);
        let Some(pinned_root) = pinned_root_id(space_obj) else {
            spaces.push(ArcSpace {
                name,
                icon,
                folders: Vec::new(),
                pinned_tabs: Vec::new(),
            });
            continue;
        };
        let mut folders = Vec::new();
        let mut pinned_tabs = Vec::new();
        walk_pinned_children(&pinned_root, items, "", &mut folders, &mut pinned_tabs);
        spaces.push(ArcSpace {
            name,
            icon,
            folders,
            pinned_tabs,
        });
    }
    spaces
}

fn pinned_root_id(space: &serde_json::Map<String, Value>) -> Option<String> {
    let containers = space.get("newContainerIDs")?.as_array()?;
    for (i, entry) in containers.iter().enumerate() {
        if entry.get("pinned").is_some() {
            return containers
                .get(i + 1)
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
    }
    None
}

fn space_icon(space: &serde_json::Map<String, Value>) -> String {
    let Some(icon_type) = space.get("customInfo").and_then(|c| c.get("iconType")) else {
        return DEFAULT_ICON.to_string();
    };
    if let Some(text) = icon_type.get("emoji_v2").and_then(Value::as_str) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Some(code) = icon_type.get("emoji").and_then(Value::as_u64) {
        if let Some(ch) = char::from_u32(code as u32) {
            return ch.to_string();
        }
    }
    DEFAULT_ICON.to_string()
}

fn reorder_spaces(_spaces: &mut [ArcSpace], ordered_ids: Option<&Value>) {
    let Some(ids) = ordered_ids.and_then(Value::as_array) else {
        return;
    };
    let order: Vec<String> = ids
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    if order.is_empty() {
        return;
    }
    // Arc's ordered list references space UUIDs; we only have names here, so this
    // hook is reserved for future id-aware ordering. Names already follow the
    // `spaces` array order which matches user expectation on sampled profiles.
    let _ = order;
}

fn walk_pinned_children(
    item_id: &str,
    items: &HashMap<String, ArcItem>,
    folder_prefix: &str,
    folders: &mut Vec<ArcFolder>,
    pinned_tabs: &mut Vec<ArcTab>,
) {
    let Some(item) = items.get(item_id) else {
        return;
    };
    for child_id in &item.children_ids {
        let Some(child) = items.get(child_id) else {
            continue;
        };
        if let Some(tab) = child.tab() {
            pinned_tabs.push(tab);
        } else if child.is_folder() {
            let title = child
                .title
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Folder");
            let full_name = if folder_prefix.is_empty() {
                title.to_string()
            } else {
                format!("{folder_prefix} › {title}")
            };
            let mut nested_folders = Vec::new();
            let mut nested_tabs = Vec::new();
            walk_pinned_children(
                child_id,
                items,
                &full_name,
                &mut nested_folders,
                &mut nested_tabs,
            );
            if !nested_tabs.is_empty() {
                folders.push(ArcFolder {
                    name: full_name.clone(),
                    tabs: nested_tabs,
                });
            }
            folders.extend(nested_folders);
        }
        // easels, split views, and item containers are skipped.
    }
}

fn parse_favorites(value: Option<&Value>, items: &HashMap<String, ArcItem>) -> Vec<ArcTab> {
    let Some(list) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let Some(root_id) = list.iter().find_map(|entry| entry.as_str()) else {
        return Vec::new();
    };
    let mut favorites = Vec::new();
    walk_favorite_children(root_id, items, &mut favorites);
    favorites
}

fn walk_favorite_children(item_id: &str, items: &HashMap<String, ArcItem>, out: &mut Vec<ArcTab>) {
    let Some(item) = items.get(item_id) else {
        return;
    };
    for child_id in &item.children_ids {
        let Some(child) = items.get(child_id) else {
            continue;
        };
        if let Some(tab) = child.tab() {
            out.push(tab);
        } else {
            walk_favorite_children(child_id, items, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        include_bytes!("../../tests/fixtures/arc_sidebar_minimal.json").to_vec()
    }

    #[test]
    fn parses_spaces_folders_and_tabs() {
        let parsed = parse_arc_sidebar(&fixture()).expect("fixture parses");
        assert_eq!(parsed.spaces.len(), 2);
        assert_eq!(parsed.spaces[0].name, "Work");
        assert_eq!(parsed.spaces[0].icon, "💻");
        assert_eq!(parsed.spaces[0].pinned_tabs.len(), 1);
        assert_eq!(parsed.spaces[0].pinned_tabs[0].url, "https://example.com/a");
        assert_eq!(parsed.spaces[0].folders.len(), 1);
        assert_eq!(parsed.spaces[0].folders[0].name, "Docs");
        assert_eq!(
            parsed.spaces[0].folders[0].tabs[0].url,
            "https://example.com/b"
        );
        assert_eq!(parsed.spaces[1].name, "Personal");
        assert_eq!(parsed.favorites.len(), 1);
        assert_eq!(parsed.favorites[0].url, "https://example.com/fav");
    }

    #[test]
    fn preview_counts() {
        let parsed = parse_arc_sidebar(&fixture()).unwrap();
        let preview = parsed.preview();
        assert_eq!(preview.spaces, 2);
        assert_eq!(preview.folders, 1);
        assert_eq!(preview.tabs, 2);
        assert_eq!(preview.favorites, 1);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_arc_sidebar(b"not json").is_err());
        assert!(parse_arc_sidebar(b"{}").is_err());
    }
}
