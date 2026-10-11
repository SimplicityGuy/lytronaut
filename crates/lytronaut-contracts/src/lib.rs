//! Version 1 platform-neutral wire contracts. Invalid inbound states fail deserialization.
//! Opaque IDs are application identifiers; source paths and camera identities never belong here.
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractError(pub &'static str);
impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for ContractError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub enum WireVersion {
    V1,
}
impl TryFrom<u16> for WireVersion {
    type Error = ContractError;
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        if v == 1 {
            Ok(Self::V1)
        } else {
            Err(ContractError("unsupported contract version"))
        }
    }
}
impl From<WireVersion> for u16 {
    fn from(_: WireVersion) -> Self {
        1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OpaqueId(String);
impl OpaqueId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for OpaqueId {
    type Error = ContractError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ContractError("invalid opaque identifier"));
        }
        Ok(Self(value))
    }
}
impl From<OpaqueId> for String {
    fn from(v: OpaqueId) -> Self {
        v.0
    }
}

/// No raw exception messages, filesystem paths or serials are sent to clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    Unauthorized,
    NotFound,
    UnsupportedVersion,
    UnsupportedFormat,
    ResourceLimit,
    CorruptAsset,
    Cancelled,
    StorageUnavailable,
    Internal,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    pub version: WireVersion,
    pub code: ErrorCode,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum CapabilityState {
    Available,
    PreservedOnly,
    Unverified,
    Disabled,
    Unavailable(ErrorCode),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityReport {
    pub version: WireVersion,
    pub originals: CapabilityState,
    pub static_preview: CapabilityState,
    pub discrete_focus: CapabilityState,
    pub calibrated_synthesis: CapabilityState,
    pub optional_renderer: CapabilityState,
    pub optional_codec: CapabilityState,
    pub legacy_edits: CapabilityState,
}
impl CapabilityReport {
    pub fn foundation() -> Self {
        Self {
            version: WireVersion::V1,
            originals: CapabilityState::Disabled,
            static_preview: CapabilityState::Disabled,
            discrete_focus: CapabilityState::Disabled,
            calibrated_synthesis: CapabilityState::Unverified,
            optional_renderer: CapabilityState::Unverified,
            optional_codec: CapabilityState::Unverified,
            legacy_edits: CapabilityState::PreservedOnly,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthReport {
    pub version: WireVersion,
    pub ready: bool,
    pub capabilities: CapabilityReport,
}

macro_rules! checked {
    ($name:ident, $raw:ident, {$($field:ident: $ty:ty),* $(,)?}) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        pub struct $name { $(pub $field: $ty),* }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $raw { $($field: $ty),* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Self::try_from($raw::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
        impl TryFrom<$raw> for $name {
            type Error = ContractError;
            fn try_from(raw: $raw) -> Result<Self, Self::Error> {
                let value = Self { $($field: raw.$field),* };
                value.validate()?;
                Ok(value)
            }
        }
    };
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    ZipLibrary,
    Directory,
}
checked!(ImportRequest, RawImportRequest, {version: WireVersion, kind: ImportKind, declared_bytes: u64, declared_files: u32});
impl ImportRequest {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.declared_bytes == 0 || self.declared_files == 0 {
            return Err(ContractError("invalid import declaration"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Cancelling,
    Cancelled,
    Succeeded,
    Failed,
}
checked!(JobReport, RawJobReport, {version: WireVersion, id: OpaqueId, state: JobState, completed_bytes: u64, total_bytes: u64, error: Option<ApiError>});
impl JobReport {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.completed_bytes > self.total_bytes
            || (self.state == JobState::Succeeded && self.completed_bytes != self.total_bytes)
            || (self.state == JobState::Failed) != self.error.is_some()
        {
            return Err(ContractError("inconsistent job progress or error"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    Original,
    JpegPreview,
    JpegFocusPlane,
    SensorSamples,
    Opaque,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSpace {
    Srgb,
    Unknown,
    NotApplicable,
}
checked!(Dimensions, RawDimensions, {width: u32, height: u32});
impl Dimensions {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.width == 0 || self.height == 0 || self.width > 65_536 || self.height > 65_536 {
            return Err(ContractError("invalid asset dimensions"));
        }
        Ok(())
    }
}
checked!(ChunkRange, RawChunkRange, {offset: u64, length: u64});
impl ChunkRange {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.length == 0
            || self.length > 8 * 1024 * 1024
            || self.offset.checked_add(self.length).is_none()
        {
            return Err(ContractError("invalid chunk range"));
        }
        Ok(())
    }
}
checked!(AssetManifest, RawAssetManifest, {
    version: WireVersion, id: OpaqueId, content_sha256: ContentDigest,
    representation: Representation, encoded_bytes: u64, decoded_bytes: Option<u64>,
    dimensions: Option<Dimensions>, color_space: ColorSpace,
    chunks: Vec<ChunkRange>, capabilities: CapabilityReport
});
impl AssetManifest {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.encoded_bytes == 0 || self.chunks.len() > 1024 {
            return Err(ContractError("invalid manifest size"));
        }
        if let Some(d) = &self.dimensions {
            d.validate()?;
        }
        if matches!(
            self.representation,
            Representation::JpegPreview | Representation::JpegFocusPlane
        ) {
            let d = self
                .dimensions
                .as_ref()
                .ok_or(ContractError("preview requires dimensions"))?;
            if self.chunks.is_empty()
                || d.width > 2048
                || d.height > 2048
                || self.encoded_bytes > 48 * 1024 * 1024
                || self.decoded_bytes != Some(u64::from(d.width) * u64::from(d.height) * 4)
            {
                return Err(ContractError(
                    "preview exceeds viewer budget or decoded size is inconsistent",
                ));
            }
        }
        let mut end = 0;
        for chunk in &self.chunks {
            chunk.validate()?;
            if chunk.offset != end {
                return Err(ContractError("chunks must be contiguous and ordered"));
            }
            end = chunk
                .offset
                .checked_add(chunk.length)
                .ok_or(ContractError("chunk overflow"))?;
            if end > self.encoded_bytes {
                return Err(ContractError("chunk exceeds asset"));
            }
        }
        // Empty chunks denote a preserved original not offered to the bounded viewer.
        if !self.chunks.is_empty() && end != self.encoded_bytes {
            return Err(ContractError("incomplete chunk coverage"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentDigest(String);
impl TryFrom<String> for ContentDigest {
    type Error = ContractError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ContractError("invalid SHA-256 digest"));
        }
        Ok(Self(value))
    }
}
impl From<ContentDigest> for String {
    fn from(v: ContentDigest) -> Self {
        v.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportPhase {
    Intake,
    CatalogPending,
    CatalogComplete,
    CatalogFailed,
    Failed,
    Cancelled,
    Cancelling,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginalRole {
    Raw,
    Depth,
    RefocusStack,
    JpegFocusStack,
    ViewStack,
    Thumbnail,
    LegacyEdits,
    Catalog,
    Other,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemError {
    pub role: OriginalRole,
    pub error: ApiError,
}
/// Semantic readiness is distinct from durable intake success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryState {
    Ready,
    PreservedOnly,
    Quarantined,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportProgress {
    #[serde(flatten)]
    pub intake: JobReport,
    pub phase: ImportPhase,
    pub library: Option<OpaqueId>,
    pub catalog_state: Option<LibraryState>,
    pub catalog_error: Option<ApiError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub version: WireVersion,
    pub items: Vec<T>,
    pub offset: u32,
    pub next_offset: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySummary {
    pub id: OpaqueId,
    pub state: LibraryState,
    pub captures: u32,
    pub albums: u32,
    pub files: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumSummary {
    pub id: OpaqueId,
    pub name: String,
    pub name_truncated: bool,
    pub sort_order: i64,
    pub captures: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureSummary {
    pub id: OpaqueId,
    pub name: String,
    pub name_truncated: bool,
    pub rating: i64,
    pub flag_status: i64,
    pub picture_index: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreservedFile {
    pub id: OpaqueId,
    pub encoded_bytes: u64,
    pub role: OriginalRole,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureDetail {
    pub version: WireVersion,
    pub capture: CaptureSummary,
    pub caption: String,
    pub caption_truncated: bool,
    pub capabilities: CapabilityReport,
    pub preview: Option<OpaqueId>,
    pub preview_error: Option<ApiError>,
    pub item_errors: Vec<ItemError>,
    pub originals: Vec<PreservedFile>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_roundtrip_and_preserved_only() {
        let fixture = include_str!("../../../fixtures/synthetic/asset-v1.json");
        let asset: AssetManifest = serde_json::from_str(fixture).unwrap();
        assert_eq!(asset.capabilities.originals, CapabilityState::PreservedOnly);
        assert_eq!(
            serde_json::from_str::<AssetManifest>(&serde_json::to_string(&asset).unwrap()).unwrap(),
            asset
        );
    }
    #[test]
    fn rejects_future_versions_paths_and_invalid_digests() {
        assert!(serde_json::from_str::<WireVersion>("2").is_err());
        for id in ["../secret", "/private", "camera:serial", "", "a\\b"] {
            assert!(OpaqueId::try_from(id.to_owned()).is_err());
        }
        assert!(ContentDigest::try_from("x".repeat(64)).is_err());
    }
    #[test]
    fn rejects_hostile_asset_extents_and_chunks() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/synthetic/asset-v1.json"))
                .unwrap();
        for (key, value) in [
            ("version", serde_json::json!(9)),
            ("encoded_bytes", serde_json::json!(0)),
            ("source_path", serde_json::json!("secret")),
            (
                "chunks",
                serde_json::json!([{"offset":18446744073709551615_u64,"length":2}]),
            ),
        ] {
            let mut candidate = fixture.clone();
            candidate[key] = value;
            assert!(
                serde_json::from_value::<AssetManifest>(candidate).is_err(),
                "{key}"
            );
        }
        let mut candidate = fixture;
        candidate["representation"] = serde_json::json!("jpeg_preview");
        candidate["dimensions"] = serde_json::json!({"width":2049,"height":2049});
        assert!(serde_json::from_value::<AssetManifest>(candidate).is_err());
    }
    #[test]
    fn rejects_inconsistent_jobs_and_unbounded_declarations() {
        assert!(
            serde_json::from_str::<ImportRequest>(
                r#"{"version":1,"kind":"zip_library","declared_bytes":0,"declared_files":1}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<JobReport>(r#"{"version":1,"id":"job-1","state":"succeeded","completed_bytes":1,"total_bytes":2,"error":null}"#).is_err());
        assert!(serde_json::from_str::<JobReport>(r#"{"version":1,"id":"job-1","state":"failed","completed_bytes":0,"total_bytes":2,"error":null}"#).is_err());
    }

    #[test]
    fn configured_admission_limits_are_separate_from_wire_validity() {
        assert!(serde_json::from_str::<ImportRequest>(r#"{"version":1,"kind":"zip_library","declared_bytes":21474836481,"declared_files":100001}"#).is_ok());
    }

    #[test]
    fn viewer_requires_complete_bounded_coverage() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/synthetic/asset-v1.json"))
                .unwrap();
        value["representation"] = serde_json::json!("jpeg_preview");
        value["dimensions"] = serde_json::json!({"width":2,"height":2});
        value["decoded_bytes"] = serde_json::json!(16);
        assert!(serde_json::from_value::<AssetManifest>(value.clone()).is_ok());
        for chunks in [
            serde_json::json!([]),
            serde_json::json!([{"offset":0,"length":15}]),
            serde_json::json!([{"offset":1,"length":15}]),
            serde_json::json!([{"offset":0,"length":9},{"offset":8,"length":8}]),
        ] {
            value["chunks"] = chunks;
            assert!(serde_json::from_value::<AssetManifest>(value.clone()).is_err());
        }
    }
}
