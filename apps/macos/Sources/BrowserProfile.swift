import Foundation

/// Build-profile identity - must stay in sync with Rust `profile_data_dir_name`.
///
/// Debug (`just run`) and release (`just dogfood`) keep separate persisted state:
/// the core pins CEF + all JSON/SQLite under `~/Library/Application Support/`
/// `{name} Dev` vs `{name}` from `product.toml`; Swift chrome prefs use the same `.dev` suffix here.
enum BrowserProfile {
    #if DEBUG
    static let defaultsKeySuffix = ".dev"
    #else
    static let defaultsKeySuffix = ""
    #endif

    static func defaultsKey(_ base: String) -> String {
        base + defaultsKeySuffix
    }
}
