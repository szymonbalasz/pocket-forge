//! The Scenario request layer: auth (API host only, FR-PRV-09), the error map (Appendix C,
//! FR-PRV-23) and the retry policy for idempotent calls (FR-PRV-24).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::secret::Credentials;
use crate::transport::{
    HttpRequest, HttpResponse, Method, Sleeper, Transport, TransportError, host_of, log_url,
};
use crate::{Connectivity, ConnectivityCell, ProviderError, ProviderResult};

/// Scenario's API base (research doc §Capability matrix).
pub const DEFAULT_BASE_URL: &str = "https://api.cloud.scenario.com/v1";

/// Timeouts (FR-PRV-24).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
pub const UPLOAD_PART_TIMEOUT: Duration = Duration::from_secs(120);

/// Retry: at most 3 attempts, waiting 1 then 2 s (the schedule continues 4 s), at least
/// `Retry-After` capped at 30 s (FR-PRV-24).
pub const MAX_ATTEMPTS: usize = 3;
pub const BACKOFF_SECS: [u64; 3] = [1, 2, 4];
pub const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);

/// Whether a call may be repeated. Submit is never retried once bytes were sent (FR-PRV-17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    Idempotent,
    Once,
}

/// The HTTP client for one Scenario account.
#[derive(Debug, Clone)]
pub struct Api {
    pub base: String,
    host: Option<String>,
    pub transport: Arc<dyn Transport>,
    pub sleeper: Arc<dyn Sleeper>,
    /// Updated from every final outcome: auth on the API host, offline, or reachable.
    connectivity: Option<Arc<ConnectivityCell>>,
}

/// A body to send.
pub enum Body {
    None,
    Json(Value),
    Bytes {
        bytes: Vec<u8>,
        content_type: String,
    },
}

impl Api {
    pub fn new(base: &str, transport: Arc<dyn Transport>, sleeper: Arc<dyn Sleeper>) -> Self {
        let base = base.trim_end_matches('/').to_owned();
        Api {
            host: host_of(&base),
            base,
            transport,
            sleeper,
            connectivity: None,
        }
    }

    pub fn with_connectivity(mut self, cell: Arc<ConnectivityCell>) -> Self {
        self.connectivity = Some(cell);
        self
    }

    fn note(&self, url: &str, out: &Result<HttpResponse, TransportError>) {
        let Some(cell) = &self.connectivity else {
            return;
        };
        cell.set(match out {
            Err(_) => Connectivity::Offline,
            Ok(r) if matches!(r.status, 401 | 403) && self.is_api_url(url) => {
                Connectivity::AuthError
            }
            Ok(_) => Connectivity::Ok,
        });
    }

    /// True if `url` is on the API host: the only place the auth header goes (FR-PRV-09).
    pub fn is_api_url(&self, url: &str) -> bool {
        self.host.is_some() && host_of(url) == self.host && url.starts_with(&self.base)
    }

    /// Sends one request with the retry policy and returns the raw response (any status).
    /// Transport failures that survive the retries become `offline` / `timeout`.
    pub async fn send(
        &self,
        creds: Option<&Credentials>,
        method: Method,
        url: &str,
        body: &Body,
        timeout: Duration,
        retry: Retry,
    ) -> ProviderResult<HttpResponse> {
        let attempts = if retry == Retry::Idempotent {
            MAX_ATTEMPTS
        } else {
            1
        };
        let authorization = creds
            .filter(|_| self.is_api_url(url))
            .map(Credentials::basic_auth);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let (headers, bytes) = match body {
                Body::None => (vec![], None),
                Body::Json(v) => (
                    vec![("content-type".into(), "application/json".into())],
                    Some(v.to_string().into_bytes()),
                ),
                Body::Bytes {
                    bytes,
                    content_type,
                } => (
                    vec![("content-type".into(), content_type.clone())],
                    Some(bytes.clone()),
                ),
            };
            let request = HttpRequest {
                method,
                url: url.to_owned(),
                headers,
                authorization: authorization.clone(),
                body: bytes,
                timeout,
            };
            let out = self.transport.send(request).await;
            let retryable = match &out {
                Ok(r) => r.status == 429 || r.status >= 500,
                Err(_) => true,
            };
            match &out {
                Ok(r) => tracing::debug!(
                    method = method.as_str(),
                    url = log_url(url),
                    status = r.status,
                    attempt,
                    "provider request"
                ),
                Err(e) => {
                    tracing::debug!(method = method.as_str(), url = log_url(url), error = %e, attempt, "provider request failed")
                }
            }
            if !retryable || attempt >= attempts {
                self.note(url, &out);
                return out.map_err(|e| match e {
                    TransportError::Timeout => ProviderError::Timeout,
                    TransportError::Offline(_) => ProviderError::Offline,
                });
            }
            let backoff = Duration::from_secs(BACKOFF_SECS[attempt - 1]);
            let wait = match &out {
                Ok(r) => backoff.max(retry_after(r).unwrap_or_default().min(RETRY_AFTER_CAP)),
                Err(_) => backoff,
            };
            self.sleeper.sleep(wait).await;
        }
    }

    /// An authenticated JSON call to an API path (`/models/x?y=z`). 2xx → the parsed body
    /// (`null` for an empty one); anything else → the normalised error.
    pub async fn json(
        &self,
        creds: &Credentials,
        method: Method,
        path: &str,
        body: Option<Value>,
        retry: Retry,
    ) -> ProviderResult<(u16, Value)> {
        let url = format!("{}{path}", self.base);
        let body = body.map_or(Body::None, Body::Json);
        let resp = self
            .send(Some(creds), method, &url, &body, REQUEST_TIMEOUT, retry)
            .await?;
        if !resp.is_success() {
            return Err(map_error(&resp, Some(creds), true));
        }
        Ok((resp.status, parse_json(&resp.body)?))
    }
}

/// Parses a 2xx body; malformed JSON is `provider_failure`, never a panic (FR-PRV-10/23).
pub fn parse_json(body: &[u8]) -> ProviderResult<Value> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    serde_json::from_slice(body).map_err(|_| ProviderError::ProviderFailure {
        hint: "the provider sent a malformed response".into(),
    })
}

/// `Retry-After` in seconds (the HTTP-date form is treated as absent).
pub fn retry_after(resp: &HttpResponse) -> Option<Duration> {
    resp.header("retry-after")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(Duration::from_secs_f64)
}

/// The human part of a Scenario error body: `reason`, else gateway `message`, plus `hint`.
pub fn error_text(body: &Value) -> String {
    let pick = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").trim();
    let mut text = [pick("reason"), pick("message"), pick("error")]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned();
    let hint = pick("hint");
    if !hint.is_empty() && !text.contains(hint) {
        text = if text.is_empty() {
            hint.to_owned()
        } else {
            format!("{text} ({hint})")
        };
    }
    text
}

/// The field a 400/404/422 names, from `details` (a string, an object with
/// `field|input|name|param|path`, or a list of those). Defaults to `request`.
pub fn error_field(body: &Value) -> String {
    fn from(v: &Value) -> Option<String> {
        match v {
            Value::String(s) if !s.trim().is_empty() && s.len() <= 64 && !s.contains(' ') => {
                Some(s.trim().to_owned())
            }
            Value::Object(m) => ["field", "input", "name", "param", "parameter"]
                .iter()
                .find_map(|k| m.get(*k).and_then(from))
                .or_else(|| {
                    m.get("path")
                        .and_then(Value::as_array)
                        .and_then(|p| p.last())
                        .and_then(from)
                }),
            Value::Array(a) => a.iter().find_map(from),
            _ => None,
        }
    }
    body.get("details")
        .and_then(from)
        .unwrap_or_else(|| "request".into())
}

fn mentions_credits(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    [
        "credit",
        "creative unit",
        "compute unit",
        "insufficient balance",
        "not enough cu",
    ]
    .iter()
    .any(|k| t.contains(k))
        || t.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| w == "cu")
}

/// Hints are short and scrubbed of anything credential-shaped (FR-PRV-23).
pub fn clean_hint(text: &str, creds: Option<&Credentials>) -> String {
    let text = creds.map_or_else(|| text.to_owned(), |c| c.scrub(text));
    let text: String = text
        .lines()
        .filter(|l| !l.to_ascii_lowercase().contains("authorization"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = text.chars().take(200).collect();
    if out.trim().is_empty() {
        out = "no details".into();
    }
    out
}

/// The Appendix C error map for a non-2xx response. `api_route` = the request went to the
/// API host (401/403 there is `auth`; on a signed URL it is not).
pub fn map_error(
    resp: &HttpResponse,
    creds: Option<&Credentials>,
    api_route: bool,
) -> ProviderError {
    let body = serde_json::from_slice::<Value>(&resp.body).unwrap_or(Value::Null);
    let text = error_text(&body);
    match resp.status {
        401 | 403 if api_route => ProviderError::Auth,
        402 => ProviderError::InsufficientBalance,
        _ if mentions_credits(&text) && (400..500).contains(&resp.status) => {
            ProviderError::InsufficientBalance
        }
        429 => ProviderError::RateLimited {
            retry_after_ms: retry_after(resp)
                .unwrap_or(Duration::from_secs(BACKOFF_SECS[0]))
                .as_millis() as u64,
        },
        400 | 404 | 422 => ProviderError::InvalidRequest {
            field: error_field(&body),
        },
        status => ProviderError::ProviderFailure {
            hint: clean_hint(
                &if text.is_empty() {
                    format!("HTTP {status}")
                } else {
                    text
                },
                creds,
            ),
        },
    }
}

/// True if an estimate/submit rejection says a referenced asset is gone (FR-PRV-15).
pub fn is_missing_asset(resp: &HttpResponse) -> bool {
    if !matches!(resp.status, 400 | 404 | 410 | 422) {
        return false;
    }
    let body = serde_json::from_slice::<Value>(&resp.body).unwrap_or(Value::Null);
    let text = format!(
        "{} {}",
        error_text(&body),
        body.get("details")
            .map(Value::to_string)
            .unwrap_or_default()
    )
    .to_ascii_lowercase();
    text.contains("asset")
        && [
            "not found",
            "missing",
            "does not exist",
            "doesn't exist",
            "unknown",
            "deleted",
        ]
        .iter()
        .any(|k| text.contains(k))
}

/// Percent-encodes one path segment (ids are opaque).
pub fn seg(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use crate::replay::{Exchange, RecordedRequest, RecordedResponse, ReplayTransport};
    use crate::transport::RecordingSleeper;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn resp(status: u16, body: Value) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![],
            body: body.to_string().into_bytes(),
        }
    }

    /// One row per Appendix C error-map line (FR-PRV-23).
    #[test]
    fn fr_prv_23_error_map_rows() {
        let c = Credentials::new("kkkk", "ssss");
        let m = |r: &HttpResponse| map_error(r, Some(&c), true);
        assert_eq!(
            m(&resp(401, json!({"reason": "Unauthorized"}))),
            ProviderError::Auth
        );
        assert_eq!(m(&resp(403, json!({}))), ProviderError::Auth);
        assert_eq!(
            map_error(&resp(403, json!({})), Some(&c), false).code(),
            "provider_failure",
            "403 on a signed URL is not an auth error"
        );
        assert_eq!(m(&resp(402, json!({}))), ProviderError::InsufficientBalance);
        assert_eq!(
            m(&resp(
                400,
                json!({"reason": "Not enough creative units to run this job"})
            )),
            ProviderError::InsufficientBalance
        );
        let mut limited = resp(429, json!({}));
        limited.headers.push(("retry-after".into(), "7".into()));
        assert_eq!(
            m(&limited),
            ProviderError::RateLimited {
                retry_after_ms: 7000
            }
        );
        assert_eq!(
            m(&resp(
                400,
                json!({"reason": "bad", "details": {"field": "mask"}})
            )),
            ProviderError::InvalidRequest {
                field: "mask".into()
            }
        );
        assert_eq!(
            m(&resp(
                422,
                json!({"details": [{"path": ["body", "numOutputs"]}]})
            )),
            ProviderError::InvalidRequest {
                field: "numOutputs".into()
            }
        );
        assert_eq!(
            m(&resp(404, json!({}))),
            ProviderError::InvalidRequest {
                field: "request".into()
            }
        );
        assert_eq!(
            m(&resp(
                503,
                json!({"message": "Service Unavailable", "hint": "try later"})
            )),
            ProviderError::ProviderFailure {
                hint: "Service Unavailable (try later)".into()
            }
        );
        assert_eq!(
            parse_json(b"{not json").unwrap_err().code(),
            "provider_failure"
        );
    }

    #[test]
    fn fr_prv_23_error_text_never_contains_request_headers() {
        let c = Credentials::new("kkkk", "ssss");
        let echo = format!(
            "bad request\nAuthorization: {}\nkey kkkk secret ssss",
            c.basic_auth().expose()
        );
        let e = map_error(&resp(500, json!({"reason": echo})), Some(&c), true);
        let text = format!("{e} {e:?}");
        assert!(!text.contains("ssss") && !text.contains("kkkk"), "{text}");
        assert!(
            !text.to_ascii_lowercase().contains("authorization"),
            "{text}"
        );
    }

    #[test]
    fn fr_prv_15_missing_asset_detection() {
        assert!(is_missing_asset(&resp(
            404,
            json!({"reason": "Asset asset_1 not found"})
        )));
        assert!(is_missing_asset(&resp(
            400,
            json!({"reason": "Invalid input", "details": {"image": "asset does not exist"}})
        )));
        assert!(!is_missing_asset(&resp(
            400,
            json!({"reason": "prompt too long"})
        )));
        assert!(!is_missing_asset(&resp(
            500,
            json!({"reason": "asset not found"})
        )));
    }

    fn ex(status: u16, headers: &[(&str, &str)], body: Value) -> Exchange {
        Exchange {
            request: RecordedRequest {
                method: Method::Get,
                path: "/models/m".into(),
                query: BTreeMap::new(),
                body: None,
            },
            response: RecordedResponse {
                status,
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body,
            },
        }
    }

    fn api(exchanges: Vec<Exchange>) -> (Api, Arc<ReplayTransport>, Arc<RecordingSleeper>) {
        let t = Arc::new(ReplayTransport::from_exchanges(DEFAULT_BASE_URL, exchanges));
        let s = Arc::new(RecordingSleeper::default());
        (Api::new(DEFAULT_BASE_URL, t.clone(), s.clone()), t, s)
    }

    #[test]
    fn fr_prv_24_idempotent_calls_retry_5xx_and_offline_with_backoff() {
        let c = Credentials::new("k", "s");
        let (a, t, s) = api(vec![
            ex(0, &[], json!("offline")),
            ex(502, &[], json!({})),
            ex(200, &[], json!({"ok": true})),
        ]);
        let (status, body) =
            block_on(a.json(&c, Method::Get, "/models/m", None, Retry::Idempotent)).unwrap();
        assert_eq!((status, body), (200, json!({"ok": true})));
        assert_eq!(s.delays(), [Duration::from_secs(1), Duration::from_secs(2)]);
        assert_eq!(t.remaining(), 0);
    }

    #[test]
    fn fr_prv_24_at_most_three_attempts_then_the_normalised_error() {
        let c = Credentials::new("k", "s");
        let (a, _, s) = api(vec![
            ex(500, &[], json!({})),
            ex(500, &[], json!({})),
            ex(500, &[], json!({"reason": "boom"})),
        ]);
        let e =
            block_on(a.json(&c, Method::Get, "/models/m", None, Retry::Idempotent)).unwrap_err();
        assert_eq!(
            e,
            ProviderError::ProviderFailure {
                hint: "boom".into()
            }
        );
        assert_eq!(s.delays().len(), 2);

        let (a, _, _) = api(vec![
            ex(0, &[], json!("timeout")),
            ex(0, &[], json!("timeout")),
            ex(0, &[], json!("timeout")),
        ]);
        assert_eq!(
            block_on(a.json(&c, Method::Get, "/models/m", None, Retry::Idempotent)).unwrap_err(),
            ProviderError::Timeout
        );
        let (a, _, _) = api(vec![
            ex(0, &[], json!("offline")),
            ex(0, &[], json!("offline")),
            ex(0, &[], json!("offline")),
        ]);
        assert_eq!(
            block_on(a.json(&c, Method::Get, "/models/m", None, Retry::Idempotent)).unwrap_err(),
            ProviderError::Offline
        );
    }

    #[test]
    fn fr_prv_24_retry_after_is_honoured_and_capped_at_30_seconds() {
        let c = Credentials::new("k", "s");
        let (a, _, s) = api(vec![
            ex(429, &[("Retry-After", "5")], json!({})),
            ex(429, &[("Retry-After", "120")], json!({})),
            ex(200, &[], json!({})),
        ]);
        block_on(a.json(&c, Method::Get, "/models/m", None, Retry::Idempotent)).unwrap();
        assert_eq!(
            s.delays(),
            [Duration::from_secs(5), Duration::from_secs(30)]
        );
    }

    #[test]
    fn fr_prv_17_once_calls_are_never_retried() {
        let c = Credentials::new("k", "s");
        let (a, t, s) = api(vec![ex(503, &[], json!({}))]);
        let e = block_on(a.json(&c, Method::Get, "/models/m", None, Retry::Once)).unwrap_err();
        assert_eq!(e.code(), "provider_failure");
        assert!(s.delays().is_empty());
        assert_eq!(t.remaining(), 0);
    }

    #[test]
    fn fr_prv_09_auth_only_on_the_api_host() {
        let (a, _, _) = api(vec![]);
        assert!(a.is_api_url("https://api.cloud.scenario.com/v1/models"));
        assert!(!a.is_api_url("https://cdn.cloud.scenario.com/assets/x.png"));
        assert!(!a.is_api_url("https://api.cloud.scenario.com.evil.example/v1/x"));
        assert!(!a.is_api_url("fake://asset/1"));
    }
}
