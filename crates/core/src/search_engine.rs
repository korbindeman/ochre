//! Default web search engine - persisted preference and query URL templates.
//!
//! The core owns which engine is selected (ADR-0010); native hosts render the
//! settings picker and suggestion fetches, keyed off the FFI wire value.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// A curated set of common search engines, including privacy-oriented options.
/// Kept sparse - no custom engines or instance URLs in this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchEngine {
    #[default]
    Google,
    DuckDuckGo,
    Bing,
    Brave,
    Startpage,
    Ecosia,
    Kagi,
}

impl SearchEngine {
    /// FFI wire value (`browser_settings_search_engine_*`): 0 Google … 6 Kagi.
    pub fn to_i32(self) -> i32 {
        match self {
            SearchEngine::Google => 0,
            SearchEngine::DuckDuckGo => 1,
            SearchEngine::Bing => 2,
            SearchEngine::Brave => 3,
            SearchEngine::Startpage => 4,
            SearchEngine::Ecosia => 5,
            SearchEngine::Kagi => 6,
        }
    }

    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => SearchEngine::DuckDuckGo,
            2 => SearchEngine::Bing,
            3 => SearchEngine::Brave,
            4 => SearchEngine::Startpage,
            5 => SearchEngine::Ecosia,
            6 => SearchEngine::Kagi,
            _ => SearchEngine::Google,
        }
    }

    /// Human-readable label for settings UI and omnibox copy.
    pub fn display_name(self) -> &'static str {
        match self {
            SearchEngine::Google => "Google",
            SearchEngine::DuckDuckGo => "DuckDuckGo",
            SearchEngine::Bing => "Bing",
            SearchEngine::Brave => "Brave Search",
            SearchEngine::Startpage => "Startpage",
            SearchEngine::Ecosia => "Ecosia",
            SearchEngine::Kagi => "Kagi",
        }
    }

    /// Build the search results URL for `query` using the selected engine template.
    pub fn search_url(self, query: &str) -> String {
        let q = encode_query_component(query.trim());
        match self {
            SearchEngine::Google => format!("https://www.google.com/search?q={q}"),
            SearchEngine::DuckDuckGo => format!("https://duckduckgo.com/?q={q}"),
            SearchEngine::Bing => format!("https://www.bing.com/search?q={q}"),
            SearchEngine::Brave => format!("https://search.brave.com/search?q={q}"),
            SearchEngine::Startpage => format!("https://www.startpage.com/sp/search?q={q}"),
            SearchEngine::Ecosia => format!("https://www.ecosia.org/search?q={q}"),
            SearchEngine::Kagi => format!("https://kagi.com/search?q={q}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct SearchEngineState {
    #[serde(default)]
    engine: SearchEngine,
}

impl Default for SearchEngineState {
    fn default() -> Self {
        Self {
            engine: SearchEngine::Google,
        }
    }
}

static STATE: Mutex<SearchEngineState> = Mutex::new(SearchEngineState {
    engine: SearchEngine::Google,
});

pub fn engine() -> SearchEngine {
    STATE.lock().unwrap_or_else(|p| p.into_inner()).engine
}

pub fn set_engine(engine: SearchEngine) {
    {
        let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
        if state.engine == engine {
            return;
        }
        state.engine = engine;
    }
    save();
}

pub fn search_url(query: &str) -> String {
    engine().search_url(query)
}

pub fn load_persisted() {
    if let Some(state) = load_from(&state_path()) {
        *STATE.lock().unwrap_or_else(|p| p.into_inner()) = state;
    }
}

fn encode_query_component(query: &str) -> String {
    let mut encoded = String::with_capacity(query.len());
    for ch in query.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => encoded.push(ch),
            _ => {
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).bytes() {
                    encoded.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    encoded
}

fn state_path() -> String {
    format!("{}/search_engine.json", crate::root_cache_dir())
}

fn load_from(path: &str) -> Option<SearchEngineState> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(state) => Some(state),
        Err(err) => {
            eprintln!("[search_engine] failed to parse {path}: {err}");
            None
        }
    }
}

fn save() {
    let snapshot = *STATE.lock().unwrap_or_else(|p| p.into_inner());
    let path = state_path();
    let Ok(json) = serde_json::to_vec_pretty(&snapshot) else {
        return;
    };
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, &json).and_then(|()| std::fs::rename(&tmp, &path)) {
        eprintln!("[search_engine] failed to write {path}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_value_round_trips() {
        for engine in [
            SearchEngine::Google,
            SearchEngine::DuckDuckGo,
            SearchEngine::Bing,
            SearchEngine::Brave,
            SearchEngine::Startpage,
            SearchEngine::Ecosia,
            SearchEngine::Kagi,
        ] {
            assert_eq!(SearchEngine::from_i32(engine.to_i32()), engine);
        }
        assert_eq!(SearchEngine::from_i32(99), SearchEngine::Google);
    }

    #[test]
    fn json_round_trips() {
        let state = SearchEngineState {
            engine: SearchEngine::DuckDuckGo,
        };
        let back: SearchEngineState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(state, back);
    }

    #[test]
    fn builds_search_urls() {
        assert_eq!(
            SearchEngine::Google.search_url("hello world"),
            "https://www.google.com/search?q=hello%20world"
        );
        assert_eq!(
            SearchEngine::DuckDuckGo.search_url("rust+lang"),
            "https://duckduckgo.com/?q=rust%2Blang"
        );
        assert_eq!(
            SearchEngine::Brave.search_url("test"),
            "https://search.brave.com/search?q=test"
        );
    }
}
