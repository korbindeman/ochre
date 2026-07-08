//! Native-styled navigation error pages for the ALLOY runtime (style §10).
//!
//! CEF's embedded ALLOY path does not render Chromium's built-in net-error
//! interstitials, so failed loads must supply their own HTML. The handler must
//! **not** hold [`super::BrowserHandler`]'s mutex across `frame.load_url` —
//! that re-enters display/load callbacks and deadlocks the UI thread.

use cef::{base64_encode, sys, uriencode, CefString, Errorcode, Frame, ImplFrame, RuntimeStyle};

/// Show a styled error page for a failed main-frame navigation.
pub fn show(
    runtime_style: RuntimeStyle,
    frame: Option<&mut Frame>,
    error_code: Errorcode,
    error_text: Option<&CefString>,
    failed_url: Option<&CefString>,
) {
    if runtime_style != RuntimeStyle::ALLOY {
        return;
    }

    let Some(frame) = frame else {
        return;
    };
    if frame.is_main() == 0 {
        return;
    }

    let code = sys::cef_errorcode_t::from(error_code);
    if code == sys::cef_errorcode_t::ERR_ABORTED {
        return;
    }

    let error_text = error_text.map(CefString::to_string).unwrap_or_default();
    let failed_url = failed_url.map(CefString::to_string).unwrap_or_default();
    let html = render(code, &error_text, &failed_url);
    let uri = data_uri(html.as_bytes(), "text/html");
    let uri = CefString::from(uri.as_str());
    frame.load_url(Some(&uri));
}

/// Build the error-page HTML for `code` / `failed_url`.
pub(crate) fn render(code: sys::cef_errorcode_t, error_text: &str, failed_url: &str) -> String {
    let (heading, summary, hints) = copy_for(code, error_text, failed_url);
    let heading = escape_html(&heading);
    let summary = escape_html(&summary);
    let failed_url = escape_html(failed_url);
    let hints_html: String = hints
        .iter()
        .map(|h| format!("<li>{}</li>", escape_html(h)))
        .collect();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>{heading}</title>
<style>
  :root {{
    color-scheme: light dark;
    --bg: #f5f5f7;
    --fg: #1d1d1f;
    --secondary: rgba(60, 60, 67, 0.6);
    --url-bg: rgba(0, 0, 0, 0.04);
    --icon: #ff9f0a;
  }}
  @media (prefers-color-scheme: dark) {{
    :root {{
      --bg: #1c1c1e;
      --fg: #f5f5f7;
      --secondary: rgba(235, 235, 245, 0.6);
      --url-bg: rgba(255, 255, 255, 0.08);
      --icon: #ffd60a;
    }}
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 48px 24px;
    background: var(--bg);
    color: var(--fg);
    font: 15px/1.5 -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif;
    -webkit-font-smoothing: antialiased;
  }}
  main {{
    max-width: 520px;
    width: 100%;
    text-align: center;
  }}
  .icon {{
    width: 56px;
    height: 56px;
    margin: 0 auto 20px;
    color: var(--icon);
  }}
  h1 {{
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.02em;
    margin-bottom: 8px;
  }}
  p {{
    color: var(--secondary);
    margin-bottom: 16px;
  }}
  .url {{
    display: inline-block;
    max-width: 100%;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--url-bg);
    font: 13px/1.4 ui-monospace, "SF Mono", Menlo, monospace;
    word-break: break-all;
    margin-bottom: 20px;
  }}
  ul {{
    list-style: none;
    text-align: left;
    color: var(--secondary);
    font-size: 14px;
  }}
  li {{
    padding: 4px 0 4px 18px;
    position: relative;
  }}
  li::before {{
    content: "•";
    position: absolute;
    left: 0;
    color: var(--secondary);
  }}
  .reload {{
    margin-top: 24px;
    font-size: 13px;
    color: var(--secondary);
  }}
  kbd {{
    font: inherit;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--url-bg);
  }}
</style>
</head>
<body>
<main>
  <svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="1.5"/>
    <path d="M12 7v6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
    <circle cx="12" cy="16.5" r="0.9" fill="currentColor"/>
  </svg>
  <h1>{heading}</h1>
  <p>{summary}</p>
  <div class="url">{failed_url}</div>
  <ul>{hints_html}</ul>
  <p class="reload">Press <kbd>⌘R</kbd> to try again.</p>
</main>
</body>
</html>"#
    )
}

fn copy_for(
    code: sys::cef_errorcode_t,
    error_text: &str,
    failed_url: &str,
) -> (String, String, Vec<String>) {
    let host = host_label(failed_url);
    match code {
        sys::cef_errorcode_t::ERR_CONNECTION_REFUSED => (
            format!("{host} refused to connect"),
            "Nothing is listening on this address and port.".into(),
            vec![
                "Check that your local server is running.".into(),
                "Verify the port number in the address.".into(),
            ],
        ),
        sys::cef_errorcode_t::ERR_CONNECTION_TIMED_OUT | sys::cef_errorcode_t::ERR_TIMED_OUT => (
            format!("{host} took too long to respond"),
            "The server didn't answer in time.".into(),
            vec![
                "Check your network connection.".into(),
                "The server may be overloaded or offline.".into(),
            ],
        ),
        sys::cef_errorcode_t::ERR_NAME_NOT_RESOLVED => (
            format!("{host} couldn't be found"),
            "The address doesn't resolve to a server.".into(),
            vec![
                "Check for typos in the hostname.".into(),
                "Your DNS or network connection may be down.".into(),
            ],
        ),
        sys::cef_errorcode_t::ERR_ADDRESS_UNREACHABLE
        | sys::cef_errorcode_t::ERR_CONNECTION_FAILED => (
            format!("{host} is unreachable"),
            "The browser couldn't establish a connection.".into(),
            vec![
                "Check your network connection.".into(),
                "The server may be down or blocking requests.".into(),
            ],
        ),
        sys::cef_errorcode_t::ERR_INTERNET_DISCONNECTED => (
            "You're offline".into(),
            "This page can't load without a network connection.".into(),
            vec!["Check your Wi‑Fi or Ethernet connection.".into()],
        ),
        sys::cef_errorcode_t::ERR_CONNECTION_RESET
        | sys::cef_errorcode_t::ERR_CONNECTION_CLOSED => (
            format!("Connection to {host} was interrupted"),
            "The server closed the connection unexpectedly.".into(),
            vec!["Try reloading the page.".into()],
        ),
        c if is_cert_error(c) => (
            format!("{host} connection isn't private"),
            "The server's security certificate couldn't be verified.".into(),
            vec![
                "Check that the site's certificate is valid.".into(),
                "Only proceed if you trust this server.".into(),
            ],
        ),
        _ => {
            let detail = if error_text.is_empty() {
                "The page couldn't be loaded.".into()
            } else {
                error_text.to_string()
            };
            (
                format!("{host} is unavailable"),
                detail,
                vec!["Try reloading the page.".into()],
            )
        }
    }
}

fn is_cert_error(code: sys::cef_errorcode_t) -> bool {
    matches!(
        code,
        sys::cef_errorcode_t::ERR_SSL_PROTOCOL_ERROR
            | sys::cef_errorcode_t::ERR_SSL_VERSION_OR_CIPHER_MISMATCH
            | sys::cef_errorcode_t::ERR_SSL_CLIENT_AUTH_CERT_NEEDED
            | sys::cef_errorcode_t::ERR_CERT_COMMON_NAME_INVALID
            | sys::cef_errorcode_t::ERR_CERT_DATE_INVALID
            | sys::cef_errorcode_t::ERR_CERT_AUTHORITY_INVALID
            | sys::cef_errorcode_t::ERR_CERT_INVALID
            | sys::cef_errorcode_t::ERR_CERT_REVOKED
    )
}

fn host_label(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return "This site".into();
    }
    // Cheap host extraction — good enough for error copy.
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    if host_port.is_empty() {
        "This site".into()
    } else {
        host_port.to_string()
    }
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn data_uri(data: &[u8], mime_type: &str) -> String {
    let data = CefString::from(&base64_encode(Some(data)));
    let uri = CefString::from(&uriencode(Some(&data), 0)).to_string();
    format!("data:{mime_type};base64,{uri}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cef::sys::cef_errorcode_t;

    #[test]
    fn connection_refused_copy_mentions_localhost() {
        let html = render(
            cef_errorcode_t::ERR_CONNECTION_REFUSED,
            "Connection refused",
            "http://localhost:4242/",
        );
        assert!(html.contains("localhost:4242 refused to connect"));
        assert!(html.contains("local server is running"));
        assert!(!html.contains("Connection refused")); // user-facing, not raw CEF text
    }

    #[test]
    fn html_escapes_url() {
        let html = render(
            cef_errorcode_t::ERR_FAILED,
            "fail",
            "http://example.com/?a=1&b=2",
        );
        assert!(html.contains("http://example.com/?a=1&amp;b=2"));
    }

    #[test]
    fn prefers_color_scheme_meta_present() {
        let html = render(cef_errorcode_t::ERR_TIMED_OUT, "", "http://example.com");
        assert!(html.contains(r#"<meta name="color-scheme" content="light dark">"#));
    }
}
