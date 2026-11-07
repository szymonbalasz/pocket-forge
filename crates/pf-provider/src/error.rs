//! Normalised provider errors (ADR 0001).

use serde::Serialize;

/// The normalised error set. Text never contains request headers or credentials (FR-PRV-23).
#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ProviderError {
    #[error("the provider rejected the credentials")]
    Auth,
    #[error("the provider balance is too low for this job")]
    InsufficientBalance,
    #[error("rate limited; retry after {retry_after_ms} ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("invalid request: {field}")]
    InvalidRequest { field: String },
    #[error("provider failure: {hint}")]
    ProviderFailure { hint: String },
    #[error("the provider can't be reached")]
    Offline,
    #[error("the provider timed out")]
    Timeout,
}

impl ProviderError {
    /// The `snake_case` code used across the app.
    pub fn code(&self) -> &'static str {
        match self {
            ProviderError::Auth => "auth",
            ProviderError::InsufficientBalance => "insufficient_balance",
            ProviderError::RateLimited { .. } => "rate_limited",
            ProviderError::InvalidRequest { .. } => "invalid_request",
            ProviderError::ProviderFailure { .. } => "provider_failure",
            ProviderError::Offline => "offline",
            ProviderError::Timeout => "timeout",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prv_23_codes_are_snake_case_and_serialise_as_tagged() {
        let e = ProviderError::RateLimited {
            retry_after_ms: 1500,
        };
        assert_eq!(e.code(), "rate_limited");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"code": "rate_limited", "retry_after_ms": 1500})
        );
        assert_eq!(
            ProviderError::InsufficientBalance.code(),
            "insufficient_balance"
        );
    }
}
