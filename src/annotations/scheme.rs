use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

use super::json::validate_unique_object_keys;
use super::model::{DicomCode, DicomCodeValueKind};
use super::profile::ProfileCode;
use super::semantic_digest::{finish, update_code, update_text};

const SCHEME_SCHEMA_VERSION: u32 = 1;
const MAX_SCHEME_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLASSES: usize = 512;
const MAX_FINDING_SITES: usize = 128;
const DCMR_UID: &str = "1.2.840.10008.8.1.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnnotationClassGeometry {
    Region,
    Point,
}

impl AnnotationClassGeometry {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Region => "Region",
            Self::Point => "Point",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AnnotationClassConceptKey(String);

impl AnnotationClassConceptKey {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AnnotationClassConceptKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationClass {
    id: String,
    label: String,
    geometry: AnnotationClassGeometry,
    display_color: [u8; 3],
    category: DicomCode,
    property_type: DicomCode,
    property_type_modifiers: Vec<DicomCode>,
    concept_key: AnnotationClassConceptKey,
}

impl AnnotationClass {
    fn new(
        id: String,
        label: String,
        geometry: AnnotationClassGeometry,
        display_color: [u8; 3],
        category: DicomCode,
        property_type: DicomCode,
        property_type_modifiers: Vec<DicomCode>,
    ) -> Result<Self> {
        validate_identifier("class id", &id, 64)?;
        validate_display_text("class label", &label, 128)?;
        if property_type_modifiers.len() > 32 {
            return Err(Error::InvalidInput(
                "an annotation class cannot define more than 32 property modifiers".into(),
            ));
        }
        let concept_key = annotation_class_concept_key(
            geometry,
            &category,
            &property_type,
            &property_type_modifiers,
        );
        Ok(Self {
            id,
            label,
            geometry,
            display_color,
            category,
            property_type,
            property_type_modifiers,
            concept_key,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn geometry(&self) -> AnnotationClassGeometry {
        self.geometry
    }

    #[must_use]
    pub const fn display_color(&self) -> [u8; 3] {
        self.display_color
    }

    #[must_use]
    pub fn recommended_display_cielab(&self) -> [u16; 3] {
        srgb_to_dicom_cielab(self.display_color)
    }

    #[must_use]
    pub fn category(&self) -> &DicomCode {
        &self.category
    }

    #[must_use]
    pub fn property_type(&self) -> &DicomCode {
        &self.property_type
    }

    #[must_use]
    pub fn property_type_modifiers(&self) -> &[DicomCode] {
        &self.property_type_modifiers
    }

    #[must_use]
    pub fn concept_key(&self) -> &AnnotationClassConceptKey {
        &self.concept_key
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationScheme {
    schema_version: u32,
    id: String,
    version: u32,
    display_name: String,
    classes: Vec<AnnotationClass>,
    finding_sites: Vec<DicomCode>,
    content_digest: String,
}

impl Serialize for AnnotationScheme {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        RawAnnotationScheme::from_scheme(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AnnotationScheme {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        RawAnnotationScheme::deserialize(deserializer)?
            .into_scheme()
            .map_err(serde::de::Error::custom)
    }
}

impl AnnotationScheme {
    pub fn from_json(json: &[u8]) -> Result<Self> {
        if json.len() > MAX_SCHEME_BYTES {
            return Err(Error::InvalidInput(
                "annotation scheme exceeds the 4 MiB input limit".into(),
            ));
        }
        validate_unique_object_keys(json, "annotation scheme")?;
        let raw: RawAnnotationScheme = serde_json::from_slice(json).map_err(|error| {
            Error::InvalidInput(format!("annotation scheme is not valid JSON: {error}"))
        })?;
        raw.into_scheme()
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        serde_json::to_vec_pretty(&RawAnnotationScheme::from_scheme(self)).map_err(|error| {
            Error::InvalidInput(format!(
                "annotation scheme could not be serialized: {error}"
            ))
        })
    }

    fn new(
        id: String,
        version: u32,
        display_name: String,
        classes: Vec<AnnotationClass>,
        finding_sites: Vec<DicomCode>,
    ) -> Result<Self> {
        validate_identifier("scheme id", &id, 128)?;
        if version == 0 {
            return Err(Error::InvalidInput(
                "annotation scheme version must be positive".into(),
            ));
        }
        validate_display_text("scheme display name", &display_name, 128)?;
        if classes.is_empty() || classes.len() > MAX_CLASSES {
            return Err(Error::InvalidInput(format!(
                "annotation scheme must define 1..={MAX_CLASSES} classes"
            )));
        }
        if finding_sites.len() > MAX_FINDING_SITES {
            return Err(Error::InvalidInput(format!(
                "annotation scheme cannot define more than {MAX_FINDING_SITES} finding sites"
            )));
        }

        let mut class_ids = HashSet::with_capacity(classes.len());
        let mut concepts = HashSet::with_capacity(classes.len());
        for class in &classes {
            if !class_ids.insert(class.id.clone()) {
                return Err(Error::InvalidInput(format!(
                    "annotation scheme contains duplicate class id {:?}",
                    class.id
                )));
            }
            if !concepts.insert(class.concept_key.clone()) {
                return Err(Error::InvalidInput(format!(
                    "annotation scheme contains duplicate class concept {}",
                    class.concept_key
                )));
            }
        }

        let mut finding_site_keys = HashSet::with_capacity(finding_sites.len());
        for site in &finding_sites {
            let key = code_identity(site);
            if !finding_site_keys.insert(key) {
                return Err(Error::InvalidInput(
                    "annotation scheme contains duplicate finding-site concepts".into(),
                ));
            }
        }

        let mut scheme = Self {
            schema_version: SCHEME_SCHEMA_VERSION,
            id,
            version,
            display_name,
            classes,
            finding_sites,
            content_digest: String::new(),
        };
        scheme.content_digest = scheme.calculate_content_digest();
        Ok(scheme)
    }

    #[must_use]
    pub fn general_pathology_v1() -> Self {
        builtin_general_pathology().expect("the built-in General Pathology scheme is valid")
    }

    #[must_use]
    pub fn tumor_mask_compatibility_v1() -> Self {
        builtin_tumor_compatibility().expect("the built-in tumor compatibility scheme is valid")
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub fn classes(&self) -> &[AnnotationClass] {
        &self.classes
    }

    #[must_use]
    pub fn finding_sites(&self) -> &[DicomCode] {
        &self.finding_sites
    }

    #[must_use]
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }

    #[must_use]
    pub fn class(&self, id: &str) -> Option<&AnnotationClass> {
        self.classes.iter().find(|class| class.id == id)
    }

    #[must_use]
    pub fn class_for_concept(
        &self,
        concept: &AnnotationClassConceptKey,
    ) -> Option<&AnnotationClass> {
        self.classes
            .iter()
            .find(|class| class.concept_key == *concept)
    }

    fn calculate_content_digest(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"frames-annotation-scheme-v1\0");
        digest.update(self.schema_version.to_le_bytes());
        update_text(&mut digest, &self.id);
        digest.update(self.version.to_le_bytes());
        update_text(&mut digest, &self.display_name);
        digest.update((self.classes.len() as u64).to_le_bytes());
        for class in &self.classes {
            update_text(&mut digest, &class.id);
            update_text(&mut digest, &class.label);
            digest.update([match class.geometry {
                AnnotationClassGeometry::Region => 0,
                AnnotationClassGeometry::Point => 1,
            }]);
            digest.update(class.display_color);
            update_code(&mut digest, &class.category);
            update_code(&mut digest, &class.property_type);
            digest.update((class.property_type_modifiers.len() as u64).to_le_bytes());
            for modifier in &class.property_type_modifiers {
                update_code(&mut digest, modifier);
            }
        }
        digest.update((self.finding_sites.len() as u64).to_le_bytes());
        for site in &self.finding_sites {
            update_code(&mut digest, site);
        }
        finish(digest)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAnnotationScheme {
    schema_version: u32,
    scheme_id: String,
    scheme_version: u32,
    display_name: String,
    classes: Vec<RawAnnotationClass>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    finding_sites: Vec<ProfileCode>,
}

impl RawAnnotationScheme {
    fn into_scheme(self) -> Result<AnnotationScheme> {
        if self.schema_version != SCHEME_SCHEMA_VERSION {
            return Err(Error::Unsupported(format!(
                "annotation scheme schema version {} is not supported",
                self.schema_version
            )));
        }
        if self.classes.len() > MAX_CLASSES {
            return Err(Error::InvalidInput(format!(
                "annotation scheme cannot define more than {MAX_CLASSES} classes"
            )));
        }
        let classes = self
            .classes
            .into_iter()
            .map(RawAnnotationClass::into_class)
            .collect::<Result<Vec<_>>>()?;
        let finding_sites = self
            .finding_sites
            .into_iter()
            .map(ProfileCode::into_code)
            .collect::<Result<Vec<_>>>()?;
        AnnotationScheme::new(
            self.scheme_id,
            self.scheme_version,
            self.display_name,
            classes,
            finding_sites,
        )
    }

    fn from_scheme(scheme: &AnnotationScheme) -> Self {
        Self {
            schema_version: scheme.schema_version,
            scheme_id: scheme.id.clone(),
            scheme_version: scheme.version,
            display_name: scheme.display_name.clone(),
            classes: scheme
                .classes
                .iter()
                .map(RawAnnotationClass::from_class)
                .collect(),
            finding_sites: scheme
                .finding_sites
                .iter()
                .map(ProfileCode::from_code)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAnnotationClass {
    id: String,
    label: String,
    geometry: AnnotationClassGeometry,
    display_color: String,
    category: ProfileCode,
    property_type: ProfileCode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    property_type_modifiers: Vec<ProfileCode>,
}

impl RawAnnotationClass {
    fn into_class(self) -> Result<AnnotationClass> {
        AnnotationClass::new(
            self.id,
            self.label,
            self.geometry,
            parse_rgb(&self.display_color)?,
            self.category.into_code()?,
            self.property_type.into_code()?,
            self.property_type_modifiers
                .into_iter()
                .map(ProfileCode::into_code)
                .collect::<Result<Vec<_>>>()?,
        )
    }

    fn from_class(class: &AnnotationClass) -> Self {
        Self {
            id: class.id.clone(),
            label: class.label.clone(),
            geometry: class.geometry,
            display_color: format!(
                "#{:02X}{:02X}{:02X}",
                class.display_color[0], class.display_color[1], class.display_color[2]
            ),
            category: ProfileCode::from_code(&class.category),
            property_type: ProfileCode::from_code(&class.property_type),
            property_type_modifiers: class
                .property_type_modifiers
                .iter()
                .map(ProfileCode::from_code)
                .collect(),
        }
    }
}

fn validate_identifier(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.is_empty()
        || value.len() > max_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(Error::InvalidInput(format!(
            "{name} must be 1..={max_bytes} ASCII letters, digits, dots, underscores, or hyphens"
        )));
    }
    Ok(())
}

fn validate_display_text(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > max_bytes
        || value.chars().any(char::is_control)
        || value.contains(['\\', '\0'])
    {
        return Err(Error::InvalidInput(format!(
            "{name} must be nonempty, at most {max_bytes} bytes, and contain no control characters"
        )));
    }
    Ok(())
}

fn parse_rgb(value: &str) -> Result<[u8; 3]> {
    if value.len() != 7 || !value.starts_with('#') {
        return Err(Error::InvalidInput(
            "annotation class display_color must use #RRGGBB".into(),
        ));
    }
    let parse = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&value[range], 16).map_err(|_| {
            Error::InvalidInput("annotation class display_color must use #RRGGBB".into())
        })
    };
    Ok([parse(1..3)?, parse(3..5)?, parse(5..7)?])
}

/// Builds the stable semantic identity used to match imported concepts to a
/// controlled annotation class.
///
/// Display meanings and other presentation metadata are deliberately ignored.
#[must_use]
pub fn annotation_class_concept_key(
    geometry: AnnotationClassGeometry,
    category: &DicomCode,
    property_type: &DicomCode,
    modifiers: &[DicomCode],
) -> AnnotationClassConceptKey {
    let mut digest = Sha256::new();
    digest.update(b"frames-annotation-class-concept-v1\0");
    digest.update([match geometry {
        AnnotationClassGeometry::Region => 0,
        AnnotationClassGeometry::Point => 1,
    }]);
    update_concept_code(&mut digest, category);
    update_concept_code(&mut digest, property_type);
    let mut modifiers = modifiers.iter().map(code_identity).collect::<Vec<_>>();
    modifiers.sort();
    digest.update((modifiers.len() as u64).to_le_bytes());
    for modifier in modifiers {
        update_text(&mut digest, &modifier);
    }
    AnnotationClassConceptKey(format!("sha256:{:x}", digest.finalize()))
}

fn update_concept_code(digest: &mut Sha256, code: &DicomCode) {
    update_text(digest, &code_identity(code));
}

fn code_identity(code: &DicomCode) -> String {
    let kind = match code.value_kind() {
        DicomCodeValueKind::Short => "short",
        DicomCodeValueKind::Long => "long",
        DicomCodeValueKind::Urn => "urn",
    };
    format!(
        "{kind}\u{1f}{}\u{1f}{}\u{1f}{}",
        code.scheme(),
        code.value(),
        code.coding_scheme_version().unwrap_or("")
    )
}

fn builtin_general_pathology() -> Result<AnnotationScheme> {
    let category = |value, meaning| {
        contextual_code(
            value,
            "SCT",
            meaning,
            "7150",
            "20260319",
            "1.2.840.10008.6.1.496",
        )
    };
    let tissue_type = |value, meaning| {
        contextual_code(
            value,
            "SCT",
            meaning,
            "7166",
            "20240611",
            "1.2.840.10008.6.1.963",
        )
    };
    let lesion_type = |value, meaning| {
        contextual_code(
            value,
            "SCT",
            meaning,
            "7159",
            "20240611",
            "1.2.840.10008.6.1.505",
        )
    };
    let microscopy_type = |value, meaning| {
        contextual_code(
            value,
            "SCT",
            meaning,
            "8135",
            "20210712",
            "1.2.840.10008.6.1.1365",
        )
    };
    let class = |id: &str, label: &str, geometry, color, category, property_type| {
        AnnotationClass::new(
            id.into(),
            label.into(),
            geometry,
            color,
            category,
            property_type,
            Vec::new(),
        )
    };
    let cell_category = || DicomCode::new("4421005", "SCT", "Cell Structure");
    AnnotationScheme::new(
        "org.frames.general-pathology".into(),
        1,
        "General Pathology".into(),
        vec![
            class(
                "tissue",
                "Tissue",
                AnnotationClassGeometry::Region,
                [0x99, 0x99, 0x99],
                category("85756007", "Tissue")?,
                tissue_type("85756007", "Tissue")?,
            )?,
            class(
                "neoplasm",
                "Neoplasm",
                AnnotationClassGeometry::Region,
                [0xD5, 0x5E, 0x00],
                category("49755003", "Morphologically Abnormal Structure")?,
                lesion_type("108369006", "Neoplasm")?,
            )?,
            class(
                "necrosis",
                "Necrosis",
                AnnotationClassGeometry::Region,
                [0xE6, 0x9F, 0x00],
                category("49755003", "Morphologically Abnormal Structure")?,
                lesion_type("6574001", "Necrosis")?,
            )?,
            class(
                "inflammation",
                "Inflammation",
                AnnotationClassGeometry::Region,
                [0xF0, 0xE4, 0x42],
                category("49755003", "Morphologically Abnormal Structure")?,
                lesion_type("409774005", "Inflammation")?,
            )?,
            class(
                "stroma",
                "Stroma / connective tissue",
                AnnotationClassGeometry::Region,
                [0x00, 0x9E, 0x73],
                category("85756007", "Tissue")?,
                tissue_type("21793004", "Connective tissue")?,
            )?,
            class(
                "unusable-tissue",
                "Unusable tissue",
                AnnotationClassGeometry::Region,
                [0xCC, 0x79, 0xA7],
                category("263496004", "Quality")?,
                contextual_code(
                    "131502",
                    "DCM",
                    "Unusable tissue",
                    "7164",
                    "20260319",
                    "1.2.840.10008.6.1.1568",
                )?,
            )?,
            class(
                "cell",
                "Cell",
                AnnotationClassGeometry::Point,
                [0x56, 0xB4, 0xE9],
                cell_category()?,
                microscopy_type("4421005", "Cell")?,
            )?,
            class(
                "nucleus",
                "Nucleus",
                AnnotationClassGeometry::Point,
                [0x00, 0x72, 0xB2],
                cell_category()?,
                microscopy_type("84640000", "Nucleus")?,
            )?,
        ],
        Vec::new(),
    )
}

fn builtin_tumor_compatibility() -> Result<AnnotationScheme> {
    AnnotationScheme::new(
        "org.frames.tumor-mask-compatibility".into(),
        1,
        "Tumor Mask Compatibility".into(),
        vec![
            AnnotationClass::new(
                "viable-tumor".into(),
                "Viable tumor".into(),
                AnnotationClassGeometry::Region,
                [0x00, 0x9E, 0x73],
                contextual_code(
                    "49755003",
                    "SCT",
                    "Morphologically Abnormal Structure",
                    "7150",
                    "20260319",
                    "1.2.840.10008.6.1.496",
                )?,
                DicomCode::new("VIABLE_TUMOR", "99FRAMES", "Viable tumor")?,
                Vec::new(),
            )?,
            AnnotationClass::new(
                "cell".into(),
                "Cell".into(),
                AnnotationClassGeometry::Point,
                [0x56, 0xB4, 0xE9],
                DicomCode::new("49755003", "SCT", "Morphologically Abnormal Structure")?,
                DicomCode::new("4421005", "SCT", "Cell")?,
                Vec::new(),
            )?,
        ],
        Vec::new(),
    )
}

fn contextual_code(
    value: &str,
    scheme: &str,
    meaning: &str,
    context_identifier: &str,
    context_group_version: &str,
    context_uid: &str,
) -> Result<DicomCode> {
    DicomCode::new(value, scheme, meaning)?
        .with_context_identifier(context_identifier, "DCMR", context_group_version)?
        .with_context_uid(context_uid, DCMR_UID)
}

#[must_use]
pub fn srgb_to_dicom_cielab(rgb: [u8; 3]) -> [u16; 3] {
    let linear = rgb.map(|value| srgb_channel_to_linear(f64::from(value) / 255.0));
    let xyz_d65 = [
        0.412_456_4 * linear[0] + 0.357_576_1 * linear[1] + 0.180_437_5 * linear[2],
        0.212_672_9 * linear[0] + 0.715_152_2 * linear[1] + 0.072_175 * linear[2],
        0.019_333_9 * linear[0] + 0.119_192 * linear[1] + 0.950_304_1 * linear[2],
    ];
    let xyz = [
        1.047_929_8 * xyz_d65[0] + 0.022_946_8 * xyz_d65[1] - 0.050_192_2 * xyz_d65[2],
        0.029_627_8 * xyz_d65[0] + 0.990_434_5 * xyz_d65[1] - 0.017_073_8 * xyz_d65[2],
        -0.009_243 * xyz_d65[0] + 0.015_055_2 * xyz_d65[1] + 0.751_874_3 * xyz_d65[2],
    ];
    let f = [
        lab_f(xyz[0] / 0.964_2),
        lab_f(xyz[1]),
        lab_f(xyz[2] / 0.824_9),
    ];
    let l = 116.0 * f[1] - 16.0;
    let a = 500.0 * (f[0] - f[1]);
    let b = 200.0 * (f[1] - f[2]);
    [
        encode_lab_component(l, 100.0, 0.0),
        encode_lab_component(a, 255.0, 128.0),
        encode_lab_component(b, 255.0, 128.0),
    ]
}

#[must_use]
pub fn dicom_cielab_to_srgb(lab: [u16; 3]) -> [u8; 3] {
    let l = f64::from(lab[0]) * 100.0 / 65_535.0;
    let a = f64::from(lab[1]) * 255.0 / 65_535.0 - 128.0;
    let b = f64::from(lab[2]) * 255.0 / 65_535.0 - 128.0;
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let xyz_d50 = [
        0.964_2 * lab_f_inverse(fx),
        lab_f_inverse(fy),
        0.824_9 * lab_f_inverse(fz),
    ];
    let xyz = [
        0.955_473_4 * xyz_d50[0] - 0.023_098_5 * xyz_d50[1] + 0.063_259_3 * xyz_d50[2],
        -0.028_369_7 * xyz_d50[0] + 1.009_995_5 * xyz_d50[1] + 0.021_041_4 * xyz_d50[2],
        0.012_314 * xyz_d50[0] - 0.020_507_7 * xyz_d50[1] + 1.330_365_9 * xyz_d50[2],
    ];
    let linear = [
        3.240_454_2 * xyz[0] - 1.537_138_5 * xyz[1] - 0.498_531_4 * xyz[2],
        -0.969_266 * xyz[0] + 1.876_010_8 * xyz[1] + 0.041_556 * xyz[2],
        0.055_643_4 * xyz[0] - 0.204_025_9 * xyz[1] + 1.057_225_2 * xyz[2],
    ];
    linear.map(|value| (linear_channel_to_srgb(value).clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn srgb_channel_to_linear(value: f64) -> f64 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_channel_to_srgb(value: f64) -> f64 {
    if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn lab_f(value: f64) -> f64 {
    const EPSILON: f64 = 216.0 / 24_389.0;
    const KAPPA: f64 = 24_389.0 / 27.0;
    if value > EPSILON {
        value.cbrt()
    } else {
        (KAPPA * value + 16.0) / 116.0
    }
}

fn lab_f_inverse(value: f64) -> f64 {
    const EPSILON: f64 = 216.0 / 24_389.0;
    const KAPPA: f64 = 24_389.0 / 27.0;
    let cube = value * value * value;
    if cube > EPSILON {
        cube
    } else {
        (116.0 * value - 16.0) / KAPPA
    }
}

fn encode_lab_component(value: f64, range: f64, offset: f64) -> u16 {
    (((value + offset) / range).clamp(0.0, 1.0) * 65_535.0).round() as u16
}
