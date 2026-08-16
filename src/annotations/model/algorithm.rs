use crate::{Error, Result};

use super::code::{validate_text, DicomCode, MAX_LONG_TEXT_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationType {
    Manual,
    Semiautomatic,
    Automatic,
}

impl GenerationType {
    #[must_use]
    pub const fn dicom_value(self) -> &'static str {
        match self {
            Self::Manual => "MANUAL",
            Self::Semiautomatic => "SEMIAUTOMATIC",
            Self::Automatic => "AUTOMATIC",
        }
    }

    pub(crate) fn from_dicom(value: &str) -> Result<Self> {
        match value.trim() {
            "MANUAL" => Ok(Self::Manual),
            "SEMIAUTOMATIC" => Ok(Self::Semiautomatic),
            "AUTOMATIC" => Ok(Self::Automatic),
            other => Err(Error::Unsupported(format!(
                "generation type {other:?} is not recognized"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgorithmIdentification {
    family: DicomCode,
    name_code: Option<DicomCode>,
    name: String,
    version: String,
    parameters: Option<String>,
    source: Option<String>,
}

impl AlgorithmIdentification {
    pub fn new(
        family: DicomCode,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<Self> {
        let name = name.into();
        let version = version.into();
        validate_text("algorithm name", &name, 64)?;
        validate_text("algorithm version", &version, 64)?;
        Ok(Self {
            family,
            name_code: None,
            name,
            version,
            parameters: None,
            source: None,
        })
    }

    #[must_use]
    pub fn with_name_code(mut self, code: DicomCode) -> Self {
        self.name_code = Some(code);
        self
    }

    pub fn with_parameters(mut self, parameters: impl Into<String>) -> Result<Self> {
        let parameters = parameters.into();
        validate_text("algorithm parameters", &parameters, MAX_LONG_TEXT_BYTES)?;
        self.parameters = Some(parameters);
        Ok(self)
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Result<Self> {
        let source = source.into();
        validate_text("algorithm source", &source, 64)?;
        self.source = Some(source);
        Ok(self)
    }

    #[must_use]
    pub fn family(&self) -> &DicomCode {
        &self.family
    }

    #[must_use]
    pub fn name_code(&self) -> Option<&DicomCode> {
        self.name_code.as_ref()
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn parameters(&self) -> Option<&str> {
        self.parameters.as_deref()
    }

    #[must_use]
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
}

pub(crate) fn validate_generation(
    generation_type: GenerationType,
    algorithms: &[AlgorithmIdentification],
) -> Result<()> {
    match generation_type {
        GenerationType::Manual if !algorithms.is_empty() => Err(Error::InvalidInput(
            "manual annotations cannot identify a generation algorithm".into(),
        )),
        GenerationType::Semiautomatic | GenerationType::Automatic if algorithms.is_empty() => {
            Err(Error::InvalidInput(
                "automatic and semiautomatic annotations require an algorithm identification"
                    .into(),
            ))
        }
        _ => Ok(()),
    }
}
