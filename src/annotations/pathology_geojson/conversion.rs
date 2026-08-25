use crate::{Error, Result};

use super::feature::ProfiledFeature;
use super::geometry::ProfiledGeometry;
use super::PathologyAnnotationSet;
use crate::annotations::ann::AnnotationDocument;
use crate::annotations::model::{
    AnnotationGeometry, AnnotationGraphicType, AnnotationGroup, AnnotationMeasurement,
};
use crate::annotations::seg::{SegmentationDocument, SegmentationSegment};

impl PathologyAnnotationSet {
    /// Build one DICOM ANN group per source feature so feature identity remains exact.
    pub fn to_ann(&self) -> Result<AnnotationDocument> {
        self.build_ann(false)
    }

    /// Build ANN geometry and measurements when a companion SR preserves coded evaluations.
    pub fn to_ann_with_companion_sr(&self) -> Result<AnnotationDocument> {
        self.build_ann(true)
    }

    fn build_ann(&self, companion_sr: bool) -> Result<AnnotationDocument> {
        let groups = self
            .features
            .iter()
            .map(|feature| annotation_group(feature, companion_sr))
            .collect::<Result<Vec<_>>>()?;
        AnnotationDocument::new(self.source.clone(), groups)
    }

    /// Build a sparse binary SEG with one segment per source feature.
    ///
    /// `companion_sr` must be true when mapped measurements or qualitative
    /// evaluations need the companion SR object selected by the caller.
    pub fn to_seg(&self, companion_sr: bool) -> Result<SegmentationDocument> {
        if !companion_sr
            && self.features.iter().any(|feature| {
                !feature.measurements.is_empty() || !feature.qualitative_evaluations.is_empty()
            })
        {
            return Err(Error::InvalidInput(
                "SEG cannot carry mapped measurements or conclusions without a companion SR target"
                    .into(),
            ));
        }
        let segments = self
            .features
            .iter()
            .map(segmentation_segment)
            .collect::<Result<Vec<_>>>()?;
        SegmentationDocument::binary(self.source.clone(), segments)
    }
}

fn annotation_group(feature: &ProfiledFeature, companion_sr: bool) -> Result<AnnotationGroup> {
    if !companion_sr && !feature.qualitative_evaluations.is_empty() {
        return Err(Error::InvalidInput(format!(
            "feature {:?} has coded qualitative evaluations that require an SR target",
            feature.tracking_id
        )));
    }
    let geometry = ann_geometry(feature)?;
    let annotation_count = geometry.annotation_count();
    if annotation_count != 1 && !feature.measurements.is_empty() && !companion_sr {
        return Err(Error::InvalidInput(format!(
            "feature {:?} contains {annotation_count} ANN primitives, so its feature-level measurements require SR",
            feature.tracking_id
        )));
    }
    let measurements = if annotation_count == 1 {
        feature
            .measurements
            .iter()
            .map(|measurement| {
                AnnotationMeasurement::new(
                    measurement.mapping.concept.clone(),
                    measurement.mapping.unit.clone(),
                    vec![measurement.value],
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    let semantics = &feature.semantics;
    AnnotationGroup::from_parts(
        feature.tracking_uid.clone(),
        feature.name.clone(),
        String::new(),
        semantics.finding.clone(),
        semantics.all_optical_paths,
        semantics.optical_paths.clone(),
        semantics.all_z_planes,
        semantics.z_coordinates_mm.clone(),
        geometry,
        measurements,
    )
}

fn ann_geometry(feature: &ProfiledFeature) -> Result<AnnotationGeometry> {
    match &feature.geometry {
        ProfiledGeometry::Points(points) => Ok(AnnotationGeometry::Points(points.clone())),
        ProfiledGeometry::Lines(lines) => polyline_geometry(lines),
        ProfiledGeometry::Polygons(polygons) => {
            if polygons.iter().any(|polygon| !polygon.holes.is_empty()) {
                return Err(Error::InvalidInput(format!(
                    "feature {:?}: ANN cannot represent polygon interior rings; select SEG",
                    feature.tracking_id
                )));
            }
            Ok(AnnotationGeometry::Polygons(
                polygons
                    .iter()
                    .map(|polygon| polygon.outer.clone())
                    .collect(),
            ))
        }
    }
}

fn polyline_geometry(lines: &[Vec<crate::Point2>]) -> Result<AnnotationGeometry> {
    let mut coordinates = Vec::new();
    let mut primitive_point_indices = Vec::with_capacity(lines.len());
    for line in lines {
        primitive_point_indices.push(u32::try_from(coordinates.len() + 1).map_err(|_| {
            Error::InvalidInput("ANN polyline coordinate index exceeds DICOM OL range".into())
        })?);
        coordinates.extend(line.iter().flat_map(|point| [point.x, point.y]));
    }
    Ok(AnnotationGeometry::ReadOnly {
        graphic_type: AnnotationGraphicType::Polyline,
        coordinates,
        primitive_point_indices,
        coordinate_dimensions: 2,
    })
}

fn segmentation_segment(feature: &ProfiledFeature) -> Result<SegmentationSegment> {
    let ProfiledGeometry::Polygons(polygons) = &feature.geometry else {
        return Err(Error::InvalidInput(format!(
            "feature {:?}: SEG accepts only Polygon and MultiPolygon geometry",
            feature.tracking_id
        )));
    };
    let semantics = &feature.semantics;
    if !semantics.all_optical_paths || !semantics.all_z_planes {
        return Err(Error::Unsupported(format!(
            "feature {:?}: the current SEG writer cannot preserve restricted optical-path or Z applicability",
            feature.tracking_id
        )));
    }
    let outer_polygons = polygons
        .iter()
        .map(|polygon| polygon.outer.clone())
        .collect();
    let component_holes = polygons
        .iter()
        .map(|polygon| polygon.holes.clone())
        .collect();
    let segment = SegmentationSegment::new(
        semantics.segment_label.clone(),
        semantics.finding.category().clone(),
        semantics.finding.property_type().clone(),
        semantics.finding.recommended_display_cielab(),
        outer_polygons,
        Vec::new(),
    )?
    .with_component_holes(component_holes)?
    .with_description(feature.name.clone())?
    .with_finding_semantics(semantics.finding.clone())?
    .with_tracking(feature.tracking_id.clone(), feature.tracking_uid.clone())?;
    Ok(segment)
}
