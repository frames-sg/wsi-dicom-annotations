mod conversion;
mod document;
mod raster;
mod read;
mod runs;
mod write;

#[cfg(test)]
#[path = "seg/performance_tests.rs"]
mod performance_tests;
#[cfg(test)]
#[path = "seg/run_tests.rs"]
mod run_tests;
#[cfg(test)]
#[path = "seg/vectorization_tests.rs"]
mod vectorization_tests;

use crate::{Error, Result};

use super::context::DicomAnnotationContext;
use super::derived_object::DerivedObjectProducer;
use super::model::AnnotationGroup;
use super::model::{
    validate_polygon, validate_text, AlgorithmIdentification, DicomCode, FindingSemantics,
    GenerationType, InteroperabilityDiagnostic, Point2,
};

const MAX_SEG_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_SEGMENTATION_FRAMES: usize = 65_536;
const MAX_SEGMENTATION_PIXELS: usize = 512 * 1024 * 1024;
const MAX_VECTORIZED_RUNS: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentationKind {
    Binary,
    LabelMap,
    Fractional,
}

/// Controls whether SEG-to-ANN projection may discard nonrepresentable semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegToAnnConversionPolicy {
    RejectLoss,
    AllowLoss,
}

/// Editable ANN groups projected from a SEG, together with typed loss diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorizedAnnotations {
    groups: Vec<AnnotationGroup>,
    diagnostics: Vec<InteroperabilityDiagnostic>,
}

impl VectorizedAnnotations {
    #[must_use]
    pub fn groups(&self) -> &[AnnotationGroup] {
        &self.groups
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[InteroperabilityDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn into_groups(self) -> Vec<AnnotationGroup> {
        self.groups
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentationSegment {
    source_segment_number: Option<u16>,
    label: String,
    description: String,
    finding: FindingSemantics,
    tracking_id: Option<String>,
    tracking_uid: Option<String>,
    outer_polygons: Vec<Vec<Point2>>,
    exclusion_polygons: Vec<Vec<Point2>>,
    // `Some` preserves GeoJSON MultiPolygon ownership: each entry contains
    // the holes belonging only to the outer polygon at the same index.
    component_holes: Option<Vec<Vec<Vec<Point2>>>>,
}

impl SegmentationSegment {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        label: impl Into<String>,
        category: DicomCode,
        property_type: DicomCode,
        recommended_display_cielab: [u16; 3],
        outer_polygons: Vec<Vec<Point2>>,
        exclusion_polygons: Vec<Vec<Point2>>,
    ) -> Result<Self> {
        let label = label.into();
        if label.trim().is_empty() || label.len() > 64 || label.contains(['\\', '\0']) {
            return Err(Error::InvalidInput(
                "segment label must be 1..=64 bytes and contain no DICOM separator or NUL".into(),
            ));
        }
        if outer_polygons.is_empty() {
            return Err(Error::InvalidInput(
                "a binary segment needs at least one outer polygon".into(),
            ));
        }
        for polygon in outer_polygons.iter().chain(&exclusion_polygons) {
            validate_polygon(polygon)?;
        }
        Ok(Self {
            source_segment_number: None,
            label,
            description: String::new(),
            finding: FindingSemantics::manual(category, property_type, recommended_display_cielab),
            tracking_id: None,
            tracking_uid: None,
            outer_polygons,
            exclusion_polygons,
            component_holes: None,
        })
    }

    pub fn with_component_holes(mut self, component_holes: Vec<Vec<Vec<Point2>>>) -> Result<Self> {
        if component_holes.len() != self.outer_polygons.len() {
            return Err(Error::InvalidInput(
                "component hole groups must match the outer polygon count".into(),
            ));
        }
        for hole in component_holes.iter().flatten() {
            validate_polygon(hole)?;
        }
        self.exclusion_polygons = component_holes.iter().flatten().cloned().collect();
        self.component_holes = Some(component_holes);
        Ok(self)
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Result<Self> {
        let description = description.into();
        if description.len() > 1_024 || description.contains('\0') {
            return Err(Error::InvalidInput(
                "segment description exceeds 1024 bytes or contains NUL".into(),
            ));
        }
        self.description = description;
        Ok(self)
    }

    pub fn with_generation(
        mut self,
        generation_type: GenerationType,
        algorithms: Vec<AlgorithmIdentification>,
    ) -> Result<Self> {
        self.finding = self.finding.with_generation(generation_type, algorithms)?;
        Ok(self)
    }

    #[must_use]
    pub fn with_property_type_modifiers(mut self, modifiers: Vec<DicomCode>) -> Self {
        self.finding = self.finding.with_property_type_modifiers(modifiers);
        self
    }

    pub fn with_tracking(
        mut self,
        tracking_id: impl Into<String>,
        tracking_uid: impl Into<String>,
    ) -> Result<Self> {
        let tracking_id = tracking_id.into();
        let tracking_uid = tracking_uid.into();
        validate_text("tracking ID", &tracking_id, 10_240)?;
        validate_text("tracking UID", &tracking_uid, 64)?;
        self.tracking_id = Some(tracking_id);
        self.tracking_uid = Some(tracking_uid);
        Ok(self)
    }

    #[must_use]
    pub fn with_anatomic_regions(mut self, regions: Vec<DicomCode>) -> Self {
        self.finding = self.finding.with_anatomic_regions(regions);
        self
    }

    #[must_use]
    pub fn with_primary_anatomic_structures(mut self, structures: Vec<DicomCode>) -> Self {
        self.finding = self.finding.with_primary_anatomic_structures(structures);
        self
    }

    pub(crate) fn with_finding_semantics(mut self, finding: FindingSemantics) -> Result<Self> {
        finding.validate()?;
        self.finding = finding;
        Ok(self)
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub const fn generation_type(&self) -> GenerationType {
        self.finding.generation_type()
    }

    #[must_use]
    pub fn algorithms(&self) -> &[AlgorithmIdentification] {
        self.finding.algorithms()
    }

    #[must_use]
    pub fn source_segment_number(&self) -> Option<u16> {
        self.source_segment_number
    }

    #[must_use]
    pub fn category(&self) -> &DicomCode {
        self.finding.category()
    }

    #[must_use]
    pub fn property_type(&self) -> &DicomCode {
        self.finding.property_type()
    }

    #[must_use]
    pub fn property_type_modifiers(&self) -> &[DicomCode] {
        self.finding.property_type_modifiers()
    }

    #[must_use]
    pub fn tracking_id(&self) -> Option<&str> {
        self.tracking_id.as_deref()
    }

    #[must_use]
    pub fn tracking_uid(&self) -> Option<&str> {
        self.tracking_uid.as_deref()
    }

    #[must_use]
    pub fn anatomic_regions(&self) -> &[DicomCode] {
        self.finding.anatomic_regions()
    }

    #[must_use]
    pub fn primary_anatomic_structures(&self) -> &[DicomCode] {
        self.finding.primary_anatomic_structures()
    }

    #[must_use]
    pub const fn recommended_display_cielab(&self) -> [u16; 3] {
        self.finding.recommended_display_cielab()
    }

    #[must_use]
    pub fn outer_polygons(&self) -> &[Vec<Point2>] {
        &self.outer_polygons
    }

    #[must_use]
    pub fn exclusion_polygons(&self) -> &[Vec<Point2>] {
        &self.exclusion_polygons
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinarySegmentationFrame {
    segment_number: u16,
    tile_col: u32,
    tile_row: u32,
    width: u16,
    height: u16,
    mask: Vec<bool>,
}

impl BinarySegmentationFrame {
    #[must_use]
    pub const fn segment_number(&self) -> u16 {
        self.segment_number
    }

    #[must_use]
    pub const fn tile_col(&self) -> u32 {
        self.tile_col
    }

    #[must_use]
    pub const fn tile_row(&self) -> u32 {
        self.tile_row
    }

    #[must_use]
    pub const fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    #[must_use]
    pub fn mask(&self) -> &[bool] {
        &self.mask
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FractionalSegmentationFrame {
    segment_number: u16,
    tile_col: u32,
    tile_row: u32,
    width: u16,
    height: u16,
    maximum_fractional_value: u16,
    values: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryMaskRun {
    segment_number: u16,
    row: u32,
    column_start: u32,
    length: u32,
}

impl BinaryMaskRun {
    #[must_use]
    pub const fn segment_number(&self) -> u16 {
        self.segment_number
    }

    #[must_use]
    pub const fn row(&self) -> u32 {
        self.row
    }

    #[must_use]
    pub const fn column_start(&self) -> u32 {
        self.column_start
    }

    #[must_use]
    pub const fn length(&self) -> u32 {
        self.length
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FractionalMaskRun {
    segment_number: u16,
    row: u32,
    column_start: u32,
    maximum_fractional_value: u16,
    values: Vec<u16>,
}

impl FractionalMaskRun {
    #[must_use]
    pub const fn segment_number(&self) -> u16 {
        self.segment_number
    }

    #[must_use]
    pub const fn row(&self) -> u32 {
        self.row
    }

    #[must_use]
    pub const fn column_start(&self) -> u32 {
        self.column_start
    }

    #[must_use]
    pub const fn maximum_fractional_value(&self) -> u16 {
        self.maximum_fractional_value
    }

    #[must_use]
    pub fn values(&self) -> &[u16] {
        &self.values
    }
}

impl FractionalSegmentationFrame {
    #[must_use]
    pub const fn segment_number(&self) -> u16 {
        self.segment_number
    }

    #[must_use]
    pub const fn tile_col(&self) -> u32 {
        self.tile_col
    }

    #[must_use]
    pub const fn tile_row(&self) -> u32 {
        self.tile_row
    }

    #[must_use]
    pub const fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    #[must_use]
    pub const fn maximum_fractional_value(&self) -> u16 {
        self.maximum_fractional_value
    }

    #[must_use]
    pub fn values(&self) -> &[u16] {
        &self.values
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentationDocument {
    source: DicomAnnotationContext,
    kind: SegmentationKind,
    sop_instance_uid: String,
    series_instance_uid: String,
    content_label: String,
    content_description: String,
    content_creator_name: Option<String>,
    producer: DerivedObjectProducer,
    segments: Vec<SegmentationSegment>,
    imported_binary_frames: Option<Vec<BinarySegmentationFrame>>,
    imported_fractional_frames: Option<Vec<FractionalSegmentationFrame>>,
    diagnostics: Vec<InteroperabilityDiagnostic>,
}
