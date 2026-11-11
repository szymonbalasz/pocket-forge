//! The HTTP seam (spec §10): [`Transport`] sends one request, [`Sleeper`] waits. The Scenario
//! provider gets both injected, so tests run over [`crate::replay::ReplayTransport`] with a
//! recording sleeper and never touch the network (AGENTS §3.7).
//!
//! [`HttpTransport`] is the real one: `reqwest` with rustls on the provider's own tokio runtime.
//! Its futures are plain join handles, so they can be awaited from any executor (including
//! [`crate::block_on`] inside a Tauri command).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::secret::Secret;

/// A boxed, sendable future.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// HTTP methods the adapter uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Put,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
        }
    }
}

/// One request. The auth header is kept apart from `headers` as a [`Secret`] so it can't be
/// logged or recorded by accident (FR-PRV-08); only the provider decides when it is set
/// (API host only, FR-PRV-09).
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub authorization: Option<Secret>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &log_url(&self.url))
            .field("authorized", &self.authorization.is_some())
            .field("body_len", &self.body.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

/// A URL safe to log: scheme, host and path, never the query (signed URLs carry tokens there).
pub fn log_url(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or("")
}

/// The host of an absolute URL (lower case, without port or user info).
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// One response: status, a header subset and the body bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    /// Lower-case names. Only `content-type`, `retry-after` and `etag` are kept.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Headers kept from a response.
pub const KEPT_HEADERS: [&str; 3] = ["content-type", "retry-after", "etag"];

/// Transport failure before a response arrived.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// Nothing was sent: connect, DNS or setup failure.
    #[error("the provider can't be reached ({0})")]
    Offline(String),
    #[error("the request timed out")]
    Timeout,
    /// The connection broke after the request went out (reset, body read error): the provider
    /// may have received and acted on it (R102).
    #[error("the connection broke after the request was sent")]
    Interrupted,
}

/// Sends one HTTP request.
pub trait Transport: Send + Sync + fmt::Debug {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>>;
}

/// Waits (retry backoff). Tests record the delays instead of sleeping.
pub trait Sleeper: Send + Sync + fmt::Debug {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()>;
}

// ---------------------------------------------------------------- test seams

/// A transport that fails the test if anything tries to use it (FR-PRV-05, FR-PRV-13).
#[derive(Debug, Default)]
pub struct PanicTransport;

impl Transport for PanicTransport {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        panic!("no network call allowed here, but got {request:?}")
    }
}

/// Records the requested delays and returns at once.
#[derive(Debug, Default)]
pub struct RecordingSleeper(pub Mutex<Vec<Duration>>);

impl RecordingSleeper {
    pub fn delays(&self) -> Vec<Duration> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Sleeper for RecordingSleeper {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(duration);
        Box::pin(std::future::ready(()))
    }
}

// ---------------------------------------------------------------- real transport

/// The provider's own I/O runtime. Created on first real use and never dropped, so it can't
/// panic by being dropped inside another runtime.
fn runtime() -> Result<&'static tokio::runtime::Runtime, TransportError> {
    static RT: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pf-provider-io")
            .enable_all()
            .build()
            .ok()
    })
    .as_ref()
    .ok_or_else(|| TransportError::Offline("the network runtime could not start".into()))
}

/// The real HTTP transport (`reqwest`, rustls).
#[derive(Debug, Default)]
pub struct HttpTransport {
    client: OnceLock<Option<reqwest::Client>>,
}

impl HttpTransport {
    fn client(&self) -> Result<reqwest::Client, TransportError> {
        self.client
            .get_or_init(|| {
                reqwest::Client::builder()
                    .user_agent(concat!("pocket-forge/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .ok()
            })
            .clone()
            .ok_or_else(|| TransportError::Offline("the HTTP client could not start".into()))
    }
}

fn map_reqwest(e: &reqwest::Error) -> TransportError {
    // Never the error's Display: it names the full URL, and signed URLs carry tokens.
    if e.is_timeout() {
        TransportError::Timeout
    } else if e.is_connect() {
        TransportError::Offline("connection failed".into())
    } else if e.is_builder() {
        TransportError::Offline("the request could not be built".into())
    } else {
        // Anything after the connection opened may have reached the provider (R102).
        TransportError::Interrupted
    }
}

impl Transport for HttpTransport {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        let prepared = (|| {
            let client = self.client()?;
            let rt = runtime()?;
            let method = match request.method {
                Method::Get => reqwest::Method::GET,
                Method::Post => reqwest::Method::POST,
                Method::Put => reqwest::Method::PUT,
            };
            let mut rb = client
                .request(method, &request.url)
                .timeout(request.timeout);
            for (k, v) in &request.headers {
                rb = rb.header(k.as_str(), v.as_str());
            }
            if let Some(auth) = &request.authorization {
                let mut value = reqwest::header::HeaderValue::from_str(auth.expose())
                    .map_err(|_| TransportError::Offline("bad credentials encoding".into()))?;
                value.set_sensitive(true);
                rb = rb.header(reqwest::header::AUTHORIZATION, value);
            }
            if let Some(body) = request.body {
                rb = rb.body(body);
            }
            Ok(rt.spawn(async move {
                let resp = rb.send().await.map_err(|e| map_reqwest(&e))?;
                let status = resp.status().as_u16();
                let headers = KEPT_HEADERS
                    .iter()
                    .filter_map(|name| {
                        resp.headers()
                            .get(*name)
                            .and_then(|v| v.to_str().ok())
                            .map(|v| ((*name).to_owned(), v.to_owned()))
                    })
                    .collect();
                let body = resp.bytes().await.map_err(|e| map_reqwest(&e))?.to_vec();
                Ok(HttpResponse {
                    status,
                    headers,
                    body,
                })
            }))
        })();
        Box::pin(async move {
            match prepared {
                Ok(handle) => handle.await.unwrap_or(Err(TransportError::Interrupted)),
                Err(e) => Err(e),
            }
        })
    }
}

/// Real sleeps on the provider runtime's timer.
#[derive(Debug, Default)]
pub struct RuntimeSleeper;

impl Sleeper for RuntimeSleeper {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        match runtime() {
            Ok(rt) => {
                // Built inside the task: `tokio::time::sleep` needs the runtime's context,
                // which the caller's thread (`block_on`) doesn't have.
                let handle = rt.spawn(async move { tokio::time::sleep(duration).await });
                Box::pin(async move {
                    let _ = handle.await;
                })
            }
            Err(_) => Box::pin(std::future::ready(())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The paid recorder polls from a plain thread (`block_on`): a real sleep there must not
    /// need the caller to be inside the runtime (it panicked after a paid submit).
    #[test]
    fn fr_prv_19_runtime_sleeper_works_outside_a_runtime() {
        let t = std::time::Instant::now();
        crate::block_on(RuntimeSleeper.sleep(Duration::from_millis(20)));
        assert!(t.elapsed() >= Duration::from_millis(15));
    }

    #[test]
    fn hosts_and_log_urls() {
        assert_eq!(
            host_of("https://API.cloud.scenario.com/v1/x").as_deref(),
            Some("api.cloud.scenario.com")
        );
        assert_eq!(
            host_of("https://u:p@example.com:443/a?b").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_of("fake://asset/1").as_deref(), Some("asset"));
        assert_eq!(host_of("no-scheme"), None);
        assert_eq!(
            log_url("https://cdn.example.com/a.png?X-Amz-Signature=abc"),
            "https://cdn.example.com/a.png"
        );
    }

    #[test]
    fn fr_prv_08_request_debug_never_shows_the_auth_header_or_query() {
        let req = HttpRequest {
            method: Method::Get,
            url: "https://h/x?token=t0k3n".into(),
            headers: vec![],
            authorization: Some(Secret::new("Basic c2VjcmV0")),
            body: None,
            timeout: Duration::from_secs(1),
        };
        let shown = format!("{req:?}");
        assert!(
            !shown.contains("c2VjcmV0") && !shown.contains("t0k3n"),
            "{shown}"
        );
        assert!(shown.contains("authorized: true"));
    }
}
