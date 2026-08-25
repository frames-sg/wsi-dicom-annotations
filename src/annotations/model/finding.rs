use crate::Result;

use super::{validate_generation, AlgorithmIdentification, DicomCode, GenerationType};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FindingSemantics {
    generation_type: GenerationType,
    algorithms: Vec<AlgorithmIdentification>,
    category: DicomCode,
    property_type: DicomCode,
    property_type_modifiers: Vec<DicomCode>,
    anatomic_regions: Vec<DicomCode>,
    primary_anatomic_structures: Vec<DicomCode>,
    recommended_display_cielab: [u16; 3],
}

impl FindingSemantics {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        generation_type: GenerationType,
        algorithms: Vec<AlgorithmIdentification>,
        category: DicomCode,
        property_type: DicomCode,
        property_type_modifiers: Vec<DicomCode>,
        anatomic_regions: Vec<DicomCode>,
        primary_anatomic_structures: Vec<DicomCode>,
        recommended_display_cielab: [u16; 3],
    ) -> Result<Self> {
        validate_generation(generation_type, &algorithms)?;
        Ok(Self {
            generation_type,
            algorithms,
            category,
            property_type,
            property_type_modifiers,
            anatomic_regions,
            primary_anatomic_structures,
            recommended_display_cielab,
        })
    }

    pub(crate) fn manual(
        category: DicomCode,
        property_type: DicomCode,
        recommended_display_cielab: [u16; 3],
    ) -> Self {
        Self {
            generation_type: GenerationType::Manual,
            algorithms: Vec::new(),
            category,
            property_type,
            property_type_modifiers: Vec::new(),
            anatomic_regions: Vec::new(),
            primary_anatomic_structures: Vec::new(),
            recommended_display_cielab,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        validate_generation(self.generation_type, &self.algorithms)
    }

    pub(crate) fn with_generation(
        mut self,
        generation_type: GenerationType,
        algorithms: Vec<AlgorithmIdentification>,
    ) -> Result<Self> {
        validate_generation(generation_type, &algorithms)?;
        self.generation_type = generation_type;
        self.algorithms = algorithms;
        Ok(self)
    }

    pub(crate) fn with_property_type_modifiers(mut self, modifiers: Vec<DicomCode>) -> Self {
        self.property_type_modifiers = modifiers;
        self
    }

    pub(crate) fn with_anatomic_regions(mut self, regions: Vec<DicomCode>) -> Self {
        self.anatomic_regions = regions;
        self
    }

    pub(crate) fn with_primary_anatomic_structures(mut self, structures: Vec<DicomCode>) -> Self {
        self.primary_anatomic_structures = structures;
        self
    }

    pub(crate) const fn generation_type(&self) -> GenerationType {
        self.generation_type
    }

    pub(crate) fn algorithms(&self) -> &[AlgorithmIdentification] {
        &self.algorithms
    }

    pub(crate) fn category(&self) -> &DicomCode {
        &self.category
    }

    pub(crate) fn property_type(&self) -> &DicomCode {
        &self.property_type
    }

    pub(crate) fn property_type_modifiers(&self) -> &[DicomCode] {
        &self.property_type_modifiers
    }

    pub(crate) fn anatomic_regions(&self) -> &[DicomCode] {
        &self.anatomic_regions
    }

    pub(crate) fn primary_anatomic_structures(&self) -> &[DicomCode] {
        &self.primary_anatomic_structures
    }

    pub(crate) const fn recommended_display_cielab(&self) -> [u16; 3] {
        self.recommended_display_cielab
    }
}

#[cfg(test)]
#[path = "finding_tests.rs"]
mod tests;
