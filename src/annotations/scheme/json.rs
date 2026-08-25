use serde::{Deserialize, Serialize};

use super::color::parse_rgb;
use super::model::{AnnotationClass, AnnotationClassGeometry, AnnotationScheme};
use super::{MAX_CLASSES, MAX_SCHEME_BYTES, SCHEME_SCHEMA_VERSION};
use crate::annotations::json::validate_unique_object_keys;
use crate::annotations::profile::ProfileCode;
use crate::{Error, Result};

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
