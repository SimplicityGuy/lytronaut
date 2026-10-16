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
    #[cfg(feature = "test-harness")]
    pub fn harness_capacity(&self) -> usize {
        self.0.capacity()
    }
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
    /// Only on a `resource_limit` refusal caused by insufficient free work
    /// space; omitted from the wire otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortfall: Option<SpaceShortfall>,
}
/// Byte counts only (never a path): what admission needed on the work volume,
/// including active reservations and the free-space floor, and what was free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceShortfall {
    pub required_bytes: u64,
    pub available_bytes: u64,
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
impl ContentDigest {
    #[cfg(feature = "test-harness")]
    pub fn harness_capacity(&self) -> usize {
        self.0.capacity()
    }
}
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
/// A newly encoded, lossy JPEG of a stored plane after an explicit viewer transform.
/// Sent with the single bounded response in X-Lytronaut-Derivative; no source hashes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusDerivative {
    pub version: WireVersion,
    pub representation: Representation,
    pub id: OpaqueId,
    pub plane_index: usize,
    pub lambda: f64,
    pub source_dimensions: Dimensions,
    pub output_dimensions: Dimensions,
    pub orientation: u8,
    pub encoded_bytes: u64,
    pub content_sha256: ContentDigest,
    pub color_space: ColorSpace,
    pub provenance: String,
}
impl FocusDerivative {
    pub fn validate(&self, stack: &FocusStack) -> Result<(), ContractError> {
        stack.validate()?;
        let source = &stack.source_dimensions;
        let (w, h) = if self.orientation >= 5 {
            (source.height, source.width)
        } else {
            (source.width, source.height)
        };
        if self.representation != Representation::JpegFocusPlane
            || self.id != stack.id
            || self.plane_index >= stack.lambdas.len()
            || self.lambda != stack.lambdas[self.plane_index]
            || !(1..=8).contains(&self.orientation)
            || self.source_dimensions != *source
            || self.output_dimensions.width != w
            || self.output_dimensions.height != h
            || self.encoded_bytes == 0
            || self.encoded_bytes > 2 * 1024 * 1024
            || self.color_space != ColorSpace::Unknown
            || self.provenance.len() > 512
        {
            return Err(ContractError("invalid focus derivative"));
        }
        Ok(())
    }
}
/// Public derivative selection metadata; source identities and recipes remain private.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusStack {
    pub version: WireVersion,
    pub id: OpaqueId,
    pub lambdas: Vec<f64>,
    pub source_dimensions: Dimensions,
    /// Explicit viewer transform only. Imported processed recipes are never replayed.
    pub orientation_policy: String,
}
impl FocusStack {
    pub fn validate(&self) -> Result<(), ContractError> {
        self.source_dimensions.validate()?;
        if self.orientation_policy.len() > 512
            || self.lambdas.is_empty()
            || self.lambdas.len() > 64
            || self.source_dimensions.width > 512
            || self.source_dimensions.height > 512
            || self.lambdas.iter().any(|v| !v.is_finite())
            || self.lambdas.windows(2).any(|v| v[0] >= v[1])
        {
            return Err(ContractError("invalid discrete focus stack"));
        }
        Ok(())
    }
    /// Clamp naturally to endpoints; exact midpoint ties choose the lower stored plane.
    pub fn nearest(&self, requested: f64) -> Result<usize, ContractError> {
        self.validate()?;
        if !requested.is_finite() {
            return Err(ContractError("invalid focus value"));
        }
        if requested <= self.lambdas[0] {
            return Ok(0);
        }
        if requested >= *self.lambdas.last().unwrap() {
            return Ok(self.lambdas.len() - 1);
        }
        let mut best = 0;
        for i in 1..self.lambdas.len() {
            if (self.lambdas[i] - requested).abs() < (self.lambdas[best] - requested).abs() {
                best = i;
            }
        }
        Ok(best)
    }
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
    fn api_error_shortfall_is_optional_numeric_and_strict() {
        let plain = r#"{"version":1,"code":"resource_limit","retryable":true}"#;
        let error: ApiError = serde_json::from_str(plain).unwrap();
        assert_eq!(error.shortfall, None);
        assert_eq!(serde_json::to_string(&error).unwrap(), plain);
        let short = r#"{"version":1,"code":"resource_limit","retryable":true,"shortfall":{"required_bytes":18446744073709551615,"available_bytes":0}}"#;
        let error: ApiError = serde_json::from_str(short).unwrap();
        assert_eq!(
            error.shortfall,
            Some(SpaceShortfall {
                required_bytes: u64::MAX,
                available_bytes: 0
            })
        );
        assert_eq!(serde_json::to_string(&error).unwrap(), short);
        // Backward compatibility: a client built before the field existed and
        // ignoring unknown fields still parses a shortfall refusal, and the
        // former strict shape still parses every error that has no shortfall
        // (job reports and all other refusals are byte-identical to before).
        #[derive(Deserialize)]
        struct OlderLenient {
            code: ErrorCode,
            retryable: bool,
        }
        let older: OlderLenient = serde_json::from_str(short).unwrap();
        assert_eq!(
            (older.code, older.retryable),
            (ErrorCode::ResourceLimit, true)
        );
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct OlderStrict {
            version: WireVersion,
            code: ErrorCode,
            retryable: bool,
        }
        assert!(serde_json::from_str::<OlderStrict>(plain).is_ok());
        for hostile in [
            r#"{"version":1,"code":"resource_limit","retryable":true,"shortfall":{"required_bytes":1,"available_bytes":0,"path":"/x"}}"#,
            r#"{"version":1,"code":"resource_limit","retryable":true,"shortfall":{"required_bytes":-1,"available_bytes":0}}"#,
            r#"{"version":1,"code":"resource_limit","retryable":true,"message":"/srv/work"}"#,
        ] {
            assert!(serde_json::from_str::<ApiError>(hostile).is_err());
        }
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

#[cfg(test)]
mod focus_tests {
    use super::*;
    fn stack() -> FocusStack {
        FocusStack {
            version: WireVersion::V1,
            id: OpaqueId::try_from("synthetic-focus".to_owned()).unwrap(),
            lambdas: vec![-2.0, 0.0, 3.0],
            source_dimensions: Dimensions {
                width: 360,
                height: 360,
            },
            orientation_policy: "Explicit viewer transform only".into(),
        }
    }
    #[test]
    fn focus_clamps_ties_and_rejects_hostile_metadata() {
        let mut s = stack();
        for (value, index) in [
            (-100.0, 0),
            (-2.0, 0),
            (-1.0, 0),
            (0.0, 1),
            (1.5, 1),
            (1.5001, 2),
            (100.0, 2),
        ] {
            assert_eq!(s.nearest(value).unwrap(), index);
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(s.nearest(value).is_err());
        }
        for lambdas in [
            vec![],
            vec![0.0, 0.0],
            vec![1.0, 0.0],
            vec![f64::NAN],
            vec![0.0; 65],
        ] {
            s.lambdas = lambdas;
            assert!(s.validate().is_err());
        }
        s = stack();
        s.source_dimensions.width = 513;
        assert!(s.validate().is_err());
    }
}

#[cfg(test)]
mod derivative_tests {
    use super::*;
    fn fixture() -> (FocusStack, FocusDerivative) {
        let id = OpaqueId::try_from("synthetic-focus".to_owned()).unwrap();
        let dimensions = Dimensions {
            width: 360,
            height: 240,
        };
        let stack = FocusStack {
            version: WireVersion::V1,
            id: id.clone(),
            lambdas: vec![-1.0, 1.0],
            source_dimensions: dimensions.clone(),
            orientation_policy: "Processed edits preserved".into(),
        };
        let derivative = FocusDerivative {
            version: WireVersion::V1,
            representation: Representation::JpegFocusPlane,
            id,
            plane_index: 0,
            lambda: -1.0,
            source_dimensions: dimensions.clone(),
            output_dimensions: dimensions,
            orientation: 1,
            encoded_bytes: 32,
            content_sha256: "a".repeat(64).try_into().unwrap(),
            color_space: ColorSpace::Unknown,
            provenance: "Lossy re-encode, processed recipe preserved".into(),
        };
        (stack, derivative)
    }
    #[test]
    fn derivative_extent_selection_transform_provenance_and_versions_are_checked() {
        let (stack, derivative) = fixture();
        derivative.validate(&stack).unwrap();
        for code in 1..=8 {
            let mut d = derivative.clone();
            d.orientation = code;
            if code >= 5 {
                d.output_dimensions = Dimensions {
                    width: 240,
                    height: 360,
                };
            }
            d.validate(&stack).unwrap();
        }
        let valid = serde_json::to_value(&derivative).unwrap();
        for (key, value) in [
            ("id", serde_json::json!("different")),
            ("lambda", serde_json::json!(0.0)),
            ("plane_index", serde_json::json!(64)),
            ("orientation", serde_json::json!(0)),
            ("encoded_bytes", serde_json::json!(0)),
            ("encoded_bytes", serde_json::json!(2097153)),
            ("color_space", serde_json::json!("srgb")),
            ("representation", serde_json::json!("original")),
            ("provenance", serde_json::json!("a".repeat(513))),
            (
                "output_dimensions",
                serde_json::json!({"width":240,"height":360}),
            ),
            (
                "source_dimensions",
                serde_json::json!({"width":360,"height":360}),
            ),
        ] {
            let mut mutated = valid.clone();
            mutated[key] = value;
            let d: FocusDerivative = serde_json::from_value(mutated).unwrap();
            assert!(d.validate(&stack).is_err(), "accepted {key}");
        }
        let mut invalid = valid;
        invalid["version"] = serde_json::json!(2);
        assert!(serde_json::from_value::<FocusDerivative>(invalid).is_err());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Jpeg,
    Png,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExportRequest {
    LivePhoto {
        version: WireVersion,
        asset: OpaqueId,
        plane_index: usize,
        orientation: u8,
    },
    Flatten {
        version: WireVersion,
        asset: OpaqueId,
        format: ExportFormat,
        plane_index: Option<usize>,
        orientation: u8,
        operations: Vec<String>,
    },
    ArchiveOriginals {
        version: WireVersion,
    },
}
impl ExportRequest {
    pub fn validate(&self) -> Result<(), ContractError> {
        if let Self::LivePhoto {
            plane_index,
            orientation,
            ..
        } = self
            && (*plane_index >= 64 || !(1..=8).contains(orientation))
        {
            return Err(ContractError("select a stored key plane and EXIF 1–8"));
        }
        if let Self::Flatten {
            orientation,
            operations,
            plane_index,
            ..
        } = self
        {
            if !(1..=8).contains(orientation) || plane_index.is_some_and(|v| v >= 64) {
                return Err(ContractError("select EXIF 1–8 and a stored plane"));
            }
            if !operations.is_empty() {
                return Err(ContractError(
                    "imported aperture/perspective/tone/denoise/animation are unsupported; remove operations and export verified stored pixels",
                ));
            }
        }
        Ok(())
    }
}
/// Separate Lytronaut record. Legacy sidecars are preserved without alteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRecipe {
    pub version: WireVersion,
    pub asset: OpaqueId,
    pub source_variant: String,
    pub plane_index: Option<usize>,
    pub lambda: Option<f64>,
    pub orientation: u8,
    pub source_dimensions: Dimensions,
    pub output_dimensions: Dimensions,
    /// Full verified stored image: x, y, width, height. No inferred sensor crop.
    pub visible_geometry: [u32; 4],
    pub format: ExportFormat,
    pub color_space: ColorSpace,
    pub encoded_bytes: u64,
    pub content_sha256: ContentDigest,
    pub processing_loss: String,
    pub color_policy: String,
    pub refocus_loss: bool,
    pub imported_adjustments: String,
}

#[cfg(test)]
mod export_tests {
    use super::*;
    #[test]
    fn export_request_rejects_unknown_operations_versions_and_orientation() {
        let valid = serde_json::json!({"kind":"flatten","version":1,"asset":"synthetic-jpeg","format":"png","plane_index":null,"orientation":1,"operations":[]});
        serde_json::from_value::<ExportRequest>(valid.clone())
            .unwrap()
            .validate()
            .unwrap();
        for (key, value) in [
            ("orientation", serde_json::json!(0)),
            ("orientation", serde_json::json!(9)),
            ("plane_index", serde_json::json!(64)),
            ("operations", serde_json::json!(["aperture"])),
            ("operations", serde_json::json!(["perspective"])),
            ("operations", serde_json::json!(["tone"])),
            ("operations", serde_json::json!(["denoise"])),
            ("operations", serde_json::json!(["animation"])),
        ] {
            let mut v = valid.clone();
            v[key] = value;
            assert!(
                serde_json::from_value::<ExportRequest>(v)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        for (key, value) in [
            ("version", serde_json::json!(2)),
            ("source_path", serde_json::json!("../private")),
            ("format", serde_json::json!("video")),
        ] {
            let mut v = valid.clone();
            v[key] = value;
            assert!(serde_json::from_value::<ExportRequest>(v).is_err());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    LivePhoto,
    Flatten,
    Archive,
    Restore,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportProgress {
    #[serde(flatten)]
    pub job: JobReport,
    pub kind: ExportKind,
}
#[cfg(test)]
mod export_progress_tests {
    use super::*;
    #[test]
    fn typed_export_progress_roundtrip_and_terminal_validation() {
        let p = ExportProgress {
            job: JobReport {
                version: WireVersion::V1,
                id: "synthetic-export".to_owned().try_into().unwrap(),
                state: JobState::Succeeded,
                completed_bytes: 12,
                total_bytes: 12,
                error: None,
            },
            kind: ExportKind::Flatten,
        };
        let value = serde_json::to_value(&p).unwrap();
        let got: ExportProgress = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(got.kind, p.kind);
        assert_eq!(got.job.state, p.job.state);
        let mut invalid = value;
        invalid["completed_bytes"] = serde_json::json!(13);
        assert!(serde_json::from_value::<ExportProgress>(invalid).is_err());
    }
}

#[cfg(test)]
mod live_photo_contract_tests {
    use super::*;
    #[test]
    fn selected_key_and_explicit_transform_are_required_and_unknown_edits_rejected() {
        let valid = serde_json::json!({"kind":"live_photo","version":1,"asset":"synthetic-focus","plane_index":1,"orientation":6});
        serde_json::from_value::<ExportRequest>(valid.clone())
            .unwrap()
            .validate()
            .unwrap();
        for (key, value) in [
            ("plane_index", serde_json::json!(64)),
            ("orientation", serde_json::json!(0)),
            ("orientation", serde_json::json!(9)),
        ] {
            let mut mutated = valid.clone();
            mutated[key] = value;
            assert!(
                serde_json::from_value::<ExportRequest>(mutated)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        for (key, value) in [
            ("version", serde_json::json!(2)),
            ("operations", serde_json::json!(["tone"])),
            ("source_path", serde_json::json!("private")),
        ] {
            let mut mutated = valid.clone();
            mutated[key] = value;
            assert!(serde_json::from_value::<ExportRequest>(mutated).is_err());
        }
    }
}
