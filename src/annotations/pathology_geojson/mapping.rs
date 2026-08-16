use std::collections::BTreeMap;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

use super::super::model::{AlgorithmIdentification, DicomCode, GenerationType};
use super::super::profile::{profile_codes, ProfileAlgorithm, ProfileCode};
use crate::annotations::json::validate_unique_object_keys;
use crate::annotations::semantic_digest::{update_algorithm, update_code, update_text};

#[derive(Debug, Clone)]
pub(super) struct LabelSemantics {
    pub(super) category: DicomCode,
    pub(super) property_type: DicomCode,
    pub(super) property_type_modifiers: Vec<DicomCode>,
    pub(super) anatomic_regions: Vec<DicomCode>,
    pub(super) primary_anatomic_structures: Vec<DicomCode>,
    pub(super) generation_type: GenerationType,
    pub(super) algorithms: Vec<AlgorithmIdentification>,
    pub(super) all_optical_paths: bool,
    pub(super) optical_paths: Vec<String>,
    pub(super) all_z_planes: bool,
    pub(super) z_coordinates_mm: Vec<f64>,
    pub(super) color: [u16; 3],
    pub(super) segment_label: String,
}

impl LabelSemantics {
    pub(super) fn update_digest(&self, digest: &mut Sha256) {
        update_code(digest, &self.category);
        update_code(digest, &self.property_type);
        for codes in [
            self.property_type_modifiers.as_slice(),
            self.anatomic_regions.as_slice(),
            self.primary_anatomic_structures.as_slice(),
        ] {
            digest.update((codes.len() as u64).to_le_bytes());
            for code in codes {
                update_code(digest, code);
            }
        }
        digest.update([self.generation_type as u8]);
        for algorithm in &self.algorithms {
            update_algorithm(digest, algorithm);
        }
        digest.update([u8::from(self.all_optical_paths)]);
        for path in &self.optical_paths {
            update_text(digest, path);
        }
        digest.update([u8::from(self.all_z_planes)]);
        for coordinate in &self.z_coordinates_mm {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
        for component in self.color {
            digest.update(component.to_le_bytes());
        }
        update_text(digest, &self.segment_label);
    }
}

#[derive(Debug, Clone)]
pub(super) struct MeasurementMapping {
    pub(super) concept: DicomCode,
    pub(super) unit: DicomCode,
}

#[derive(Debug, Clone)]
pub(super) struct QualitativeMapping {
    pub(super) concept: DicomCode,
    pub(super) values: BTreeMap<String, DicomCode>,
}

#[derive(Debug, Clone)]
pub(super) struct StructuredReportMapping {
    pub(super) report_title: DicomCode,
    pub(super) procedures_reported: Vec<DicomCode>,
}

impl StructuredReportMapping {
    pub(super) fn update_digest(&self, digest: &mut Sha256) {
        update_code(digest, &self.report_title);
        digest.update((self.procedures_reported.len() as u64).to_le_bytes());
        for procedure in &self.procedures_reported {
            update_code(digest, procedure);
        }
    }
}

#[derive(Debug)]
pub(super) struct Mapping {
    pub(super) labels: BTreeMap<String, LabelSemantics>,
    pub(super) measurements: BTreeMap<String, MeasurementMapping>,
    pub(super) qualitative_evaluations: BTreeMap<String, QualitativeMapping>,
    pub(super) sr: Option<StructuredReportMapping>,
}

impl Mapping {
    pub(super) fn from_json(json: &[u8]) -> Result<Self> {
        validate_unique_object_keys(json, "mapping profile")?;
        let raw: RawMapping = serde_json::from_slice(json).map_err(|error| {
            Error::InvalidInput(format!("mapping profile is not valid JSON: {error}"))
        })?;
        if raw.schema_version != 1 {
            return Err(Error::Unsupported(format!(
                "mapping schema version {} is not supported",
                raw.schema_version
            )));
        }
        if raw.labels.is_empty() {
            return Err(Error::InvalidInput(
                "mapping profile must define at least one label".into(),
            ));
        }
        let labels = raw
            .labels
            .into_iter()
            .map(|(source, label)| {
                if source.trim().is_empty() {
                    return Err(Error::InvalidInput(
                        "mapping label source classification cannot be empty".into(),
                    ));
                }
                Ok((source, label.into_semantics()?))
            })
            .collect::<Result<_>>()?;
        let measurements = raw
            .measurements
            .into_iter()
            .map(|(name, mapping)| {
                validate_mapping_key("measurement", &name)?;
                Ok((
                    name,
                    MeasurementMapping {
                        concept: mapping.concept.into_code()?,
                        unit: mapping.unit.into_code()?,
                    },
                ))
            })
            .collect::<Result<_>>()?;
        let qualitative_evaluations = raw
            .qualitative_evaluations
            .into_iter()
            .map(|(key, mapping)| {
                validate_mapping_key("qualitative evaluation", &key)?;
                if mapping.values.is_empty() {
                    return Err(Error::InvalidInput(format!(
                        "qualitative evaluation {key:?} has no source-value mappings"
                    )));
                }
                let values = mapping
                    .values
                    .into_iter()
                    .map(|(value, code)| {
                        validate_mapping_key("qualitative source value", &value)?;
                        Ok((value, code.into_code()?))
                    })
                    .collect::<Result<_>>()?;
                Ok((
                    key,
                    QualitativeMapping {
                        concept: mapping.concept.into_code()?,
                        values,
                    },
                ))
            })
            .collect::<Result<_>>()?;
        let sr = raw.sr.map(RawStructuredReport::into_mapping).transpose()?;
        Ok(Self {
            labels,
            measurements,
            qualitative_evaluations,
            sr,
        })
    }
}

fn validate_mapping_key(kind: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(Error::InvalidInput(format!(
            "{kind} mapping key cannot be empty or contain NUL"
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMapping {
    schema_version: u32,
    labels: BTreeMap<String, RawLabel>,
    #[serde(default)]
    measurements: BTreeMap<String, RawMeasurement>,
    #[serde(default)]
    qualitative_evaluations: BTreeMap<String, RawQualitative>,
    #[serde(default)]
    sr: Option<RawStructuredReport>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLabel {
    category: ProfileCode,
    property_type: ProfileCode,
    #[serde(default)]
    property_type_modifiers: Vec<ProfileCode>,
    #[serde(default)]
    anatomic_regions: Vec<ProfileCode>,
    #[serde(default)]
    primary_anatomic_structures: Vec<ProfileCode>,
    generation_type: RawGenerationType,
    #[serde(default)]
    algorithm: Option<ProfileAlgorithm>,
    #[serde(default)]
    applicability: RawApplicability,
    recommended_display_cielab: [u16; 3],
    segment_label: String,
}

impl RawLabel {
    fn into_semantics(self) -> Result<LabelSemantics> {
        validate_mapping_key("segment label", &self.segment_label)?;
        if self.segment_label.len() > 64 || self.segment_label.contains('\\') {
            return Err(Error::InvalidInput(
                "segment label must fit DICOM LO and contain no value separator".into(),
            ));
        }
        let generation_type = self.generation_type.into();
        let algorithms = self
            .algorithm
            .map(ProfileAlgorithm::into_algorithm)
            .transpose()?
            .into_iter()
            .collect::<Vec<_>>();
        match generation_type {
            GenerationType::Manual if !algorithms.is_empty() => {
                return Err(Error::InvalidInput(
                    "manual label mapping cannot identify a generation algorithm".into(),
                ));
            }
            GenerationType::Semiautomatic | GenerationType::Automatic if algorithms.is_empty() => {
                return Err(Error::InvalidInput(
                    "automatic and semiautomatic label mappings require algorithm identity".into(),
                ));
            }
            _ => {}
        }
        self.applicability.validate()?;
        Ok(LabelSemantics {
            category: self.category.into_code()?,
            property_type: self.property_type.into_code()?,
            property_type_modifiers: profile_codes(self.property_type_modifiers)?,
            anatomic_regions: profile_codes(self.anatomic_regions)?,
            primary_anatomic_structures: profile_codes(self.primary_anatomic_structures)?,
            generation_type,
            algorithms,
            all_optical_paths: self.applicability.all_optical_paths,
            optical_paths: self.applicability.optical_paths,
            all_z_planes: self.applicability.all_z_planes,
            z_coordinates_mm: self.applicability.z_coordinates_mm,
            color: self.recommended_display_cielab,
            segment_label: self.segment_label,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawApplicability {
    #[serde(default = "default_true")]
    all_optical_paths: bool,
    #[serde(default)]
    optical_paths: Vec<String>,
    #[serde(default = "default_true")]
    all_z_planes: bool,
    #[serde(default)]
    z_coordinates_mm: Vec<f64>,
}

impl Default for RawApplicability {
    fn default() -> Self {
        Self {
            all_optical_paths: true,
            optical_paths: Vec::new(),
            all_z_planes: true,
            z_coordinates_mm: Vec::new(),
        }
    }
}

impl RawApplicability {
    fn validate(&self) -> Result<()> {
        if self.all_optical_paths != self.optical_paths.is_empty() {
            return Err(Error::InvalidInput(
                "optical-path applicability must be either all or a nonempty explicit list".into(),
            ));
        }
        if self.all_z_planes != self.z_coordinates_mm.is_empty() {
            return Err(Error::InvalidInput(
                "Z applicability must be either all or a nonempty explicit coordinate list".into(),
            ));
        }
        if self
            .z_coordinates_mm
            .iter()
            .any(|coordinate| !coordinate.is_finite())
        {
            return Err(Error::InvalidInput(
                "Z applicability coordinates must be finite".into(),
            ));
        }
        Ok(())
    }
}

const fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum RawGenerationType {
    Manual,
    Semiautomatic,
    Automatic,
}

impl From<RawGenerationType> for GenerationType {
    fn from(value: RawGenerationType) -> Self {
        match value {
            RawGenerationType::Manual => Self::Manual,
            RawGenerationType::Semiautomatic => Self::Semiautomatic,
            RawGenerationType::Automatic => Self::Automatic,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMeasurement {
    concept: ProfileCode,
    unit: ProfileCode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQualitative {
    concept: ProfileCode,
    values: BTreeMap<String, ProfileCode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStructuredReport {
    report_title: ProfileCode,
    procedures_reported: Vec<ProfileCode>,
}

impl RawStructuredReport {
    fn into_mapping(self) -> Result<StructuredReportMapping> {
        if self.procedures_reported.is_empty() {
            return Err(Error::InvalidInput(
                "SR mapping requires at least one coded procedure reported".into(),
            ));
        }
        Ok(StructuredReportMapping {
            report_title: self.report_title.into_code()?,
            procedures_reported: profile_codes(self.procedures_reported)?,
        })
    }
}
