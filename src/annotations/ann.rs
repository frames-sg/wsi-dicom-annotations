mod geometry_codec;
mod read;
mod write;

#[cfg(test)]
#[path = "ann_validation_tests.rs"]
mod validation_tests;

use std::collections::BTreeSet;
use std::path::Path;

use crate::{Error, Result};

use super::context::DicomAnnotationContext;
use super::derived_object::DerivedObjectProducer;
use super::dicom_dataset::new_dicom_uid;
use super::model::{AnnotationGeometry, AnnotationGroup, InteroperabilityDiagnostic, Point2};

const MAX_ANN_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ANNOTATION_GROUPS: usize = 10_000;
const MAX_COORDINATE_VALUES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationDocument {
    source: DicomAnnotationContext,
    sop_instance_uid: String,
    series_instance_uid: String,
    predecessor_sop_instance_uid: Option<String>,
    coordinate_type: String,
    pixel_origin_interpretation: Option<String>,
    references_source_image: bool,
    referenced_frame_number: Option<u32>,
    content_label: String,
    content_description: String,
    content_creator_name: Option<String>,
    producer: DerivedObjectProducer,
    groups: Vec<AnnotationGroup>,
    diagnostics: Vec<InteroperabilityDiagnostic>,
}

impl AnnotationDocument {
    pub fn new(source: DicomAnnotationContext, groups: Vec<AnnotationGroup>) -> Result<Self> {
        let document = Self {
            source,
            sop_instance_uid: new_dicom_uid(),
            series_instance_uid: new_dicom_uid(),
            predecessor_sop_instance_uid: None,
            coordinate_type: "2D".into(),
            pixel_origin_interpretation: Some("VOLUME".into()),
            references_source_image: true,
            referenced_frame_number: None,
            content_label: "WSI_ANNOTATION".into(),
            content_description: "WSI vector annotations".into(),
            content_creator_name: None,
            producer: DerivedObjectProducer::library_default(9101, "WSI annotations"),
            groups,
            diagnostics: Vec::new(),
        };
        document.validate()?;
        Ok(document)
    }

    pub fn read_ann(path: impl AsRef<Path>, source: &DicomAnnotationContext) -> Result<Self> {
        read::read_ann(path.as_ref(), source)
    }

    pub fn write_ann(&self, path: impl AsRef<Path>) -> Result<()> {
        self.write_ann_with_loss_policy(path, false)
    }

    pub fn write_ann_with_loss_policy(
        &self,
        path: impl AsRef<Path>,
        allow_lossy: bool,
    ) -> Result<()> {
        write::write_ann(self, path.as_ref(), allow_lossy)
    }

    #[must_use]
    pub fn revised(&self) -> Self {
        let mut document = self.clone();
        document.sop_instance_uid = new_dicom_uid();
        document.predecessor_sop_instance_uid = Some(self.sop_instance_uid.clone());
        document
    }

    pub fn revised_with_groups(&self, groups: Vec<AnnotationGroup>) -> Result<Self> {
        let mut document = self.revised();
        document.groups = groups;
        document.validate()?;
        Ok(document)
    }

    /// Replaces the neutral library identity with caller-owned producer metadata.
    #[must_use]
    pub fn with_producer(mut self, producer: DerivedObjectProducer) -> Self {
        self.producer = producer;
        self
    }

    #[must_use]
    pub fn producer(&self) -> &DerivedObjectProducer {
        &self.producer
    }

    /// Atomically replaces all groups after validating group and document invariants.
    pub fn replace_groups(&mut self, groups: Vec<AnnotationGroup>) -> Result<()> {
        let mut candidate = self.clone();
        candidate.groups = groups;
        candidate.validate()?;
        self.groups = candidate.groups;
        Ok(())
    }

    /// Atomically replaces one group after validating group and document invariants.
    pub fn replace_group(&mut self, index: usize, group: AnnotationGroup) -> Result<()> {
        let mut groups = self.groups.clone();
        let group_count = groups.len();
        let destination = groups.get_mut(index).ok_or_else(|| {
            Error::InvalidInput(format!(
                "annotation group index {index} is outside the {}-group document",
                group_count
            ))
        })?;
        *destination = group;
        self.replace_groups(groups)
    }

    #[must_use]
    pub fn source(&self) -> &DicomAnnotationContext {
        &self.source
    }

    #[must_use]
    pub fn sop_instance_uid(&self) -> &str {
        &self.sop_instance_uid
    }

    #[must_use]
    pub fn series_instance_uid(&self) -> &str {
        &self.series_instance_uid
    }

    #[must_use]
    pub fn predecessor_sop_instance_uid(&self) -> Option<&str> {
        self.predecessor_sop_instance_uid.as_deref()
    }

    #[must_use]
    pub fn supports_2d_volume_editing(&self) -> bool {
        self.coordinate_type == "2D"
            && self.pixel_origin_interpretation.as_deref() == Some("VOLUME")
    }

    #[must_use]
    pub fn coordinate_type(&self) -> &str {
        &self.coordinate_type
    }

    #[must_use]
    pub fn pixel_origin_interpretation(&self) -> Option<&str> {
        self.pixel_origin_interpretation.as_deref()
    }

    #[must_use]
    pub const fn referenced_frame_number(&self) -> Option<u32> {
        self.referenced_frame_number
    }

    #[must_use]
    pub fn content_label(&self) -> &str {
        &self.content_label
    }

    #[must_use]
    pub fn content_description(&self) -> &str {
        &self.content_description
    }

    #[must_use]
    pub fn content_creator_name(&self) -> Option<&str> {
        self.content_creator_name.as_deref()
    }

    pub fn canonical_level0_pixel(
        &self,
        canonical_source: &DicomAnnotationContext,
        x: f64,
        y: f64,
        z: Option<f64>,
    ) -> Result<Point2> {
        if self.source.frame_of_reference_uid().is_none()
            || self.source.frame_of_reference_uid() != canonical_source.frame_of_reference_uid()
        {
            return Err(Error::InvalidInput(
                "annotation source and canonical source do not share a Frame of Reference UID"
                    .into(),
            ));
        }
        let canonical_pixel = if self.coordinate_type == "3D" {
            canonical_source.slide_coordinate_to_pixel3(
                x,
                y,
                z.ok_or_else(|| {
                    Error::InvalidInput("3D ANN canonicalization requires a Z coordinate".into())
                })?,
            )?
        } else {
            let source_pixel = match self.pixel_origin_interpretation.as_deref() {
                Some("VOLUME") => Point2::new(x, y),
                Some("FRAME") => self.source.frame_coordinate_to_total_pixel(
                    self.referenced_frame_number.ok_or_else(|| {
                        Error::InvalidInput("FRAME-relative ANN has no referenced frame".into())
                    })?,
                    x,
                    y,
                )?,
                value => {
                    return Err(Error::Unsupported(format!(
                        "cannot canonicalize Pixel Origin Interpretation {value:?}"
                    )))
                }
            };
            let slide = self
                .source
                .pixel_to_slide_coordinate(source_pixel.x, source_pixel.y)?;
            canonical_source.slide_coordinate_to_pixel3(slide.x, slide.y, slide.z)?
        };
        canonical_source.validate_point(canonical_pixel.x, canonical_pixel.y)?;
        Ok(canonical_pixel)
    }

    #[must_use]
    pub fn groups(&self) -> &[AnnotationGroup] {
        &self.groups
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[InteroperabilityDiagnostic] {
        &self.diagnostics
    }

    fn ensure_roundtrip_safe(&self, allow_lossy: bool) -> Result<()> {
        if allow_lossy {
            return Ok(());
        }
        let blocking = self
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.blocks_roundtrip())
            .map(InteroperabilityDiagnostic::code)
            .collect::<Vec<_>>();
        if blocking.is_empty() {
            Ok(())
        } else {
            Err(Error::Unsupported(format!(
                "ANN rewrite would lose semantics ({}); pass the explicit lossy override to continue",
                blocking.join(", ")
            )))
        }
    }

    fn validate(&self) -> Result<()> {
        if self.groups.is_empty() || self.groups.len() > MAX_ANNOTATION_GROUPS {
            return Err(Error::InvalidInput(format!(
                "an ANN document must contain 1..={MAX_ANNOTATION_GROUPS} groups"
            )));
        }
        let mut coordinate_values = 0_usize;
        let mut group_uids = BTreeSet::new();
        for group in &self.groups {
            group.validate()?;
            if !group_uids.insert(group.uid()) {
                return Err(Error::InvalidInput(format!(
                    "duplicate Annotation Group UID {}",
                    group.uid()
                )));
            }
            match group.geometry() {
                AnnotationGeometry::Points(points) => {
                    for point in points {
                        self.source.validate_point(point.x, point.y)?;
                    }
                    coordinate_values = coordinate_values.saturating_add(points.len() * 2);
                }
                AnnotationGeometry::Polygons(polygons) => {
                    for polygon in polygons {
                        for point in polygon {
                            self.source.validate_point(point.x, point.y)?;
                        }
                        coordinate_values =
                            coordinate_values.saturating_add(polygon.len().saturating_mul(2));
                    }
                }
                AnnotationGeometry::ReadOnly {
                    coordinates,
                    coordinate_dimensions,
                    ..
                } => {
                    if self.coordinate_type == "2D" {
                        if *coordinate_dimensions != 2 {
                            return Err(Error::InvalidInput(
                                "2D ANN geometry does not contain coordinate pairs".into(),
                            ));
                        }
                        for point in coordinates.chunks_exact(2) {
                            match self.pixel_origin_interpretation.as_deref() {
                                Some("VOLUME") => self.source.validate_point(point[0], point[1])?,
                                Some("FRAME") => {
                                    self.source.frame_coordinate_to_total_pixel(
                                        self.referenced_frame_number.ok_or_else(|| {
                                            Error::InvalidInput(
                                                "FRAME-relative ANN has no referenced frame".into(),
                                            )
                                        })?,
                                        point[0],
                                        point[1],
                                    )?;
                                }
                                _ => {
                                    return Err(Error::InvalidInput(
                                        "2D ANN has no valid Pixel Origin Interpretation".into(),
                                    ))
                                }
                            }
                        }
                    } else {
                        match *coordinate_dimensions {
                            2 if group.common_z_coordinates().is_empty() => {
                                return Err(Error::InvalidInput(
                                    "3D ANN coordinate pairs require Common Z Coordinate Value"
                                        .into(),
                                ));
                            }
                            3 if !group.common_z_coordinates().is_empty() => {
                                return Err(Error::InvalidInput(
                                    "3D ANN coordinate triplets cannot also contain Common Z Coordinate Value"
                                        .into(),
                                ));
                            }
                            2 | 3 => {}
                            _ => {
                                return Err(Error::InvalidInput(
                                    "3D ANN geometry must contain coordinate pairs or triplets"
                                        .into(),
                                ))
                            }
                        }
                    }
                    coordinate_values = coordinate_values.saturating_add(coordinates.len());
                }
            }
        }
        if coordinate_values > MAX_COORDINATE_VALUES {
            return Err(Error::InvalidInput(format!(
                "ANN contains {coordinate_values} coordinate values, exceeding the {MAX_COORDINATE_VALUES}-value limit"
            )));
        }
        Ok(())
    }
}
