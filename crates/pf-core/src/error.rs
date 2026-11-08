//! Normalised command errors. Every command returns [`CoreError`]; codes are `snake_case` and use
//! PRJ's names where they apply (`read_only`, `locked`, `conflict`, …).

use std::fmt;

use serde::Serialize;
use serde_json::Value;

/// Error codes shared by every command. Add codes here; never rename one (the UI matches on them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Input failed validation at the command boundary (`field` says which).
    InvalidInput,
    NotFound,
    Unsupported,
    Animated,
    TooLarge,
    Corrupt,
    OutsideLibrary,
    LibraryUnavailable,
    NotAvailableOffline,
    NoProject,
    ReadOnly,
    Locked,
    Conflict,
    Unchanged,
    UnknownId,
    Unsaved,
    /// `edt_image_open` without `max_side` on an image over 4096 px per side: use tiles.
    UseTiles,
    /// A mask rect outside the image or over 4096 px per side (EDT `edt_mask_submit`).
    InvalidRect,
    /// A mask body that isn't w·h bytes (EDT `edt_mask_submit`).
    SizeMismatch,
    /// A provider error; `detail.provider` holds the normalised provider code.
    Provider,
    Io,
    Internal,
    /// The OS credential store can't be used (FR-PRV-06; no fallback file).
    StoreUnavailable,
    /// The provider has no credentials (`provider_test_connection`).
    NotConfigured,
    /// A fake-only command ran in another provider mode.
    NotFakeMode,
    /// A region under 16 × 16 px (RGN `rgn_region_fit`).
    RegionTooSmall,
    /// A region fully outside the image (RGN).
    OutOfBounds,
    /// An outer rect side over 4096 px (RGN).
    RegionTooLarge,
    /// The provider doesn't know the model (RGN).
    UnknownModel,
    /// The mask failed validation; `detail.reason` = `size|depth|empty` (RGN).
    MaskInvalid,
    /// The sent image or mask is over a model limit; `detail{field, limit}` (RGN, R20).
    InputTooLarge,
    /// The decoded original isn't ready yet (R11).
    OriginalUnavailable,
    /// A control value the capabilities reject; `detail{name, reason}` (RGN FR-RGN-33).
    InvalidControl,
    /// The estimate id doesn't match the job's cost-affecting inputs (FR-RGN-13).
    EstimateStale,
    /// Session spend + estimate is over the spend cap (FR-RGN-14).
    OverSpendCap,
    /// A needed confirmation wasn't given; `detail.needs` (FR-RGN-15/16).
    ConfirmationMissing,
    /// The asset project already has an active job (FR-RGN-17).
    JobActive,
    /// A settings value out of range (RGN `rgn_spend_set`).
    InvalidValue,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidInput => "invalid_input",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::Animated => "animated",
            ErrorCode::TooLarge => "too_large",
            ErrorCode::Corrupt => "corrupt",
            ErrorCode::OutsideLibrary => "outside_library",
            ErrorCode::LibraryUnavailable => "library_unavailable",
            ErrorCode::NotAvailableOffline => "not_available_offline",
            ErrorCode::NoProject => "no_project",
            ErrorCode::ReadOnly => "read_only",
            ErrorCode::Locked => "locked",
            ErrorCode::Conflict => "conflict",
            ErrorCode::Unchanged => "unchanged",
            ErrorCode::UnknownId => "unknown_id",
            ErrorCode::Unsaved => "unsaved",
            ErrorCode::UseTiles => "use_tiles",
            ErrorCode::InvalidRect => "invalid_rect",
            ErrorCode::SizeMismatch => "size_mismatch",
            ErrorCode::Provider => "provider",
            ErrorCode::Io => "io",
            ErrorCode::Internal => "internal",
            ErrorCode::StoreUnavailable => "store_unavailable",
            ErrorCode::NotConfigured => "not_configured",
            ErrorCode::NotFakeMode => "not_fake_mode",
            ErrorCode::RegionTooSmall => "region_too_small",
            ErrorCode::OutOfBounds => "out_of_bounds",
            ErrorCode::RegionTooLarge => "region_too_large",
            ErrorCode::UnknownModel => "unknown_model",
            ErrorCode::MaskInvalid => "mask_invalid",
            ErrorCode::InputTooLarge => "input_too_large",
            ErrorCode::OriginalUnavailable => "original_unavailable",
            ErrorCode::InvalidControl => "invalid_control",
            ErrorCode::EstimateStale => "estimate_stale",
            ErrorCode::OverSpendCap => "over_spend_cap",
            ErrorCode::ConfirmationMissing => "confirmation_missing",
            ErrorCode::JobActive => "job_active",
            ErrorCode::InvalidValue => "invalid_value",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error every core command returns, serialised to the UI as
/// `{code, message, field?, detail?}`.
#[derive(Debug, Clone, PartialEq, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct CoreError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

/// Result of a core command.
pub type CoreResult<T> = Result<T, CoreError>;

impl CoreError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        CoreError {
            code,
            message: message.into(),
            field: None,
            detail: Value::Null,
        }
    }

    /// A field-level validation error.
    pub fn invalid(field: impl Into<String>, message: impl Into<String>) -> Self {
        CoreError {
            field: Some(field.into()),
            ..CoreError::new(ErrorCode::InvalidInput, message)
        }
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = detail;
        self
    }

    /// Maps an I/O error: sharing violations become `locked`, missing files `not_found`.
    pub fn io(context: impl fmt::Display, e: &std::io::Error) -> Self {
        let code = if is_locked(e) {
            ErrorCode::Locked
        } else if e.kind() == std::io::ErrorKind::NotFound {
            ErrorCode::NotFound
        } else {
            ErrorCode::Io
        };
        CoreError::new(code, format!("{context}: {e}"))
    }

    pub fn internal(message: impl Into<String>) -> Self {
        CoreError::new(ErrorCode::Internal, message)
    }
}

impl From<pf_provider::ProviderError> for CoreError {
    fn from(e: pf_provider::ProviderError) -> Self {
        let detail = serde_json::to_value(&e).unwrap_or(Value::Null);
        CoreError::new(ErrorCode::Provider, e.to_string())
            .with_detail(serde_json::json!({ "provider": detail }))
    }
}

/// True for errors another process holding the file causes (Windows sharing or lock violation).
pub fn is_locked(e: &std::io::Error) -> bool {
    // ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33), ERROR_ACCESS_DENIED (5) on rename
    // over a file another process has open.
    cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_code_message_and_field() {
        let e = CoreError::invalid("roots[0]", "folder does not exist");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"code": "invalid_input", "message": "folder does not exist", "field": "roots[0]"})
        );
        assert_eq!(ErrorCode::ReadOnly.to_string(), "read_only");
    }

    #[test]
    fn provider_errors_keep_their_normalised_code() {
        let e: CoreError = pf_provider::ProviderError::Offline.into();
        assert_eq!(e.code, ErrorCode::Provider);
        assert_eq!(e.detail["provider"]["code"], "offline");
    }
}
