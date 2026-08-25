use crate::{Error, Result};

use super::super::dicom_dataset::new_dicom_uid;
use super::algorithm::{AlgorithmIdentification, GenerationType};
use super::code::{is_valid_dicom_uid, validate_text, DicomCode, MAX_LONG_TEXT_BYTES};
use super::finding::FindingSemantics;
use super::geometry::{validate_points, validate_polygon, AnnotationGeometry, Point2};

#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationMeasurement {
    concept: DicomCode,
    units: DicomCode,
    values: Vec<f64>,
    annotation_indices: Option<Vec<u32>>,
}

impl AnnotationMeasurement {
    #[must_use]
    pub fn new(concept: DicomCode, units: DicomCode, values: Vec<f64>) -> Self {
        Self {
            concept,
            units,
            values,
            annotation_indices: None,
        }
    }

    pub fn for_annotations(
        concept: DicomCode,
        units: DicomCode,
        values: Vec<f64>,
        annotation_indices: Vec<u32>,
    ) -> Result<Self> {
        if values.len() != annotation_indices.len() {
            return Err(Error::InvalidInput(format!(
                "measurement has {} values but {} annotation indices",
                values.len(),
                annotation_indices.len()
            )));
        }
        if annotation_indices.windows(2).any(|pair| pair[0] >= pair[1])
            || annotation_indices.first().is_some_and(|index| *index == 0)
        {
            return Err(Error::InvalidInput(
                "measurement annotation indices must be strictly increasing and one-based".into(),
            ));
        }
        Ok(Self {
            concept,
            units,
            values,
            annotation_indices: Some(annotation_indices),
        })
    }

    #[must_use]
    pub fn concept(&self) -> &DicomCode {
        &self.concept
    }

    #[must_use]
    pub fn units(&self) -> &DicomCode {
        &self.units
    }

    #[must_use]
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    #[must_use]
    pub fn annotation_indices(&self) -> Option<&[u32]> {
        self.annotation_indices.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationGroup {
    uid: String,
    label: String,
    description: String,
    finding: FindingSemantics,
    applies_to_all_optical_paths: bool,
    referenced_optical_paths: Vec<String>,
    applies_to_all_z_planes: bool,
    common_z_coordinates: Vec<f64>,
    geometry: AnnotationGeometry,
    measurements: Vec<AnnotationMeasurement>,
}

impl AnnotationGroup {
    pub fn points(
        label: impl Into<String>,
        category: DicomCode,
        property_type: DicomCode,
        recommended_display_cielab: [u16; 3],
        points: Vec<Point2>,
    ) -> Result<Self> {
        validate_points(points.iter().copied())?;
        Self::from_parts(
            new_dicom_uid(),
            label.into(),
            String::new(),
            FindingSemantics::manual(category, property_type, recommended_display_cielab),
            true,
            Vec::new(),
            true,
            Vec::new(),
            AnnotationGeometry::Points(points),
            Vec::new(),
        )
    }

    pub fn polygons(
        label: impl Into<String>,
        category: DicomCode,
        property_type: DicomCode,
        recommended_display_cielab: [u16; 3],
        polygons: Vec<Vec<Point2>>,
    ) -> Result<Self> {
        for polygon in &polygons {
            validate_polygon(polygon)?;
        }
        Self::from_parts(
            new_dicom_uid(),
            label.into(),
            String::new(),
            FindingSemantics::manual(category, property_type, recommended_display_cielab),
            true,
            Vec::new(),
            true,
            Vec::new(),
            AnnotationGeometry::Polygons(polygons),
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        uid: String,
        label: String,
        description: String,
        finding: FindingSemantics,
        applies_to_all_optical_paths: bool,
        referenced_optical_paths: Vec<String>,
        applies_to_all_z_planes: bool,
        common_z_coordinates: Vec<f64>,
        geometry: AnnotationGeometry,
        measurements: Vec<AnnotationMeasurement>,
    ) -> Result<Self> {
        let group = Self {
            uid,
            label,
            description,
            finding,
            applies_to_all_optical_paths,
            referenced_optical_paths,
            applies_to_all_z_planes,
            common_z_coordinates,
            geometry,
            measurements,
        };
        group.validate()?;
        Ok(group)
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Result<Self> {
        let description = description.into();
        if description.len() > MAX_LONG_TEXT_BYTES {
            return Err(Error::InvalidInput(format!(
                "annotation group description exceeds {MAX_LONG_TEXT_BYTES} bytes"
            )));
        }
        if description.contains('\0') {
            return Err(Error::InvalidInput(
                "annotation group description contains NUL".into(),
            ));
        }
        self.description = description;
        Ok(self)
    }

    /// Replaces the DICOM Annotation Group UID with a stable, caller-owned UID.
    ///
    /// This is useful when one tracked finding is represented by one ANN group
    /// and its identity must remain stable across repeated exports.
    pub fn with_uid(mut self, uid: impl Into<String>) -> Result<Self> {
        let uid = uid.into();
        validate_text("annotation group UID", &uid, 64)?;
        if !super::code::is_valid_dicom_uid(&uid) {
            return Err(Error::InvalidInput(
                "annotation group UID is not a valid DICOM UID".into(),
            ));
        }
        self.uid = uid;
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
        self.finding = finding;
        self.validate()?;
        Ok(self)
    }

    pub fn with_referenced_optical_paths(mut self, paths: Vec<String>) -> Result<Self> {
        validate_optical_path_applicability(false, &paths)?;
        for path in &paths {
            validate_text("referenced optical path identifier", path, 16)?;
        }
        if paths
            .iter()
            .enumerate()
            .any(|(index, path)| paths[index + 1..].contains(path))
        {
            return Err(Error::InvalidInput(
                "referenced optical path identifiers must be unique".into(),
            ));
        }
        self.applies_to_all_optical_paths = false;
        self.referenced_optical_paths = paths;
        Ok(self)
    }

    pub fn with_common_z_coordinates(mut self, coordinates: Vec<f64>) -> Result<Self> {
        if coordinates.is_empty() || coordinates.iter().any(|value| !value.is_finite()) {
            return Err(Error::InvalidInput(
                "common Z coordinates must contain finite values".into(),
            ));
        }
        self.applies_to_all_z_planes = false;
        self.common_z_coordinates = coordinates;
        Ok(self)
    }

    pub fn add_measurement(&mut self, measurement: AnnotationMeasurement) -> Result<()> {
        self.measurements.push(measurement);
        if let Err(error) = self.validate_measurements() {
            self.measurements.pop();
            return Err(error);
        }
        Ok(())
    }

    pub fn validate_geometry(&self) -> Result<()> {
        match &self.geometry {
            AnnotationGeometry::Points(points) => validate_points(points.iter().copied()),
            AnnotationGeometry::Polygons(polygons) => {
                for polygon in polygons {
                    validate_polygon(polygon)?;
                }
                Ok(())
            }
            AnnotationGeometry::ReadOnly { .. } => Ok(()),
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        validate_text("annotation group UID", &self.uid, 64)?;
        if !is_valid_dicom_uid(&self.uid) {
            return Err(Error::InvalidInput(
                "annotation group UID is not a valid DICOM UID".into(),
            ));
        }
        validate_text("annotation group label", &self.label, 64)?;
        if self.description.len() > MAX_LONG_TEXT_BYTES || self.description.contains('\0') {
            return Err(Error::InvalidInput(format!(
                "annotation group description exceeds {MAX_LONG_TEXT_BYTES} bytes or contains NUL"
            )));
        }
        self.finding.validate()?;
        validate_optical_path_applicability(
            self.applies_to_all_optical_paths,
            &self.referenced_optical_paths,
        )?;
        for path in &self.referenced_optical_paths {
            validate_text("referenced optical path identifier", path, 16)?;
        }
        if self
            .referenced_optical_paths
            .iter()
            .enumerate()
            .any(|(index, path)| self.referenced_optical_paths[index + 1..].contains(path))
        {
            return Err(Error::InvalidInput(
                "referenced optical path identifiers must be unique".into(),
            ));
        }
        if self
            .common_z_coordinates
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(Error::InvalidInput(
                "common Z coordinates must be finite".into(),
            ));
        }
        if self.applies_to_all_z_planes && !self.common_z_coordinates.is_empty() {
            return Err(Error::InvalidInput(
                "common Z coordinates cannot be present when annotations apply to all Z planes"
                    .into(),
            ));
        }
        if self.annotation_count() == 0 {
            return Err(Error::InvalidInput(format!(
                "annotation group {:?} has no annotations",
                self.label
            )));
        }
        self.validate_geometry()?;
        self.validate_measurements()
    }

    #[must_use]
    pub fn annotation_count(&self) -> usize {
        self.geometry.annotation_count()
    }

    fn validate_measurements(&self) -> Result<()> {
        let count = self.geometry.annotation_count();
        for measurement in &self.measurements {
            if measurement.values.iter().any(|value| !value.is_finite()) {
                return Err(Error::InvalidInput(
                    "annotation measurements must be finite".into(),
                ));
            }
            match measurement.annotation_indices() {
                Some(indices) => {
                    if indices.last().is_some_and(|index| *index as usize > count) {
                        return Err(Error::InvalidInput(format!(
                            "measurement references annotation beyond group size {count}"
                        )));
                    }
                }
                None if measurement.values.len() != count => {
                    return Err(Error::InvalidInput(format!(
                        "measurement has {} values for {count} annotations",
                        measurement.values.len()
                    )));
                }
                None => {}
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn uid(&self) -> &str {
        &self.uid
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
    pub fn anatomic_regions(&self) -> &[DicomCode] {
        self.finding.anatomic_regions()
    }

    #[must_use]
    pub fn primary_anatomic_structures(&self) -> &[DicomCode] {
        self.finding.primary_anatomic_structures()
    }

    #[must_use]
    pub const fn applies_to_all_optical_paths(&self) -> bool {
        self.applies_to_all_optical_paths
    }

    #[must_use]
    pub fn referenced_optical_paths(&self) -> &[String] {
        &self.referenced_optical_paths
    }

    #[must_use]
    pub const fn applies_to_all_z_planes(&self) -> bool {
        self.applies_to_all_z_planes
    }

    #[must_use]
    pub fn common_z_coordinates(&self) -> &[f64] {
        &self.common_z_coordinates
    }

    #[must_use]
    pub const fn recommended_display_cielab(&self) -> [u16; 3] {
        self.finding.recommended_display_cielab()
    }

    #[must_use]
    pub fn geometry(&self) -> &AnnotationGeometry {
        &self.geometry
    }

    #[must_use]
    pub fn point_annotations(&self) -> Option<&[Point2]> {
        match &self.geometry {
            AnnotationGeometry::Points(points) => Some(points),
            _ => None,
        }
    }

    /// Atomically replaces point geometry after revalidating all group invariants.
    pub fn replace_points(&mut self, points: Vec<Point2>) -> Result<()> {
        if !matches!(self.geometry, AnnotationGeometry::Points(_)) {
            return Err(Error::InvalidInput(
                "point replacement requires a point annotation group".into(),
            ));
        }
        let mut candidate = self.clone();
        candidate.geometry = AnnotationGeometry::Points(points);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    #[must_use]
    pub fn polygon_annotations(&self) -> Option<&[Vec<Point2>]> {
        match &self.geometry {
            AnnotationGeometry::Polygons(polygons) => Some(polygons),
            _ => None,
        }
    }

    /// Atomically replaces polygon geometry after revalidating all group invariants.
    pub fn replace_polygons(&mut self, polygons: Vec<Vec<Point2>>) -> Result<()> {
        if !matches!(self.geometry, AnnotationGeometry::Polygons(_)) {
            return Err(Error::InvalidInput(
                "polygon replacement requires a polygon annotation group".into(),
            ));
        }
        let mut candidate = self.clone();
        candidate.geometry = AnnotationGeometry::Polygons(polygons);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    #[must_use]
    pub fn measurements(&self) -> &[AnnotationMeasurement] {
        &self.measurements
    }

    #[must_use]
    pub const fn editable(&self) -> bool {
        self.geometry.editable()
    }
}

fn validate_optical_path_applicability(all: bool, identifiers: &[String]) -> Result<()> {
    if all == identifiers.is_empty() {
        return Ok(());
    }
    if all {
        return Err(Error::InvalidInput(
            "referenced optical path identifiers cannot be present when applicability is all"
                .into(),
        ));
    }
    Err(Error::InvalidInput(
        "at least one referenced optical path identifier is required".into(),
    ))
}

#[cfg(test)]
#[path = "annotation_tests.rs"]
mod tests;
