//! Apply a parsed import tree to the live tab store.

use crate::import::arc::ArcImport;
use crate::tabs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    pub spaces: usize,
    pub folders: usize,
    pub tabs: usize,
    pub favorites: usize,
    pub skipped_duplicates: usize,
    /// History pages merged from Arc's `History` file (new or changed).
    pub history: usize,
}

#[derive(Debug)]
pub enum ApplyError {
    StoreUnavailable,
    NothingToImport,
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StoreUnavailable => write!(f, "tab store not initialized"),
            Self::NothingToImport => write!(f, "nothing to import"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// Merge `import` into the tab model. `replace` clears the existing session first.
pub fn apply_arc_import(import: &ArcImport, replace: bool) -> Result<ApplyResult, ApplyError> {
    let preview = import.preview();
    if preview.spaces == 0 && preview.tabs == 0 && preview.favorites == 0 {
        return Err(ApplyError::NothingToImport);
    }
    tabs::apply_arc_import(import, replace).ok_or(ApplyError::StoreUnavailable)
}
