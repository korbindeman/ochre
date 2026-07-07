//! HTTP transport seam for the vault client (ADR-0016).
//!
//! The vault's auth (and later sync) code is pure request-building + response-
//! parsing; the actual network send is isolated behind [`HttpTransport`] so the
//! flow logic is unit-testable against a mock and the real `ureq` call lives in
//! exactly one place ([`UreqTransport`]). The client is **blocking** — vault
//! calls run off the UI thread; there is no async runtime.

/// HTTP method — GET/POST plus PUT/DELETE for single-cipher edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

/// A request to hand to a [`HttpTransport`].
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// A GET with the given headers.
    pub fn get(url: String, headers: Vec<(String, String)>) -> Self {
        Self {
            method: Method::Get,
            url,
            headers,
            body: Vec::new(),
        }
    }

    /// A POST with the given headers and body.
    pub fn post(url: String, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        Self {
            method: Method::Post,
            url,
            headers,
            body,
        }
    }

    /// A PUT with the given headers and body.
    pub fn put(url: String, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        Self {
            method: Method::Put,
            url,
            headers,
            body,
        }
    }

    /// A DELETE with the given headers.
    pub fn delete(url: String, headers: Vec<(String, String)>) -> Self {
        Self {
            method: Method::Delete,
            url,
            headers,
            body: Vec::new(),
        }
    }
}

/// A response from a [`HttpTransport`]. Non-2xx statuses are returned normally
/// (not as errors) so the flow can inspect 4xx bodies — a Bitwarden 2FA
/// challenge arrives as HTTP 400 with a body we must parse.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// A transport-level failure (DNS, TLS, connection, body read) — distinct from
/// an HTTP error status, which is a normal [`HttpResponse`].
#[derive(Debug)]
pub struct TransportError(pub String);

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transport error: {}", self.0)
    }
}

impl std::error::Error for TransportError {}

/// The seam. Real network access is [`UreqTransport`]; tests supply a mock.
pub trait HttpTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// The production transport: a blocking `ureq` agent with rustls TLS, configured
/// to surface 4xx/5xx as responses rather than errors.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for UreqTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let result = match request.method {
            Method::Get => {
                let mut builder = self.agent.get(&request.url);
                for (k, v) in &request.headers {
                    builder = builder.header(k.as_str(), v.as_str());
                }
                builder.call()
            }
            Method::Post => {
                let mut builder = self.agent.post(&request.url);
                for (k, v) in &request.headers {
                    builder = builder.header(k.as_str(), v.as_str());
                }
                builder.send(&request.body[..])
            }
            Method::Put => {
                let mut builder = self.agent.put(&request.url);
                for (k, v) in &request.headers {
                    builder = builder.header(k.as_str(), v.as_str());
                }
                builder.send(&request.body[..])
            }
            Method::Delete => {
                let mut builder = self.agent.delete(&request.url);
                for (k, v) in &request.headers {
                    builder = builder.header(k.as_str(), v.as_str());
                }
                builder.call()
            }
        };
        let mut response = result.map_err(|e| TransportError(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_vec()
            .map_err(|e| TransportError(e.to_string()))?;
        Ok(HttpResponse { status, body })
    }
}
