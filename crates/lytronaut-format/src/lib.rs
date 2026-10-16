//! Independent preservation-first LFP/F01 and pure-Rust JPEG readers.
//! Original bytes and unknown metadata remain the caller's immutable assets.
//! Sensor mosaics and relative float maps are never calibrated scene photographs.
pub mod annex_b;
mod assets;
#[cfg(feature = "test-harness")]
pub mod capacity_observations;
mod container;
pub mod live_photo;
pub use assets::*;
pub use container::*;
pub use lytronaut_contracts::{AssetManifest, CapabilityState, ErrorCode};

/// Stable errors contain no private paths, camera identities or source metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    Budget,
    Truncated,
    Magic,
    Version,
    Alignment,
    Identity,
    Duplicate,
    MissingReference,
    Json,
    Metadata,
    Extent,
    Unsupported,
    NonFinite,
    Codec,
}
impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for FormatError {}
pub type Result<T> = std::result::Result<T, FormatError>;

/// Admission limits are enforced before parser allocations or decompression.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub container_bytes: usize,
    pub sections: usize,
    pub json_bytes: usize,
    pub json_depth: usize,
    pub json_nodes: usize,
    pub pixels: u64,
    pub decoded_bytes: u64,
    pub decoder_working_bytes: u64,
    pub jpeg_bytes: usize,
    pub dimension: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            container_bytes: 128 * 1024 * 1024,
            sections: 1024,
            json_bytes: 2 * 1024 * 1024,
            json_depth: 64,
            json_nodes: 100_000,
            pixels: 16_000_000,
            decoded_bytes: 64 * 1024 * 1024,
            decoder_working_bytes: 128 * 1024 * 1024,
            jpeg_bytes: 48 * 1024 * 1024,
            dimension: 8192,
        }
    }
}
impl Limits {
    pub fn extent(&self, width: u32, height: u32, bytes_per_pixel: u64) -> Result<usize> {
        if width == 0 || height == 0 || width > self.dimension || height > self.dimension {
            return Err(FormatError::Extent);
        }
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(FormatError::Extent)?;
        let bytes = pixels
            .checked_mul(bytes_per_pixel)
            .ok_or(FormatError::Extent)?;
        if pixels > self.pixels || bytes > self.decoded_bytes {
            return Err(FormatError::Budget);
        }
        usize::try_from(bytes).map_err(|_| FormatError::Budget)
    }
}

impl FormatError {
    pub fn error_code(self) -> ErrorCode {
        match self {
            Self::Budget => ErrorCode::ResourceLimit,
            Self::Version => ErrorCode::UnsupportedVersion,
            Self::Unsupported => ErrorCode::UnsupportedFormat,
            _ => ErrorCode::CorruptAsset,
        }
    }
}
