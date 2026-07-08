//! BrowserApp — the CEF App trait implementation.
//!
//! This wires up the browser process handler that fires when CEF's context
//! is initialized, creating the first browser instance.

use cef::*;
use std::cell::RefCell;

use crate::client::{BrowserBrowserViewDelegate, BrowserClient, BrowserWindowDelegate};

/// Chromium stores cookie/password encryption keys in the macOS login keychain
/// ("Chromium Safe Storage"). Unsigned or ad-hoc-signed builds get a new
/// signature on every rebuild, so the keychain ACL no longer matches and macOS
/// prompts for your login password — once per CEF subprocess (GPU, renderer, …).
///
/// Until a Developer-ID-signed build ships (backlog P0-2), use Chromium's
/// in-memory mock keychain so dev/dogfood launches stay silent. Enable the
/// `production-keychain` feature only for signed release CI where passkeys and
/// persistent cookie encryption need the real keychain.
pub(crate) fn append_mock_keychain_switch(command_line: &mut CommandLine) {
    #[cfg(not(feature = "production-keychain"))]
    command_line.append_switch(Some(&CefString::from("use-mock-keychain")));
}

wrap_app! {
    pub struct BrowserApp;

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(BrowserBrowserProcessHandler::new(RefCell::new(None)))
        }

        /// Runs for every process type (browser + helpers) so each subprocess
        /// gets the mock-keychain switch even when launched via `execute_process`.
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(command_line) = command_line {
                append_mock_keychain_switch(command_line);
            }
        }
    }
}

wrap_browser_process_handler! {
    struct BrowserBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);

            // Determine runtime style from command line.
            let command_line = command_line_get_global().expect("Failed to get command line");
            let use_alloy_style =
                command_line.has_switch(Some(&CefString::from("use-alloy-style"))) != 0;
            let runtime_style = if use_alloy_style {
                RuntimeStyle::ALLOY
            } else {
                RuntimeStyle::DEFAULT
            };

            // Create the client (handlers for display, life span, load, etc.).
            {
                let mut client = self.client.borrow_mut();
                *client = Some(BrowserClient::create(runtime_style));
            }

            // Browser settings.
            let settings = BrowserSettings::default();

            // URL from --url= or default.
            let url = CefString::from(&command_line.switch_value(Some(&CefString::from("url"))))
                .to_string();
            let url = if url.is_empty() {
                "https://www.google.com/"
            } else {
                url.as_str()
            };
            let url = CefString::from(url);

            // Use the Views framework by default (native window with browser view).
            // Add --use-native to use the older native popup path instead.
            let use_views = command_line.has_switch(Some(&CefString::from("use-native"))) != 0;

            if use_views {
                // Create the BrowserView.
                let mut client = self.default_client();
                let mut delegate = BrowserBrowserViewDelegate::new(runtime_style);
                let browser_view = browser_view_create(
                    client.as_mut(),
                    Some(&url),
                    Some(&settings),
                    None,
                    None,
                    Some(&mut delegate),
                );

                // Create the top-level Window. It will show itself after creation.
                let mut delegate = BrowserWindowDelegate::new(
                    RefCell::new(browser_view),
                    runtime_style,
                    ShowState::NORMAL,
                );
                window_create_top_level(Some(&mut delegate));
            } else {
                // Native popup path.
                let window_info = WindowInfo {
                    runtime_style,
                    ..Default::default()
                };

                #[cfg(target_os = "windows")]
                let window_info = window_info.set_as_popup(Default::default(), "Browser");

                let mut client = self.default_client();
                browser_host_create_browser(
                    Some(&window_info),
                    client.as_mut(),
                    Some(&url),
                    Some(&settings),
                    None,
                    None,
                );
            }
        }

        fn default_client(&self) -> Option<Client> {
            self.client.borrow().clone()
        }
    }
}
