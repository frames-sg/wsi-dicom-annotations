use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

use super::geometry::{profile_geometry, ProfiledGeometry, RawGeometry};
use super::mapping::{LabelSemantics, Mapping, MeasurementMapping, QualitativeMapping};
use super::{PathologyCoordinateSpace, PathologyGeometryKind};
use crate::annotations::context::DicomAnnotationContext;
use crate::annotations::dicom_dataset::new_dicom_uid;
use crate::annotations::json::validate_unique_object_keys;
use crate::annotations::model::{
    is_valid_dicom_uid, DiagnosticDisposition, DiagnosticSeverity, DicomCode,
    InteroperabilityDiagnostic,
};
use crate::annotations::semantic_digest::{update_code, update_text};

const MAX_FEATURES: usize = 100_000;

#[derive(Debug)]
pub(super) struct ProfiledFeature {
    pub(super) kind: PathologyGeometryKind,
    pub(super) tracking_id: String,
    pub(super) tracking_uid: String,
    pub(super) name: String,
    pub(super) object_type: Option<String>,
    pub(super) classification: String,
    pub(super) semantics: LabelSemantics,
    pub(super) geometry: ProfiledGeometry,
    pub(super) measurements: Vec<ProfiledMeasurement>,
    pub(super) qualitative_evaluations: Vec<ProfiledQualitative>,
    pub(super) dropped_metadata_keys: Vec<String>,
}

#[derive(Debug)]
pub(super) struct ProfiledMeasurement {
    pub(super) source_name: String,
    pub(super) value: f64,
    pub(super) mapping: MeasurementMapping,
}

#[derive(Debug)]
pub(super) struct ProfiledQualitative {
    pub(super) source_key: String,
    pub(super) concept: DicomCode,
    pub(super) value: DicomCode,
}

impl ProfiledFeature {
    pub(super) fn update_semantic_digest(&self, digest: &mut Sha256) {
        for value in [
            self.tracking_id.as_str(),
            self.tracking_uid.as_str(),
            self.name.as_str(),
            self.object_type.as_deref().unwrap_or(""),
            self.classification.as_str(),
            geometry_kind_label(self.kind),
        ] {
            update_text(digest, value);
        }
        self.semantics.update_digest(digest);
        self.geometry.update_digest(digest);
        for measurement in &self.measurements {
            update_text(digest, &measurement.source_name);
            update_code(digest, &measurement.mapping.concept);
            update_code(digest, &measurement.mapping.unit);
            digest.update(measurement.value.to_bits().to_le_bytes());
        }
        for evaluation in &self.qualitative_evaluations {
            update_text(digest, &evaluation.source_key);
            update_code(digest, &evaluation.concept);
            update_code(digest, &evaluation.value);
        }
        for key in &self.dropped_metadata_keys {
            update_text(digest, key);
        }
    }
}

const fn geometry_kind_label(kind: PathologyGeometryKind) -> &'static str {
    match kind {
        PathologyGeometryKind::Point => "Point",
        PathologyGeometryKind::MultiPoint => "MultiPoint",
        PathologyGeometryKind::LineString => "LineString",
        PathologyGeometryKind::MultiLineString => "MultiLineString",
        PathologyGeometryKind::Polygon => "Polygon",
        PathologyGeometryKind::MultiPolygon => "MultiPolygon",
    }
}

pub(super) fn parse_feature_collection(
    json: &[u8],
    mapping: &Mapping,
    source: &DicomAnnotationContext,
    canonical_source: &DicomAnnotationContext,
    coordinate_space: PathologyCoordinateSpace,
    allow_lossy: bool,
) -> Result<(Vec<ProfiledFeature>, Vec<InteroperabilityDiagnostic>)> {
    validate_unique_object_keys(json, "GeoJSON")?;
    let collection: RawFeatureCollection = serde_json::from_slice(json).map_err(|error| {
        Error::InvalidInput(format!(
            "GeoJSON is malformed or has invalid nesting: {error}"
        ))
    })?;
    if collection.kind != "FeatureCollection" {
        return Err(Error::InvalidInput(
            "GeoJSON root type must be FeatureCollection".into(),
        ));
    }
    validate_bbox(collection.bbox.as_deref(), "FeatureCollection.bbox")?;
    if collection.features.is_empty() || collection.features.len() > MAX_FEATURES {
        return Err(Error::InvalidInput(format!(
            "GeoJSON FeatureCollection must contain 1..={MAX_FEATURES} features"
        )));
    }
    let mut diagnostics = Vec::new();
    let mut coordinate_pairs = 0_usize;
    let mut features = Vec::with_capacity(collection.features.len());
    for (index, raw) in collection.features.into_iter().enumerate() {
        features.push(profile_feature(
            raw,
            index,
            mapping,
            source,
            canonical_source,
            coordinate_space,
            allow_lossy,
            &mut coordinate_pairs,
            &mut diagnostics,
        )?);
    }
    Ok((features, diagnostics))
}

#[allow(clippy::too_many_arguments)]
fn profile_feature(
    raw: RawFeature,
    index: usize,
    mapping: &Mapping,
    source: &DicomAnnotationContext,
    canonical_source: &DicomAnnotationContext,
    coordinate_space: PathologyCoordinateSpace,
    allow_lossy: bool,
    coordinate_pairs: &mut usize,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<ProfiledFeature> {
    let path = format!("features[{index}]");
    if raw.kind != "Feature" {
        return Err(Error::InvalidInput(format!("{path}.type must be Feature")));
    }
    validate_bbox(raw.bbox.as_deref(), &format!("{path}.bbox"))?;
    let properties = profile_properties(
        raw.properties,
        &path,
        mapping,
        coordinate_space,
        allow_lossy,
        diagnostics,
    )?;
    let (tracking_id, tracking_uid) = parse_identity(raw.id, &path)?;
    let (kind, geometry) = profile_geometry(
        raw.geometry,
        &format!("{path}.geometry"),
        source,
        canonical_source,
        coordinate_space,
        coordinate_pairs,
        diagnostics,
    )?;
    let semantics = mapping
        .labels
        .get(&properties.classification)
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "{path} classification {:?} has no label mapping",
                properties.classification
            ))
        })?
        .clone();
    Ok(ProfiledFeature {
        kind,
        tracking_id,
        tracking_uid,
        name: properties
            .name
            .unwrap_or_else(|| semantics.segment_label.clone()),
        object_type: properties.object_type,
        classification: properties.classification,
        semantics,
        geometry,
        measurements: properties.measurements,
        qualitative_evaluations: properties.qualitative_evaluations,
        dropped_metadata_keys: properties.dropped_metadata_keys,
    })
}

struct ProfiledProperties {
    name: Option<String>,
    object_type: Option<String>,
    classification: String,
    measurements: Vec<ProfiledMeasurement>,
    qualitative_evaluations: Vec<ProfiledQualitative>,
    dropped_metadata_keys: Vec<String>,
}

fn profile_properties(
    raw: RawProperties,
    feature_path: &str,
    mapping: &Mapping,
    coordinate_space: PathologyCoordinateSpace,
    allow_lossy: bool,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<ProfiledProperties> {
    let object_type = normalize_object_type(
        raw.object_type,
        raw.object_type_snake,
        feature_path,
        diagnostics,
    )?;
    validate_coordinate_declaration(
        raw.coordinate_space.as_deref(),
        coordinate_space,
        feature_path,
        diagnostics,
    )?;
    let classification = raw.classification.classification_key(feature_path)?;
    let measurements = profile_measurements(raw.measurements, mapping, feature_path)?;
    let mut metadata = raw.metadata;
    for (key, value) in raw.other {
        insert_semantic_property(&mut metadata, key, value, feature_path)?;
    }
    for (key, value) in raw.classification.extras {
        insert_semantic_property(
            &mut metadata,
            format!("classification.{key}"),
            value,
            feature_path,
        )?;
    }
    let (qualitative_evaluations, dropped_metadata_keys) =
        profile_metadata(metadata, mapping, feature_path, allow_lossy, diagnostics)?;
    Ok(ProfiledProperties {
        name: raw.name,
        object_type,
        classification,
        measurements,
        qualitative_evaluations,
        dropped_metadata_keys,
    })
}

fn insert_semantic_property(
    metadata: &mut BTreeMap<String, Value>,
    key: String,
    value: Value,
    feature_path: &str,
) -> Result<()> {
    if metadata.contains_key(&key) {
        return Err(Error::InvalidInput(format!(
            "{feature_path}.properties property {key} is declared more than once"
        )));
    }
    metadata.insert(key, value);
    Ok(())
}

fn normalize_object_type(
    camel: Option<String>,
    snake: Option<String>,
    feature_path: &str,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<Option<String>> {
    if camel.is_some() && snake.is_some() && camel != snake {
        return Err(Error::InvalidInput(format!(
            "{feature_path}.properties objectType and object_type disagree"
        )));
    }
    if camel.is_none() && snake.is_some() {
        diagnostics.push(InteroperabilityDiagnostic::normalized(
            "GEOJSON_OBJECT_TYPE_ALIAS",
            format!("{feature_path}.properties.object_type"),
            "normalized QuPath object_type to objectType",
        ));
    }
    let value = camel.or(snake);
    if value
        .as_deref()
        .is_some_and(|value| !matches!(value, "annotation" | "detection"))
    {
        return Err(Error::InvalidInput(format!(
            "{feature_path}.properties object type must be annotation or detection"
        )));
    }
    Ok(value)
}

fn validate_coordinate_declaration(
    declared: Option<&str>,
    expected: PathologyCoordinateSpace,
    feature_path: &str,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<()> {
    let Some(declared) = declared else {
        return Ok(());
    };
    let (actual, alias) = match declared {
        "level0-pixels" => (PathologyCoordinateSpace::Level0Pixels, false),
        "level-0_pixels" => (PathologyCoordinateSpace::Level0Pixels, true),
        "source-pixels" => (PathologyCoordinateSpace::SourcePixels, false),
        "slide-mm" => (PathologyCoordinateSpace::SlideMillimeters, false),
        other => {
            return Err(Error::InvalidInput(format!(
                "{feature_path}.properties.coordinate_space {other:?} is not supported"
            )));
        }
    };
    if actual != expected {
        return Err(Error::InvalidInput(format!(
            "{feature_path}.properties.coordinate_space declares {declared:?}, but the CLI declares {:?}",
            expected.label()
        )));
    }
    if alias {
        diagnostics.push(InteroperabilityDiagnostic::normalized(
            "GEOJSON_COORDINATE_SPACE_ALIAS",
            format!("{feature_path}.properties.coordinate_space"),
            "normalized viewer coordinate-space alias level-0_pixels to level0-pixels",
        ));
    }
    Ok(())
}

fn profile_measurements(
    raw: BTreeMap<String, Value>,
    mapping: &Mapping,
    feature_path: &str,
) -> Result<Vec<ProfiledMeasurement>> {
    raw.into_iter()
        .map(|(name, value)| {
            let mapping = mapping.measurements.get(&name).ok_or_else(|| {
                Error::InvalidInput(format!(
                    "{feature_path}.properties.measurements has unmapped measurement {name:?}"
                ))
            })?;
            let value = value.as_f64().ok_or_else(|| {
                Error::InvalidInput(format!(
                    "{feature_path}.properties.measurements.{name} must be a finite number"
                ))
            })?;
            if !value.is_finite() {
                return Err(Error::InvalidInput(format!(
                    "{feature_path}.properties.measurements.{name} must be finite"
                )));
            }
            Ok(ProfiledMeasurement {
                source_name: name,
                value,
                mapping: mapping.clone(),
            })
        })
        .collect()
}

fn profile_metadata(
    metadata: BTreeMap<String, Value>,
    mapping: &Mapping,
    feature_path: &str,
    allow_lossy: bool,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<(Vec<ProfiledQualitative>, Vec<String>)> {
    let mut mapped = Vec::new();
    let mut dropped = Vec::new();
    for (key, value) in metadata {
        let Some(qualitative) = mapping.qualitative_evaluations.get(&key) else {
            if !allow_lossy {
                return Err(Error::InvalidInput(format!(
                    "{feature_path}.properties has unmapped property {key}"
                )));
            }
            diagnostics.push(InteroperabilityDiagnostic::new(
                "GEOJSON_PROPERTY_DROPPED",
                DiagnosticSeverity::Warning,
                format!("{feature_path}.properties.{key}"),
                DiagnosticDisposition::WouldDrop,
                "dropped explicitly allowed nonstructural metadata",
            ));
            dropped.push(key);
            continue;
        };
        mapped.push(profile_qualitative(key, value, qualitative, feature_path)?);
    }
    Ok((mapped, dropped))
}

fn profile_qualitative(
    key: String,
    value: Value,
    mapping: &QualitativeMapping,
    feature_path: &str,
) -> Result<ProfiledQualitative> {
    let source_value = scalar_key(&value).ok_or_else(|| {
        Error::InvalidInput(format!(
            "{feature_path}.properties.metadata.{key} must be a string, number, or boolean"
        ))
    })?;
    let value = mapping.values.get(&source_value).ok_or_else(|| {
        Error::InvalidInput(format!(
            "{feature_path}.properties.metadata.{key} value {source_value:?} has no DICOM code mapping"
        ))
    })?;
    Ok(ProfiledQualitative {
        source_key: key,
        concept: mapping.concept.clone(),
        value: value.clone(),
    })
}

fn scalar_key(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn parse_identity(id: Option<Value>, feature_path: &str) -> Result<(String, String)> {
    let Some(id) = id else {
        let uid = new_dicom_uid();
        return Ok((uid.clone(), uid));
    };
    let text = id.as_str().ok_or_else(|| {
        Error::InvalidInput(format!(
            "{feature_path}.id must be a UUID or valid DICOM UID string"
        ))
    })?;
    if let Ok(uuid) = uuid::Uuid::parse_str(text) {
        return Ok((text.to_string(), format!("2.25.{}", uuid.as_u128())));
    }
    if is_valid_dicom_uid(text) {
        return Ok((text.to_string(), text.to_string()));
    }
    Err(Error::InvalidInput(format!(
        "{feature_path}.id is neither a UUID nor a valid DICOM UID"
    )))
}

fn validate_bbox(bbox: Option<&[f64]>, path: &str) -> Result<()> {
    if bbox.is_some_and(|bbox| {
        !matches!(bbox.len(), 4 | 6) || bbox.iter().any(|value| !value.is_finite())
    }) {
        return Err(Error::InvalidInput(format!(
            "{path} must contain four or six finite values"
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFeatureCollection {
    #[serde(rename = "type")]
    kind: String,
    features: Vec<RawFeature>,
    #[serde(default)]
    bbox: Option<Vec<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFeature {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<Value>,
    geometry: RawGeometry,
    properties: RawProperties,
    #[serde(default)]
    bbox: Option<Vec<f64>>,
}

#[derive(Deserialize)]
struct RawProperties {
    #[serde(default, rename = "objectType")]
    object_type: Option<String>,
    #[serde(default)]
    #[serde(rename = "object_type")]
    object_type_snake: Option<String>,
    #[serde(default)]
    name: Option<String>,
    classification: RawClassification,
    #[serde(default)]
    coordinate_space: Option<String>,
    #[serde(default)]
    measurements: BTreeMap<String, Value>,
    #[serde(default)]
    metadata: BTreeMap<String, Value>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct RawClassification {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    names: Option<Vec<String>>,
    #[serde(flatten)]
    extras: BTreeMap<String, Value>,
}

impl RawClassification {
    fn classification_key(&self, feature_path: &str) -> Result<String> {
        let hierarchical = self.names.as_ref().map(|names| names.join("/"));
        if self.names.as_ref().is_some_and(|names| {
            names.is_empty() || names.iter().any(|name| name.trim().is_empty())
        }) {
            return Err(Error::InvalidInput(format!(
                "{feature_path}.properties.classification.names must be a nonempty path"
            )));
        }
        if let (Some(name), Some(names)) = (&self.name, &self.names) {
            if names.last() != Some(name) {
                return Err(Error::InvalidInput(format!(
                    "{feature_path}.properties.classification name and names disagree"
                )));
            }
        }
        hierarchical.or_else(|| self.name.clone()).ok_or_else(|| {
            Error::InvalidInput(format!(
                "{feature_path}.properties.classification requires name or names"
            ))
        })
    }
}
