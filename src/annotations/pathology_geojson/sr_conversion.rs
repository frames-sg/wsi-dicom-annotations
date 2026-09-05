use std::collections::BTreeMap;

use crate::{Error, Point2, Result};

use super::feature::ProfiledFeature;
use super::geometry::ProfiledGeometry;
use super::{PathologyAnnotationSet, PathologyGeometryKind};
use crate::annotations::seg::SegmentationDocument;
use crate::annotations::sr::{
    CoordinateGraphic, RegionReference, SegmentationReference, SpatialCoordinates,
    StructuredReportDocument, StructuredReportMeasurement, StructuredReportMeasurementGroup,
    StructuredReportQualitativeEvaluation,
};

impl PathologyAnnotationSet {
    /// Build a Comprehensive 3D SR measurement report.
    ///
    /// Polygon holes and disconnected polygon components require a companion SEG
    /// from this annotation set. Lines and multipoints are represented only as
    /// coordinate references for mapped numeric measurements.
    pub fn to_sr(
        &self,
        segmentation: Option<&SegmentationDocument>,
    ) -> Result<StructuredReportDocument> {
        let mapping = self.sr_mapping.as_ref().ok_or_else(|| {
            Error::InvalidInput("SR target requires an sr section in the mapping profile".into())
        })?;
        let segmentation_frames = segmentation
            .map(|document| segmentation_frame_numbers(document, &self.source))
            .transpose()?;
        let groups = self
            .features
            .iter()
            .map(|feature| {
                measurement_group(
                    feature,
                    &self.source,
                    segmentation,
                    segmentation_frames.as_ref(),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        StructuredReportDocument::new(
            self.source.clone(),
            mapping.report_title.clone(),
            mapping.procedures_reported.clone(),
            groups,
        )
    }
}

fn measurement_group(
    feature: &ProfiledFeature,
    source: &crate::DicomAnnotationContext,
    segmentation: Option<&SegmentationDocument>,
    segmentation_frames: Option<&BTreeMap<u16, Vec<u32>>>,
) -> Result<StructuredReportMeasurementGroup> {
    let (template_id, reference, measurement_coordinates) = match feature.kind {
        PathologyGeometryKind::Point => (
            "1410",
            RegionReference::Coordinates(direct_coordinates(feature, source)?),
            Vec::new(),
        ),
        PathologyGeometryKind::Polygon if polygon_is_simple(feature) => (
            "1410",
            RegionReference::Coordinates(direct_coordinates(feature, source)?),
            Vec::new(),
        ),
        PathologyGeometryKind::Polygon | PathologyGeometryKind::MultiPolygon => (
            "1410",
            RegionReference::Segmentation(segmentation_reference(
                feature,
                segmentation,
                segmentation_frames,
            )?),
            Vec::new(),
        ),
        PathologyGeometryKind::MultiPoint
        | PathologyGeometryKind::LineString
        | PathologyGeometryKind::MultiLineString => {
            if feature.measurements.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "feature {:?}: multipoint and line geometry requires a mapped numeric measurement for lossless SR representation",
                    feature.tracking_id
                )));
            }
            (
                "1501",
                RegionReference::MeasurementCoordinates,
                measurement_coordinates(feature, source)?,
            )
        }
    };
    let measurements = feature
        .measurements
        .iter()
        .map(|measurement| StructuredReportMeasurement {
            concept: measurement.mapping.concept.clone(),
            unit: measurement.mapping.unit.clone(),
            value: measurement.value,
            coordinates: measurement_coordinates.clone(),
        })
        .collect();
    let qualitative_evaluations = feature
        .qualitative_evaluations
        .iter()
        .map(|evaluation| StructuredReportQualitativeEvaluation {
            concept: evaluation.concept.clone(),
            value: evaluation.value.clone(),
        })
        .collect();
    Ok(StructuredReportMeasurementGroup {
        template_id,
        tracking_id: feature.tracking_id.clone(),
        tracking_uid: feature.tracking_uid.clone(),
        finding_category: feature.semantics.finding.category().clone(),
        finding_type: feature.semantics.finding.property_type().clone(),
        algorithms: feature.semantics.finding.algorithms().to_vec(),
        finding_sites: feature.semantics.finding.anatomic_regions().to_vec(),
        reference,
        measurements,
        qualitative_evaluations,
    })
}

fn polygon_is_simple(feature: &ProfiledFeature) -> bool {
    matches!(
        &feature.geometry,
        ProfiledGeometry::Polygons(polygons)
            if polygons.len() == 1 && polygons[0].holes.is_empty()
    )
}

fn direct_coordinates(
    feature: &ProfiledFeature,
    source: &crate::DicomAnnotationContext,
) -> Result<SpatialCoordinates> {
    let (graphic, points) = match &feature.geometry {
        ProfiledGeometry::Points(points) if points.len() == 1 => {
            (CoordinateGraphic::Point, points.as_slice())
        }
        ProfiledGeometry::Polygons(polygons)
            if polygons.len() == 1 && polygons[0].holes.is_empty() =>
        {
            (CoordinateGraphic::Polygon, polygons[0].outer.as_slice())
        }
        _ => {
            return Err(Error::InvalidInput(format!(
                "feature {:?} is not a directly representable TID 1410 region",
                feature.tracking_id
            )));
        }
    };
    spatial_coordinates(graphic, points, source)
}

fn measurement_coordinates(
    feature: &ProfiledFeature,
    source: &crate::DicomAnnotationContext,
) -> Result<Vec<SpatialCoordinates>> {
    match &feature.geometry {
        ProfiledGeometry::Points(points) => Ok(vec![spatial_coordinates(
            CoordinateGraphic::Multipoint,
            points,
            source,
        )?]),
        ProfiledGeometry::Lines(lines) => lines
            .iter()
            .map(|line| spatial_coordinates(CoordinateGraphic::Polyline, line, source))
            .collect(),
        ProfiledGeometry::Polygons(_) => Err(Error::InvalidInput(
            "polygon geometry cannot use TID 1501 measurement coordinates".into(),
        )),
    }
}

fn spatial_coordinates(
    graphic: CoordinateGraphic,
    points: &[Point2],
    source: &crate::DicomAnnotationContext,
) -> Result<SpatialCoordinates> {
    let frame_of_reference_uid = source
        .frame_of_reference_uid()
        .ok_or_else(|| Error::InvalidInput("source WSI has no Frame of Reference UID".into()))?;
    let points = points
        .iter()
        .map(|point| source.pixel_to_slide_coordinate(point.x, point.y))
        .collect::<Result<Vec<_>>>()?;
    Ok(SpatialCoordinates {
        graphic,
        points,
        frame_of_reference_uid: frame_of_reference_uid.to_string(),
    })
}

fn segmentation_frame_numbers(
    segmentation: &SegmentationDocument,
    source: &crate::DicomAnnotationContext,
) -> Result<BTreeMap<u16, Vec<u32>>> {
    if segmentation.source().study_instance_uid() != source.study_instance_uid()
        || segmentation.source().sop_instance_uid() != source.sop_instance_uid()
        || segmentation.source().frame_of_reference_uid() != source.frame_of_reference_uid()
    {
        return Err(Error::InvalidInput(
            "companion SEG does not reference the profiled source WSI".into(),
        ));
    }
    let mut by_segment = BTreeMap::<u16, Vec<u32>>::new();
    for (index, frame) in segmentation.rasterized_frames()?.iter().enumerate() {
        let frame_number = u32::try_from(index + 1)
            .map_err(|_| Error::InvalidInput("SEG frame number exceeds u32".into()))?;
        by_segment
            .entry(frame.segment_number())
            .or_default()
            .push(frame_number);
    }
    Ok(by_segment)
}

fn segmentation_reference(
    feature: &ProfiledFeature,
    segmentation: Option<&SegmentationDocument>,
    frames: Option<&BTreeMap<u16, Vec<u32>>>,
) -> Result<SegmentationReference> {
    let segmentation = segmentation.ok_or_else(|| {
        Error::InvalidInput(format!(
            "feature {:?}: polygon holes or disconnected components require a SEG target referenced by SR",
            feature.tracking_id
        ))
    })?;
    let (segment_index, segment) = segmentation
        .segments()
        .iter()
        .enumerate()
        .find(|(_, segment)| {
            segment.tracking_id() == Some(feature.tracking_id.as_str())
                && segment.tracking_uid() == Some(feature.tracking_uid.as_str())
        })
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "companion SEG has no segment matching feature {:?} tracking identity",
                feature.tracking_id
            ))
        })?;
    let segment_number = segment.source_segment_number().unwrap_or(
        u16::try_from(segment_index + 1)
            .map_err(|_| Error::InvalidInput("SEG segment number exceeds u16".into()))?,
    );
    let frame_numbers = frames
        .and_then(|frames| frames.get(&segment_number))
        .filter(|frames| !frames.is_empty())
        .cloned()
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "companion SEG segment {segment_number} has no rasterized frames"
            ))
        })?;
    Ok(SegmentationReference {
        sop_instance_uid: segmentation.sop_instance_uid().to_string(),
        series_instance_uid: segmentation.series_instance_uid().to_string(),
        segment_number,
        frame_numbers,
    })
}
