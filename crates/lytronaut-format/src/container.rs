use crate::{FormatError as E, Limits, Result};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet};

const PREFIX: &[u8; 8] = b"\x89LFP\r\n\x1a\n";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Directory,
    Content,
}
#[derive(Clone, Copy)]
pub struct Section<'a> {
    pub kind: SectionKind,
    pub identity: &'a str,
    pub payload: &'a [u8],
    pub offset: usize,
}
/// Debug deliberately avoids payloads and section identities.
impl std::fmt::Debug for Section<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Section")
            .field("kind", &self.kind)
            .field("bytes", &self.payload.len())
            .finish()
    }
}
#[derive(Debug)]
pub struct Container<'a> {
    pub sections: Vec<Section<'a>>,
}
fn align16(n: usize) -> Result<usize> {
    n.checked_add(15).map(|v| v & !15).ok_or(E::Alignment)
}
fn identity(s: &str) -> bool {
    s.len() == 45
        && s.starts_with("sha1-")
        && s[5..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
impl<'a> Container<'a> {
    pub fn parse(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        if bytes.len() > limits.container_bytes {
            return Err(E::Budget);
        }
        let header = bytes.get(..16).ok_or(E::Truncated)?;
        if &header[..8] != PREFIX {
            return Err(E::Magic);
        }
        if header[8..12] != [0, 0, 0, 1] {
            return Err(E::Version);
        }
        if header[12..16] != [0; 4] {
            return Err(E::Magic);
        }
        let mut sections = Vec::new();
        let mut seen = BTreeSet::new();
        let mut pos = 16;
        while pos < bytes.len() {
            if sections.len() >= limits.sections {
                return Err(E::Budget);
            }
            let end = pos.checked_add(96).ok_or(E::Truncated)?;
            let h = bytes.get(pos..end).ok_or(E::Truncated)?;
            let kind = match &h[..8] {
                b"\x89LFM\r\n\x1a\n" => SectionKind::Directory,
                b"\x89LFC\r\n\x1a\n" => SectionKind::Content,
                _ => return Err(E::Magic),
            };
            if h[8..12] != [0, 0, 0, 0] {
                return Err(E::Version);
            }
            let n = u32::from_be_bytes(h[12..16].try_into().unwrap()) as usize;
            if n == 0 {
                return Err(E::Extent);
            }
            let id = std::str::from_utf8(&h[16..61]).map_err(|_| E::Identity)?;
            if !identity(id) || h[61..].iter().any(|b| *b != 0) {
                return Err(E::Identity);
            }
            let payload_end = end.checked_add(n).ok_or(E::Truncated)?;
            let payload = bytes.get(end..payload_end).ok_or(E::Truncated)?;
            if format!(
                "sha1-{}",
                Sha1::digest(payload)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            ) != id
            {
                return Err(E::Identity);
            }
            if !seen.insert(id) {
                return Err(E::Duplicate);
            }
            let next = align16(payload_end)?;
            if next > bytes.len() {
                return Err(E::Truncated);
            }
            if bytes[payload_end..next].iter().any(|b| *b != 0) {
                return Err(E::Alignment);
            }
            sections.push(Section {
                kind,
                identity: id,
                payload,
                offset: end,
            });
            pos = next;
        }
        if sections.is_empty() {
            return Err(E::Extent);
        }
        Ok(Self { sections })
    }
    pub fn section(&self, id: &str) -> Result<Section<'a>> {
        self.sections
            .iter()
            .find(|s| s.identity == id)
            .copied()
            .ok_or(E::MissingReference)
    }
    pub fn directory(&self, limits: Limits) -> Result<Value> {
        let mut dirs = self
            .sections
            .iter()
            .filter(|s| s.kind == SectionKind::Directory);
        let s = dirs.next().ok_or(E::Metadata)?;
        if dirs.next().is_some() {
            return Err(E::Metadata);
        }
        bounded_json(s.payload, limits)
    }
    /// Required references must resolve locally. Only the source frame references
    /// in a processed directory are external; an arbitrary missing Ref is rejected.
    pub fn references(&self, dir: &Value, mode: ReferenceMode) -> Result<Vec<Reference>> {
        let mut refs = Vec::new();
        collect_refs(dir, "", &mut refs, 0, &mut 0, &mut 0)?;
        for r in &mut refs {
            if self.section(&r.identity).is_ok() {
                r.location = ReferenceLocation::Local;
            } else if mode == ReferenceMode::Processed && source_frame_ref(&r.pointer) {
                r.location = ReferenceLocation::ExternalOriginal;
            } else {
                return Err(E::MissingReference);
            }
        }
        #[cfg(feature = "test-harness")]
        crate::capacity_observations::metadata_references(&refs);
        Ok(refs)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceMode {
    Raw,
    Processed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceLocation {
    Local,
    ExternalOriginal,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Reference {
    pub pointer: String,
    pub identity: String,
    pub location: ReferenceLocation,
}
impl std::fmt::Debug for Reference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reference")
            .field("location", &self.location)
            .finish()
    }
}
fn source_frame_ref(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    parts.len() == 6
        && parts[1] == "picture"
        && parts[2] == "frameArray"
        && parts[3].parse::<usize>().is_ok()
        && parts[4] == "frame"
        && ["metadataRef", "privateMetadataRef", "imageRef"].contains(&parts[5])
}
fn collect_refs(
    v: &Value,
    path: &str,
    out: &mut Vec<Reference>,
    depth: usize,
    nodes: &mut usize,
    reference_bytes: &mut usize,
) -> Result<()> {
    *nodes += 1;
    if depth > 64 || *nodes > 100_000 || path.len() > 4096 {
        return Err(E::Budget);
    }
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                if path
                    .len()
                    .saturating_add(k.len().saturating_mul(2))
                    .saturating_add(1)
                    > 4096
                {
                    return Err(E::Budget);
                }
                let p = format!("{path}/{}", k.replace('~', "~0").replace('/', "~1"));
                if k.ends_with("Ref") {
                    let id = v.as_str().ok_or(E::Metadata)?;
                    if !identity(id) {
                        return Err(E::Identity);
                    }
                    *reference_bytes = reference_bytes
                        .saturating_add(p.len())
                        .saturating_add(id.len());
                    if *reference_bytes > 2 * 1024 * 1024 {
                        return Err(E::Budget);
                    }
                    out.push(Reference {
                        pointer: p,
                        identity: id.into(),
                        location: ReferenceLocation::Local,
                    });
                } else {
                    collect_refs(v, &p, out, depth + 1, nodes, reference_bytes)?;
                }
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                collect_refs(
                    v,
                    &format!("{path}/{i}"),
                    out,
                    depth + 1,
                    nodes,
                    reference_bytes,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Lexical preflight bounds recursion and element count before serde allocates.
/// Object duplicate keys are rejected instead of silently discarding originals.
pub fn bounded_json(bytes: &[u8], limits: Limits) -> Result<Value> {
    if bytes.len() > limits.json_bytes {
        return Err(E::Budget);
    }
    let mut depth = 0usize;
    let mut nodes = 0usize;
    let mut string = false;
    let mut escape = false;
    for &b in bytes {
        if string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                string = false;
            }
            continue;
        }
        match b {
            b'"' => {
                string = true;
                nodes += 1;
            }
            b'{' | b'[' => {
                depth += 1;
                nodes += 1;
                if depth > limits.json_depth.min(64) {
                    return Err(E::Budget);
                }
            }
            b'}' | b']' => {
                depth = depth.checked_sub(1).ok_or(E::Json)?;
            }
            b',' | b':' => nodes += 1,
            _ => {}
        }
        if nodes > limits.json_nodes {
            return Err(E::Budget);
        }
    }
    if string || depth != 0 {
        return Err(E::Json);
    }
    // A visitor rejects duplicate keys at every nesting level.
    use serde::Deserialize;
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let value = UniqueValue::deserialize(&mut de).map_err(|_| E::Json)?.0;
    de.end().map_err(|_| E::Json)?;
    Ok(value)
}
struct UniqueValue(Value);
impl<'de> serde::Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                v: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("nonfinite"))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                v: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element::<UniqueValue>()? {
                    v.push(x.0);
                }
                Ok(UniqueValue(Value::Array(v)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut v = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if v.contains_key(&k) {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                    v.insert(k, a.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(v)))
            }
        }
        d.deserialize_any(Visitor)
    }
}

/// Typed originals retain their complete original JSON and original scalar types.
#[derive(Clone)]
pub struct Recipe {
    pub original: Value,
    pub fields: BTreeMap<String, Value>,
}
impl Recipe {
    pub fn from_value(original: Value) -> Result<Self> {
        let fields = original
            .as_object()
            .ok_or(E::Metadata)?
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Ok(Self { original, fields })
    }
}
