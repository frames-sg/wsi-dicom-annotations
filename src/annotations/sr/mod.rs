mod reader;
mod writer;

use std::path::Path;

use crate::{Error, Result};

use super::context::DicomAnnotationContext;
use super::derived_object::DerivedObjectProducer;
use super::dicom_dataset::new_dicom_uid;
use super::model::{AlgorithmIdentification, DicomCode, Point2, Point3, TrackingIdentity};
use super::seg::SegmentationDocument;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredReportReferenceKind {
    Point,
    Polygon,
    Segmentation,
    MeasurementCoordinates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateGraphic {
    Point,
    Multipoint,
    Polyline,
    Polygon,
}

impl CoordinateGraphic {
    #[must_use]
    pub const fn dicom_value(self) -> &'static str {
        match self {
            Self::Point => "POINT",
            Self::Multipoint => "MULTIPOINT",
            Self::Polyline => "POLYLINE",
            Self::Polygon => "POLYGON",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpatialCoordinates {
    pub(crate) graphic: CoordinateGraphic,
    pub(crate) points: Vec<Point3>,
    pub(crate) frame_of_reference_uid: String,
}

impl SpatialCoordinates {
    #[must_use]
    pub const fn graphic(&self) -> CoordinateGraphic {
        self.graphic
    }

    #[must_use]
    pub fn points(&self) -> &[Point3] {
        &self.points
    }

    #[must_use]
    pub fn frame_of_reference_uid(&self) -> &str {
        &self.frame_of_reference_uid
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SegmentationReference {
    pub(crate) sop_instance_uid: String,
    pub(crate) series_instance_uid: String,
    pub(crate) segment_number: u16,
    pub(crate) frame_numbers: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RegionReference {
    Coordinates(SpatialCoordinates),
    Segmentation(SegmentationReference),
    MeasurementCoordinates,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructuredReportMeasurement {
    pub(crate) concept: DicomCode,
    pub(crate) unit: DicomCode,
    pub(crate) value: f64,
    pub(crate) coordinates: Vec<SpatialCoordinates>,
}

impl StructuredReportMeasurement {
    #[must_use]
    pub fn concept(&self) -> &DicomCode {
        &self.concept
    }

    #[must_use]
    pub fn unit(&self) -> &DicomCode {
        &self.unit
    }

    #[must_use]
    pub const fn value(&self) -> f64 {
        self.value
    }

    #[must_use]
    pub fn coordinates(&self) -> &[SpatialCoordinates] {
        &self.coordinates
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructuredReportQualitativeEvaluation {
    pub(crate) concept: DicomCode,
    pub(crate) value: DicomCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementReportSemantics {
    report_title: DicomCode,
    procedures_reported: Vec<DicomCode>,
    length_concept: DicomCode,
    length_unit: DicomCode,
}

impl MeasurementReportSemantics {
    pub fn new(
        report_title: DicomCode,
        procedures_reported: Vec<DicomCode>,
        length_concept: DicomCode,
        length_unit: DicomCode,
    ) -> Result<Self> {
        if procedures_reported.is_empty() {
            return Err(Error::InvalidInput(
                "measurement report semantics require at least one procedure".into(),
            ));
        }
        Ok(Self {
            report_title,
            procedures_reported,
            length_concept,
            length_unit,
        })
    }

    #[must_use]
    pub fn pathology_v1() -> Self {
        Self::new(
            DicomCode::new("126000", "DCM", "Imaging Measurement Report")
                .expect("fixed report title code is valid"),
            vec![DicomCode::new("252416005", "SCT", "Histopathology test")
                .expect("fixed pathology procedure code is valid")],
            DicomCode::new("410668003", "SCT", "Length").expect("fixed length code is valid"),
            DicomCode::new("mm", "UCUM", "mm").expect("fixed millimeter code is valid"),
        )
        .expect("fixed pathology measurement semantics are valid")
    }

    #[must_use]
    pub fn report_title(&self) -> &DicomCode {
        &self.report_title
    }

    #[must_use]
    pub fn procedures_reported(&self) -> &[DicomCode] {
        &self.procedures_reported
    }

    #[must_use]
    pub fn length_concept(&self) -> &DicomCode {
        &self.length_concept
    }

    #[must_use]
    pub fn length_unit(&self) -> &DicomCode {
        &self.length_unit
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearMeasurementSpec {
    tracking: TrackingIdentity,
    finding_category: DicomCode,
    finding_type: DicomCode,
    finding_sites: Vec<DicomCode>,
    endpoints: [Point2; 2],
}

impl LinearMeasurementSpec {
    pub fn new(
        tracking: TrackingIdentity,
        finding_category: DicomCode,
        finding_type: DicomCode,
        start: Point2,
        end: Point2,
    ) -> Result<Self> {
        if !start.x.is_finite() || !start.y.is_finite() || !end.x.is_finite() || !end.y.is_finite()
        {
            return Err(Error::InvalidInput(
                "linear measurement endpoints must be finite".into(),
            ));
        }
        if start == end {
            return Err(Error::InvalidInput(
                "linear measurement requires distinct endpoints".into(),
            ));
        }
        Ok(Self {
            tracking,
            finding_category,
            finding_type,
            finding_sites: Vec::new(),
            endpoints: [start, end],
        })
    }

    #[must_use]
    pub fn with_finding_sites(mut self, finding_sites: Vec<DicomCode>) -> Self {
        self.finding_sites = finding_sites;
        self
    }

    #[must_use]
    pub fn tracking(&self) -> &TrackingIdentity {
        &self.tracking
    }

    #[must_use]
    pub fn finding_category(&self) -> &DicomCode {
        &self.finding_category
    }

    #[must_use]
    pub fn finding_type(&self) -> &DicomCode {
        &self.finding_type
    }

    #[must_use]
    pub fn finding_sites(&self) -> &[DicomCode] {
        &self.finding_sites
    }

    #[must_use]
    pub const fn endpoints(&self) -> [Point2; 2] {
        self.endpoints
    }
}

impl StructuredReportQualitativeEvaluation {
    #[must_use]
    pub fn concept(&self) -> &DicomCode {
        &self.concept
    }

    #[must_use]
    pub fn value(&self) -> &DicomCode {
        &self.value
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructuredReportMeasurementGroup {
    pub(crate) template_id: &'static str,
    pub(crate) tracking_id: String,
    pub(crate) tracking_uid: String,
    pub(crate) finding_category: DicomCode,
    pub(crate) finding_type: DicomCode,
    pub(crate) algorithms: Vec<AlgorithmIdentification>,
    pub(crate) finding_sites: Vec<DicomCode>,
    pub(crate) reference: RegionReference,
    pub(crate) measurements: Vec<StructuredReportMeasurement>,
    pub(crate) qualitative_evaluations: Vec<StructuredReportQualitativeEvaluation>,
}

impl StructuredReportMeasurementGroup {
    #[must_use]
    pub fn tracking_id(&self) -> &str {
        &self.tracking_id
    }

    #[must_use]
    pub fn tracking_uid(&self) -> &str {
        &self.tracking_uid
    }

    #[must_use]
    pub fn finding_category(&self) -> &DicomCode {
        &self.finding_category
    }

    #[must_use]
    pub fn finding_type(&self) -> &DicomCode {
        &self.finding_type
    }

    #[must_use]
    pub fn algorithms(&self) -> &[AlgorithmIdentification] {
        &self.algorithms
    }

    #[must_use]
    pub fn finding_sites(&self) -> &[DicomCode] {
        &self.finding_sites
    }

    #[must_use]
    pub fn region_coordinates(&self) -> Option<&SpatialCoordinates> {
        match &self.reference {
            RegionReference::Coordinates(coordinates) => Some(coordinates),
            RegionReference::Segmentation(_) | RegionReference::MeasurementCoordinates => None,
        }
    }

    #[must_use]
    pub const fn reference_kind(&self) -> StructuredReportReferenceKind {
        match &self.reference {
            RegionReference::Coordinates(coordinates) => match coordinates.graphic {
                CoordinateGraphic::Point => StructuredReportReferenceKind::Point,
                CoordinateGraphic::Polygon => StructuredReportReferenceKind::Polygon,
                CoordinateGraphic::Multipoint | CoordinateGraphic::Polyline => {
                    StructuredReportReferenceKind::MeasurementCoordinates
                }
            },
            RegionReference::Segmentation(_) => StructuredReportReferenceKind::Segmentation,
            RegionReference::MeasurementCoordinates => {
                StructuredReportReferenceKind::MeasurementCoordinates
            }
        }
    }

    #[must_use]
    pub fn referenced_segmentation_uid(&self) -> Option<&str> {
        match &self.reference {
            RegionReference::Segmentation(reference) => Some(&reference.sop_instance_uid),
            _ => None,
        }
    }

    #[must_use]
    pub const fn referenced_segment_number(&self) -> Option<u16> {
        match &self.reference {
            RegionReference::Segmentation(reference) => Some(reference.segment_number),
            _ => None,
        }
    }

    #[must_use]
    pub fn measurements(&self) -> &[StructuredReportMeasurement] {
        &self.measurements
    }

    #[must_use]
    pub fn qualitative_evaluations(&self) -> &[StructuredReportQualitativeEvaluation] {
        &self.qualitative_evaluations
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructuredReportDocument {
    pub(crate) source: DicomAnnotationContext,
    pub(crate) sop_instance_uid: String,
    pub(crate) series_instance_uid: String,
    pub(crate) device_observer_uid: String,
    pub(crate) producer: DerivedObjectProducer,
    pub(crate) report_title: DicomCode,
    pub(crate) procedures_reported: Vec<DicomCode>,
    pub(crate) groups: Vec<StructuredReportMeasurementGroup>,
    pub(crate) completion_flag: String,
    pub(crate) verification_flag: String,
    pub(crate) preliminary_flag: String,
}

impl StructuredReportDocument {
    pub(crate) fn new(
        source: DicomAnnotationContext,
        report_title: DicomCode,
        procedures_reported: Vec<DicomCode>,
        groups: Vec<StructuredReportMeasurementGroup>,
    ) -> Result<Self> {
        if procedures_reported.is_empty() || groups.is_empty() {
            return Err(Error::InvalidInput(
                "TID 1500 requires a procedure reported and at least one measurement group".into(),
            ));
        }
        if source.frame_of_reference_uid().is_none() {
            return Err(Error::InvalidInput(
                "Comprehensive 3D SR coordinates require a source Frame of Reference UID".into(),
            ));
        }
        Ok(Self {
            source,
            sop_instance_uid: new_dicom_uid(),
            series_instance_uid: new_dicom_uid(),
            device_observer_uid: new_dicom_uid(),
            producer: DerivedObjectProducer::library_default(9301, "WSI measurement reports"),
            report_title,
            procedures_reported,
            groups,
            completion_flag: "COMPLETE".into(),
            verification_flag: "UNVERIFIED".into(),
            preliminary_flag: "PRELIMINARY".into(),
        })
    }

    pub fn from_linear_measurements(
        source: DicomAnnotationContext,
        semantics: &MeasurementReportSemantics,
        measurements: &[LinearMeasurementSpec],
    ) -> Result<Self> {
        if measurements.is_empty() {
            return Err(Error::InvalidInput(
                "measurement report requires at least one linear measurement".into(),
            ));
        }
        let frame_of_reference_uid = source
            .frame_of_reference_uid()
            .ok_or_else(|| {
                Error::InvalidInput(
                    "Comprehensive 3D SR coordinates require a source Frame of Reference UID"
                        .into(),
                )
            })?
            .to_string();
        let groups = measurements
            .iter()
            .map(|spec| {
                let endpoints = spec.endpoints();
                let points = endpoints
                    .into_iter()
                    .map(|point| source.pixel_to_slide_coordinate(point.x, point.y))
                    .collect::<Result<Vec<_>>>()?;
                let delta = Point3::new(
                    points[1].x - points[0].x,
                    points[1].y - points[0].y,
                    points[1].z - points[0].z,
                );
                let value = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
                if !value.is_finite() || value <= 0.0 {
                    return Err(Error::InvalidInput(
                        "linear measurement has no positive physical length".into(),
                    ));
                }
                let coordinates = SpatialCoordinates {
                    graphic: CoordinateGraphic::Polyline,
                    points,
                    frame_of_reference_uid: frame_of_reference_uid.clone(),
                };
                Ok(StructuredReportMeasurementGroup {
                    template_id: "1501",
                    tracking_id: spec.tracking.id().to_string(),
                    tracking_uid: spec.tracking.uid().to_string(),
                    finding_category: spec.finding_category.clone(),
                    finding_type: spec.finding_type.clone(),
                    algorithms: Vec::new(),
                    finding_sites: spec.finding_sites.clone(),
                    reference: RegionReference::MeasurementCoordinates,
                    measurements: vec![StructuredReportMeasurement {
                        concept: semantics.length_concept.clone(),
                        unit: semantics.length_unit.clone(),
                        value,
                        coordinates: vec![coordinates],
                    }],
                    qualitative_evaluations: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Self::new(
            source,
            semantics.report_title.clone(),
            semantics.procedures_reported.clone(),
            groups,
        )
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

    #[must_use]
    pub fn source(&self) -> &DicomAnnotationContext {
        &self.source
    }

    pub fn read_sr(
        path: impl AsRef<Path>,
        source: &DicomAnnotationContext,
        segmentation: Option<&SegmentationDocument>,
    ) -> Result<Self> {
        reader::read_sr(path.as_ref(), source, segmentation)
    }

    pub fn write_sr(&self, path: impl AsRef<Path>) -> Result<()> {
        writer::write_sr(self, path.as_ref())
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
    pub fn groups(&self) -> &[StructuredReportMeasurementGroup] {
        &self.groups
    }

    #[must_use]
    pub fn report_title(&self) -> &DicomCode {
        &self.report_title
    }

    #[must_use]
    pub fn procedures_reported(&self) -> &[DicomCode] {
        &self.procedures_reported
    }

    #[must_use]
    pub fn completion_flag(&self) -> &str {
        &self.completion_flag
    }

    #[must_use]
    pub fn verification_flag(&self) -> &str {
        &self.verification_flag
    }

    #[must_use]
    pub fn preliminary_flag(&self) -> &str {
        &self.preliminary_flag
    }
}
