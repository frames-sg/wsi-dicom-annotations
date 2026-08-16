use serde::{Deserialize, Serialize};

use crate::{Error, Result};

use super::model::{AlgorithmIdentification, DicomCode, DicomCodeQualifiers};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileAlgorithm {
    family: ProfileCode,
    #[serde(default)]
    name_code: Option<ProfileCode>,
    name: String,
    version: String,
    #[serde(default)]
    parameters: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

impl ProfileAlgorithm {
    pub(crate) fn into_algorithm(self) -> Result<AlgorithmIdentification> {
        let mut algorithm =
            AlgorithmIdentification::new(self.family.into_code()?, self.name, self.version)?;
        if let Some(code) = self.name_code {
            algorithm = algorithm.with_name_code(code.into_code()?);
        }
        if let Some(parameters) = self.parameters {
            algorithm = algorithm.with_parameters(parameters)?;
        }
        if let Some(source) = self.source {
            algorithm = algorithm.with_source(source)?;
        }
        Ok(algorithm)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileCode {
    #[serde(default)]
    code_value: Option<String>,
    #[serde(default)]
    long_code_value: Option<String>,
    #[serde(default)]
    urn_code_value: Option<String>,
    #[serde(default)]
    coding_scheme_designator: String,
    #[serde(default)]
    coding_scheme_version: Option<String>,
    code_meaning: String,
    #[serde(default)]
    context_identifier: Option<String>,
    #[serde(default)]
    context_uid: Option<String>,
    #[serde(default)]
    mapping_resource: Option<String>,
    #[serde(default)]
    mapping_resource_uid: Option<String>,
    #[serde(default)]
    context_group_version: Option<String>,
    #[serde(default)]
    context_group_local_version: Option<String>,
    #[serde(default)]
    context_group_extension: Option<bool>,
    #[serde(default)]
    context_group_extension_creator_uid: Option<String>,
}

impl ProfileCode {
    pub(crate) fn from_code(code: &DicomCode) -> Self {
        let (code_value, long_code_value, urn_code_value) = match code.value_kind() {
            super::model::DicomCodeValueKind::Short => (Some(code.value().to_string()), None, None),
            super::model::DicomCodeValueKind::Long => (None, Some(code.value().to_string()), None),
            super::model::DicomCodeValueKind::Urn => (None, None, Some(code.value().to_string())),
        };
        Self {
            code_value,
            long_code_value,
            urn_code_value,
            coding_scheme_designator: code.scheme().to_string(),
            coding_scheme_version: code.coding_scheme_version().map(str::to_string),
            code_meaning: code.meaning().to_string(),
            context_identifier: code.context_identifier().map(str::to_string),
            context_uid: code.context_uid().map(str::to_string),
            mapping_resource: code.mapping_resource().map(str::to_string),
            mapping_resource_uid: code.mapping_resource_uid().map(str::to_string),
            context_group_version: code.context_group_version().map(str::to_string),
            context_group_local_version: code.context_group_local_version().map(str::to_string),
            context_group_extension: code.context_group_extension(),
            context_group_extension_creator_uid: code
                .context_group_extension_creator_uid()
                .map(str::to_string),
        }
    }

    pub(crate) fn into_code(self) -> Result<DicomCode> {
        let mut code = match (self.code_value, self.long_code_value, self.urn_code_value) {
            (Some(value), None, None) => {
                DicomCode::new(value, self.coding_scheme_designator, self.code_meaning)?
            }
            (None, Some(value), None) => {
                DicomCode::new_long(value, self.coding_scheme_designator, self.code_meaning)?
            }
            (None, None, Some(value)) => {
                DicomCode::new_urn(value, self.coding_scheme_designator, self.code_meaning)?
            }
            _ => {
                return Err(Error::InvalidInput(
                    "a mapped code requires exactly one of code_value, long_code_value, or urn_code_value"
                        .into(),
                ));
            }
        };
        code = code.with_optional_qualifiers(DicomCodeQualifiers {
            coding_scheme_version: self.coding_scheme_version,
            context_identifier: self.context_identifier,
            context_uid: self.context_uid,
            mapping_resource: self.mapping_resource,
            mapping_resource_uid: self.mapping_resource_uid,
            context_group_version: self.context_group_version,
            context_group_local_version: self.context_group_local_version,
            context_group_extension: self.context_group_extension,
            context_group_extension_creator_uid: self.context_group_extension_creator_uid,
        })?;
        Ok(code)
    }
}

pub(crate) fn profile_codes(codes: Vec<ProfileCode>) -> Result<Vec<DicomCode>> {
    codes.into_iter().map(ProfileCode::into_code).collect()
}
