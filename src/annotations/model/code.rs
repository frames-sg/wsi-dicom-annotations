use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const MAX_SHORT_CODE_VALUE_BYTES: usize = 16;
const MAX_SCHEME_BYTES: usize = 16;
const MAX_CODE_MEANING_BYTES: usize = 64;
pub(super) const MAX_LONG_TEXT_BYTES: usize = 10_240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DicomCodeValueKind {
    Short,
    Long,
    Urn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DicomCode {
    value: String,
    value_kind: DicomCodeValueKind,
    scheme: String,
    coding_scheme_version: Option<String>,
    meaning: String,
    context_identifier: Option<String>,
    context_uid: Option<String>,
    mapping_resource: Option<String>,
    mapping_resource_uid: Option<String>,
    context_group_version: Option<String>,
    context_group_local_version: Option<String>,
    context_group_extension: Option<bool>,
    context_group_extension_creator_uid: Option<String>,
}

#[derive(Default)]
pub(crate) struct DicomCodeQualifiers {
    pub(crate) coding_scheme_version: Option<String>,
    pub(crate) context_identifier: Option<String>,
    pub(crate) context_uid: Option<String>,
    pub(crate) mapping_resource: Option<String>,
    pub(crate) mapping_resource_uid: Option<String>,
    pub(crate) context_group_version: Option<String>,
    pub(crate) context_group_local_version: Option<String>,
    pub(crate) context_group_extension: Option<bool>,
    pub(crate) context_group_extension_creator_uid: Option<String>,
}

impl DicomCode {
    pub fn new(
        value: impl Into<String>,
        scheme: impl Into<String>,
        meaning: impl Into<String>,
    ) -> Result<Self> {
        Self::from_kind(value, DicomCodeValueKind::Short, scheme, meaning)
    }

    pub fn new_long(
        value: impl Into<String>,
        scheme: impl Into<String>,
        meaning: impl Into<String>,
    ) -> Result<Self> {
        Self::from_kind(value, DicomCodeValueKind::Long, scheme, meaning)
    }

    pub fn new_urn(
        value: impl Into<String>,
        scheme: impl Into<String>,
        meaning: impl Into<String>,
    ) -> Result<Self> {
        Self::from_kind(value, DicomCodeValueKind::Urn, scheme, meaning)
    }

    fn from_kind(
        value: impl Into<String>,
        value_kind: DicomCodeValueKind,
        scheme: impl Into<String>,
        meaning: impl Into<String>,
    ) -> Result<Self> {
        let code = Self {
            value: value.into(),
            value_kind,
            scheme: scheme.into(),
            coding_scheme_version: None,
            meaning: meaning.into(),
            context_identifier: None,
            context_uid: None,
            mapping_resource: None,
            mapping_resource_uid: None,
            context_group_version: None,
            context_group_local_version: None,
            context_group_extension: None,
            context_group_extension_creator_uid: None,
        };
        let max_value_bytes = match value_kind {
            DicomCodeValueKind::Short => MAX_SHORT_CODE_VALUE_BYTES,
            DicomCodeValueKind::Long | DicomCodeValueKind::Urn => u32::MAX as usize,
        };
        validate_text("code value", &code.value, max_value_bytes)?;
        if value_kind != DicomCodeValueKind::Urn || !code.scheme.is_empty() {
            validate_text("coding scheme designator", &code.scheme, MAX_SCHEME_BYTES)?;
        }
        validate_text("code meaning", &code.meaning, MAX_CODE_MEANING_BYTES)?;
        Ok(code)
    }

    pub fn with_coding_scheme_version(mut self, version: impl Into<String>) -> Result<Self> {
        if self.scheme.is_empty() {
            return Err(Error::InvalidInput(
                "coding scheme version requires a coding scheme designator".into(),
            ));
        }
        let version = version.into();
        validate_text("coding scheme version", &version, 16)?;
        self.coding_scheme_version = Some(version);
        Ok(self)
    }

    pub fn with_context_identifier(
        mut self,
        identifier: impl Into<String>,
        mapping_resource: impl Into<String>,
        context_group_version: impl Into<String>,
    ) -> Result<Self> {
        let identifier = identifier.into();
        let mapping_resource = mapping_resource.into();
        let context_group_version = context_group_version.into();
        validate_text("context identifier", &identifier, 16)?;
        validate_text("mapping resource", &mapping_resource, 16)?;
        validate_text("context group version", &context_group_version, 26)?;
        self.context_identifier = Some(identifier);
        self.mapping_resource = Some(mapping_resource);
        self.context_group_version = Some(context_group_version);
        Ok(self)
    }

    pub fn with_context_uid(
        mut self,
        context_uid: impl Into<String>,
        mapping_resource_uid: impl Into<String>,
    ) -> Result<Self> {
        let context_uid = context_uid.into();
        let mapping_resource_uid = mapping_resource_uid.into();
        validate_dicom_uid("context UID", &context_uid)?;
        validate_dicom_uid("mapping resource UID", &mapping_resource_uid)?;
        self.context_uid = Some(context_uid);
        self.mapping_resource_uid = Some(mapping_resource_uid);
        Ok(self)
    }

    pub(crate) fn with_optional_qualifiers(
        mut self,
        qualifiers: DicomCodeQualifiers,
    ) -> Result<Self> {
        if let Some(value) = qualifiers.coding_scheme_version {
            self = self.with_coding_scheme_version(value)?;
        }
        if let Some(value) = qualifiers.context_identifier {
            validate_text("context identifier", &value, 16)?;
            self.context_identifier = Some(value);
        }
        if let Some(value) = qualifiers.mapping_resource {
            validate_text("mapping resource", &value, 16)?;
            self.mapping_resource = Some(value);
        }
        if let Some(value) = qualifiers.context_group_version {
            validate_text("context group version", &value, 26)?;
            self.context_group_version = Some(value);
        }
        if let Some(value) = qualifiers.context_uid {
            validate_dicom_uid("context UID", &value)?;
            self.context_uid = Some(value);
        }
        if let Some(value) = qualifiers.mapping_resource_uid {
            validate_dicom_uid("mapping resource UID", &value)?;
            self.mapping_resource_uid = Some(value);
        }
        if let Some(value) = qualifiers.context_group_local_version {
            validate_text("context group local version", &value, 26)?;
            self.context_group_local_version = Some(value);
        }
        self.context_group_extension = qualifiers.context_group_extension;
        if let Some(value) = qualifiers.context_group_extension_creator_uid {
            validate_dicom_uid("context group extension creator UID", &value)?;
            self.context_group_extension_creator_uid = Some(value);
        }
        if self.context_identifier.is_some() {
            if self.mapping_resource.is_none() {
                return Err(Error::InvalidInput(
                    "context identifier requires a mapping resource".into(),
                ));
            }
            if self.context_group_version.is_none() {
                return Err(Error::InvalidInput(
                    "context identifier requires a context group version".into(),
                ));
            }
        }
        if self.context_group_extension == Some(true) {
            if self.context_identifier.is_none() {
                return Err(Error::InvalidInput(
                    "context group extension requires a context identifier".into(),
                ));
            }
            if self.context_group_local_version.is_none() {
                return Err(Error::InvalidInput(
                    "context group extension requires a context group local version".into(),
                ));
            }
            if self.context_group_extension_creator_uid.is_none() {
                return Err(Error::InvalidInput(
                    "context group extension requires an extension creator UID".into(),
                ));
            }
        }
        Ok(self)
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    #[must_use]
    pub const fn value_kind(&self) -> DicomCodeValueKind {
        self.value_kind
    }

    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    #[must_use]
    pub fn coding_scheme_version(&self) -> Option<&str> {
        self.coding_scheme_version.as_deref()
    }

    #[must_use]
    pub fn meaning(&self) -> &str {
        &self.meaning
    }

    #[must_use]
    pub fn context_identifier(&self) -> Option<&str> {
        self.context_identifier.as_deref()
    }

    #[must_use]
    pub fn context_uid(&self) -> Option<&str> {
        self.context_uid.as_deref()
    }

    #[must_use]
    pub fn mapping_resource(&self) -> Option<&str> {
        self.mapping_resource.as_deref()
    }

    #[must_use]
    pub fn mapping_resource_uid(&self) -> Option<&str> {
        self.mapping_resource_uid.as_deref()
    }

    #[must_use]
    pub fn context_group_version(&self) -> Option<&str> {
        self.context_group_version.as_deref()
    }

    #[must_use]
    pub fn context_group_local_version(&self) -> Option<&str> {
        self.context_group_local_version.as_deref()
    }

    #[must_use]
    pub const fn context_group_extension(&self) -> Option<bool> {
        self.context_group_extension
    }

    #[must_use]
    pub fn context_group_extension_creator_uid(&self) -> Option<&str> {
        self.context_group_extension_creator_uid.as_deref()
    }
}

pub(crate) fn validate_text(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.trim().is_empty() {
        return Err(Error::InvalidInput(format!("{name} cannot be empty")));
    }
    if value.len() > max_bytes {
        return Err(Error::InvalidInput(format!(
            "{name} exceeds {max_bytes} bytes"
        )));
    }
    if value.contains(['\\', '\0']) {
        return Err(Error::InvalidInput(format!(
            "{name} contains a prohibited DICOM value separator or NUL"
        )));
    }
    Ok(())
}

pub(crate) fn is_valid_dicom_uid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.split('.').all(|component| {
            !component.is_empty()
                && component.bytes().all(|byte| byte.is_ascii_digit())
                && (component.len() == 1 || !component.starts_with('0'))
        })
}

fn validate_dicom_uid(name: &str, value: &str) -> Result<()> {
    if !is_valid_dicom_uid(value) {
        return Err(Error::InvalidInput(format!(
            "{name} is not a valid DICOM UID"
        )));
    }
    Ok(())
}
