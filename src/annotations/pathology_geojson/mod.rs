mod conversion;
mod documents;
mod feature;
mod geometry;
mod mapping;
mod preview;
mod sr_conversion;

use crate::{Error, Result};
use sha2::{Digest, Sha256};

use super::context::DicomAnnotationContext;
use super::model::InteroperabilityDiagnostic;
use super::semantic_digest::{finish, update_text};
pub use documents::{PathologyDicomDocuments, PathologyDicomTarget, PathologyDocumentWriteError};
use feature::{parse_feature_collection, ProfiledFeature};
use mapping::{Mapping, StructuredReportMapping};
pub use preview::{
    PathologyPreview, PathologyPreviewFeature, PathologyPreviewGeometry, PathologyPreviewPolygon,
};

const MAX_GEOJSON_BYTES: usize = 64 * 1024 * 1024;
const MAX_MAPPING_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathologyCoordinateSpace {
    Level0Pixels,
    SourcePixels,
    SlideMillimeters,
}

impl PathologyCoordinateSpace {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Level0Pixels => "level0-pixels",
            Self::SourcePixels => "source-pixels",
            Self::SlideMillimeters => "slide-mm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathologyGeometryKind {
    Point,
    MultiPoint,
    LineString,
    MultiLineString,
    Polygon,
    MultiPolygon,
}

#[derive(Debug)]
pub struct PathologyAnnotationSet {
    source: DicomAnnotationContext,
    features: Vec<ProfiledFeature>,
    diagnostics: Vec<InteroperabilityDiagnostic>,
    sr_mapping: Option<StructuredReportMapping>,
}

impl PathologyAnnotationSet {
    /// Parse and validate pathology-profiled GeoJSON and its explicit DICOM mapping.
    ///
    /// Coordinates are transformed into the referenced source WSI's Total Pixel
    /// Matrix coordinate system. `canonical_source` is used only when the declared
    /// coordinate space is level-zero pixels.
    pub fn from_json(
        geojson: &[u8],
        mapping: &[u8],
        source: &DicomAnnotationContext,
        canonical_source: &DicomAnnotationContext,
        coordinate_space: PathologyCoordinateSpace,
        allow_lossy: bool,
    ) -> Result<Self> {
        if geojson.len() > MAX_GEOJSON_BYTES {
            return Err(Error::InvalidInput(format!(
                "GeoJSON input exceeds the {MAX_GEOJSON_BYTES}-byte limit"
            )));
        }
        if mapping.len() > MAX_MAPPING_BYTES {
            return Err(Error::InvalidInput(format!(
                "mapping profile exceeds the {MAX_MAPPING_BYTES}-byte limit"
            )));
        }
        validate_coordinate_contexts(source, canonical_source, coordinate_space)?;
        let mapping = Mapping::from_json(mapping)?;
        let (features, diagnostics) = parse_feature_collection(
            geojson,
            &mapping,
            source,
            canonical_source,
            coordinate_space,
            allow_lossy,
        )?;
        Ok(Self {
            source: source.clone(),
            features,
            diagnostics,
            sr_mapping: mapping.sr,
        })
    }

    #[must_use]
    pub fn feature_count(&self) -> usize {
        self.features.len()
    }

    #[must_use]
    pub fn feature_geometry_kind(&self, index: usize) -> Option<PathologyGeometryKind> {
        self.features.get(index).map(|feature| feature.kind)
    }

    #[must_use]
    pub fn feature_tracking_id(&self, index: usize) -> Option<&str> {
        self.features
            .get(index)
            .map(|feature| feature.tracking_id.as_str())
    }

    #[must_use]
    pub fn feature_tracking_uid(&self, index: usize) -> Option<&str> {
        self.features
            .get(index)
            .map(|feature| feature.tracking_uid.as_str())
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[InteroperabilityDiagnostic] {
        &self.diagnostics
    }

    /// Consume the normalized annotation set and transfer its geometry into a
    /// read-only viewer preview without retaining a second full geometry copy.
    /// Mixed feature collections and polygon interior rings remain intact, and
    /// coordinates are in the source WSI's Total Pixel Matrix space.
    #[must_use]
    pub fn into_preview(self) -> PathologyPreview {
        PathologyPreview::from_owned_features(self.features)
    }

    /// SHA-256 over the normalized profile semantics and source-space geometry.
    #[must_use]
    pub fn semantic_sha256(&self) -> String {
        let mut digest = Sha256::new();
        update_text(&mut digest, "pathology-geojson-semantics-v1");
        update_text(&mut digest, self.source.sop_instance_uid());
        for feature in &self.features {
            feature.update_semantic_digest(&mut digest);
        }
        digest.update([u8::from(self.sr_mapping.is_some())]);
        if let Some(mapping) = &self.sr_mapping {
            mapping.update_digest(&mut digest);
        }
        finish(digest)
    }
}

fn validate_coordinate_contexts(
    source: &DicomAnnotationContext,
    canonical_source: &DicomAnnotationContext,
    coordinate_space: PathologyCoordinateSpace,
) -> Result<()> {
    if coordinate_space != PathologyCoordinateSpace::Level0Pixels {
        return Ok(());
    }
    source.require_shared_slide_frame(canonical_source)
}
