mod ann;
mod coded_content;
mod context;
mod derived_object;
mod dicom_dataset;
mod dicom_file;
mod dicom_value;
mod json;
mod model;
#[cfg(feature = "parametric-map")]
mod parametric_map;
mod pathology_geojson;
mod profile;
mod publication;
mod scheme;
mod seg;
mod semantic_digest;
mod sidecar;
mod sr;

#[cfg(test)]
mod provenance_tests;

pub use ann::AnnotationDocument;
pub use context::DicomAnnotationContext;
pub use derived_object::DerivedObjectProducer;
pub use json::validate_unique_object_keys;
pub use model::{
    polygon_boundaries_intersect, polygon_contains_point, polygon_self_intersects,
    polygon_signed_area, validate_polygon, AlgorithmIdentification, AnnotationGeometry,
    AnnotationGraphicType, AnnotationGroup, AnnotationMeasurement, DiagnosticDisposition,
    DiagnosticSeverity, DicomCode, DicomCodeValueKind, GenerationType, InteroperabilityDiagnostic,
    Point2, Point3, TrackingIdentity,
};
#[cfg(feature = "parametric-map")]
pub use parametric_map::{
    ParametricMapDocument, ParametricMapInstance, ParametricMapPartPlan, ParametricMapPlan,
    ParametricMapPreview, RasterChannelSelection, RasterInputFormat, RasterProfile,
};
pub use pathology_geojson::{
    PathologyAnnotationSet, PathologyCoordinateSpace, PathologyDicomDocuments,
    PathologyDicomTarget, PathologyDocumentWriteError, PathologyGeometryKind, PathologyPreview,
    PathologyPreviewFeature, PathologyPreviewGeometry, PathologyPreviewPolygon,
};
pub use publication::{DicomBundlePublication, DicomPublicationError, DicomSinglePublication};
pub use scheme::{
    annotation_class_concept_key, dicom_cielab_to_srgb, srgb_to_dicom_cielab, AnnotationClass,
    AnnotationClassConceptKey, AnnotationClassGeometry, AnnotationScheme,
};
pub use seg::{
    BinaryMaskRun, BinarySegmentationFrame, FractionalMaskRun, FractionalSegmentationFrame,
    SegToAnnConversionPolicy, SegmentationDocument, SegmentationKind, SegmentationSegment,
    VectorizedAnnotations,
};
pub use sidecar::{
    annotation_object_kind, discover_sidecars, AnnotationObjectKind, SidecarKind, SidecarMetadata,
};
pub use sr::{
    CoordinateGraphic, LinearMeasurementSpec, MeasurementReportSemantics, SpatialCoordinates,
    StructuredReportDocument, StructuredReportMeasurement, StructuredReportMeasurementGroup,
    StructuredReportQualitativeEvaluation, StructuredReportReferenceKind,
};
