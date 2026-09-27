//! Engine error type. Wraps `FitError` plus backend-specific failures.

use calibration_types::FitError;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EngineError {
    #[error(transparent)]
    Fit(#[from] FitError),

    #[error("FFT backend: {0}")]
    Fft(String),

    #[error("curve interpolation: {0}")]
    Interp(String),

    #[error("cache: {0}")]
    Cache(String),

    #[error("unknown: {0}")]
    Other(String),
}

// Allow EngineError → FitResultCacheError coercion so callers can keep ?.
impl From<EngineError> for String {
    fn from(e: EngineError) -> Self {
        e.to_string()
    }
}
