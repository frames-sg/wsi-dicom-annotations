use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::builtins::{builtin_general_pathology, builtin_tumor_compatibility};
use super::color::srgb_to_dicom_cielab;
use super::concept_key::{annotation_class_concept_key, code_identity, AnnotationClassConceptKey};
use super::{MAX_CLASSES, MAX_FINDING_SITES, SCHEME_SCHEMA_VERSION};
use crate::annotations::model::DicomCode;
use crate::annotations::semantic_digest::{finish, update_code, update_text};
use crate::{Error, Result};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationClass {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) geometry: AnnotationClassGeometry,
    pub(super) display_color: [u8; 3],
    pub(super) category: DicomCode,
    pub(super) property_type: DicomCode,
    pub(super) property_type_modifiers: Vec<DicomCode>,
    pub(super) concept_key: AnnotationClassConceptKey,
}

impl AnnotationClass {
    pub(super) fn new(
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
    pub(super) schema_version: u32,
    pub(super) id: String,
    pub(super) version: u32,
    pub(super) display_name: String,
    pub(super) classes: Vec<AnnotationClass>,
    pub(super) finding_sites: Vec<DicomCode>,
    pub(super) content_digest: String,
}

impl AnnotationScheme {
    pub(super) fn new(
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
            if !finding_site_keys.insert(code_identity(site)) {
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
