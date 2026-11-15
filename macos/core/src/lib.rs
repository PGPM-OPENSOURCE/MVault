//! MVault core: the Memoro Vault format (byte-compatible with Memoro Vault 1.0.8+ and the Electron
//! MVault), its crypto, the QR paper backup, and the answer-strength estimate. No UI here.

pub mod build;
pub mod crypto;
pub mod entropy;
pub mod jsstr;
pub mod paper;
pub mod recover;
pub mod vault;
pub mod zxcvbn;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Crypto(String),
    #[error("{0}")]
    Format(String),
    #[error("{0}")]
    Io(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Format(e.to_string())
    }
}
impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Error::Format(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// `n` formatted like file-viewer.js `formatBytes`.
pub fn format_bytes(n: u64) -> String {
    let f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", f / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.2} MB", f / 1024.0 / 1024.0)
    } else {
        format!("{:.2} GB", f / 1024.0 / 1024.0 / 1024.0)
    }
}

/// build-vault.js `fmtBytes` (no GB step).
pub fn fmt_bytes_build(n: u64) -> String {
    let f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", f / 1024.0)
    } else {
        format!("{:.2} MB", f / 1024.0 / 1024.0)
    }
}
