//! BrowserApp - the CEF App trait implementation.
//!
//! This wires up the browser process handler that fires when CEF's context
//! is initialized, creating the first browser instance.

use cef::*;
use std::cell::RefCell;

use crate::client::{BrowserBrowserViewDelegate, BrowserClient, BrowserWindowDelegate};

/// Chromium stores cookie/password encryption keys in the macOS login keychain
/// ("Chromium Safe Storage"). Unsigned or ad-hoc-signed builds get a new
/// signature on every rebuild, so the keychain ACL no longer matches and macOS
/// prompts for your login password - once per CEF subprocess (GPU, renderer, …).
///
/// Until a Developer-ID-signed build ships (backlog P0-2), use Chromium's
/// in-memory mock keychain so dev/dogfood launches stay silent. Enable the
/// `production-keychain` feature only for signed release CI where passkeys and
/// persistent cookie encryption need the real keychain.
pub(crate) fn append_mock_keychain_switch(command_line: &mut CommandLine) {
    #[cfg(not(feature = "production-keychain"))]
    command_line.append_switch(Some(&CefString::from("use-mock-keychain")));
}

/// Blink `WebAuthentication` stays off until this *browser process* binary
/// carries Apple's web-browser public-key-credential entitlement (ADR-0061,
/// CRASH-0003). Helpers inherit the browser process command line; they must
/// not re-decide from their own (unentitled) helper signature.
const WEB_AUTHN_GUARD_BLINK_FEATURES: &str = "WebAuthentication";

pub(crate) fn append_webauthn_guard_switch(
    command_line: &mut CommandLine,
    is_browser_process: bool,
) {
    if !is_browser_process {
        return;
    }
    if crate::mac::has_web_browser_passkey_entitlement() {
        return;
    }
    command_line.append_switch_with_value(
        Some(&CefString::from("disable-blink-features")),
        Some(&CefString::from(WEB_AUTHN_GUARD_BLINK_FEATURES)),
    );
}

/// Chromium's Media Router (Google Cast) is on by default, but Ochre has no
/// cast-device picker, so any device discovery it ran would power nothing the
/// user can reach. Verified inert under ALLOY on CEF 148 (getAvailability
/// resolves false, no mDNS/DIAL sockets open) - this switch encodes the intent
/// and guards against a future CEF bump silently activating discovery, which
/// would trip the macOS 15+ local-network permission prompt. Note: the prompt
/// users actually see today comes from WebRTC's mDNS local-IP obfuscation,
/// not from cast discovery - see ADR-0050. Re-enable only as part of a full
/// Cast slice (backlog P2-44).
pub(crate) fn append_disable_cast_discovery_switch(command_line: &mut CommandLine) {
    command_line.append_switch_with_value(
        Some(&CefString::from("disable-features")),
        Some(&CefString::from("MediaRouter,DialMediaRouteProvider")),
    );
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
            process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(command_line) = command_line {
                let is_browser_process = process_type
                    .map(|t| t.to_string().is_empty())
                    .unwrap_or(true);
                append_mock_keychain_switch(command_line);
                append_webauthn_guard_switch(command_line, is_browser_process);
                append_disable_cast_discovery_switch(command_line);
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
                let window_info =
                    window_info.set_as_popup(Default::default(), crate::product::NAME);

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

#[cfg(test)]
mod tests {
    #[test]
    fn webauthn_guard_disables_blink_webauthentication() {
        assert!(super::WEB_AUTHN_GUARD_BLINK_FEATURES
            .split(',')
            .any(|f| f == "WebAuthentication"));
    }
}
