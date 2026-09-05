use std::collections::{BTreeSet, HashMap};

use super::{
    SegToAnnConversionPolicy, SegmentationDocument, SegmentationKind, VectorizedAnnotations,
};
use crate::annotations::model::{
    AnnotationGroup, DiagnosticDisposition, DiagnosticSeverity, InteroperabilityDiagnostic, Point2,
};
use crate::{Error, Result};

pub(super) fn vectorized_annotations(
    document: &SegmentationDocument,
    policy: SegToAnnConversionPolicy,
) -> Result<VectorizedAnnotations> {
    if document.kind == SegmentationKind::Fractional {
        return Err(Error::Unsupported(
            "fractional SEG remains a read-only raster overlay".into(),
        ));
    }
    let segment_indices = segment_indices_by_number(document)?;
    let runs = document.binary_runs()?;
    let mut polygon_counts = vec![0_usize; document.segments.len()];
    for run in &runs {
        let segment_index = segment_indices.get(&run.segment_number).ok_or_else(|| {
            Error::InvalidInput(format!(
                "SEG run references missing segment {}",
                run.segment_number
            ))
        })?;
        polygon_counts[*segment_index] = polygon_counts[*segment_index]
            .checked_add(1)
            .ok_or_else(|| Error::InvalidInput("SEG polygon count overflows".into()))?;
    }
    let mut polygons_by_segment = polygon_counts
        .into_iter()
        .map(Vec::with_capacity)
        .collect::<Vec<_>>();
    for run in runs {
        let segment_index = segment_indices[&run.segment_number];
        let x0 = f64::from(run.column_start);
        let x1 = f64::from(
            run.column_start
                .checked_add(run.length)
                .ok_or_else(|| Error::InvalidInput("SEG run end overflows".into()))?,
        );
        let y0 = f64::from(run.row);
        let y1 = y0 + 1.0;
        polygons_by_segment[segment_index].push(vec![
            Point2::new(x0, y0),
            Point2::new(x1, y0),
            Point2::new(x1, y1),
            Point2::new(x0, y1),
        ]);
    }
    let mut diagnostics = document.diagnostics.clone();
    diagnostics.push(InteroperabilityDiagnostic::normalized(
        "SEG_RASTER_VECTORIZED",
        "$.PixelData",
        "projected the exact SEG raster into canonical pixel-edge ANN rectangles",
    ));
    let mut groups = Vec::with_capacity(document.segments.len());
    let mut group_uids = BTreeSet::new();
    for (index, (segment, polygons)) in document
        .segments
        .iter()
        .zip(polygons_by_segment)
        .enumerate()
    {
        let path = format!("SegmentSequence[{index}]");
        if polygons.is_empty() {
            diagnostics.push(InteroperabilityDiagnostic::new(
                "EMPTY_SEGMENT_NOT_VECTORIZED",
                DiagnosticSeverity::Warning,
                path,
                DiagnosticDisposition::WouldDrop,
                "segment has no nonzero pixels and cannot produce a nonempty ANN group",
            ));
            continue;
        }
        let mut group = AnnotationGroup::polygons(
            segment.label(),
            segment.category().clone(),
            segment.property_type().clone(),
            segment.recommended_display_cielab(),
            polygons,
        )?
        .with_description(segment.description())?
        .with_finding_semantics(segment.finding.clone())?;
        if let Some(number) = segment.source_segment_number() {
            diagnostics.push(InteroperabilityDiagnostic::new(
                "SEGMENT_NUMBER_NOT_REPRESENTABLE",
                DiagnosticSeverity::Warning,
                format!("{path}.SegmentNumber"),
                DiagnosticDisposition::WouldDrop,
                format!("source Segment Number {number} has no ANN equivalent"),
            ));
        }
        if segment.tracking_id().is_some() {
            diagnostics.push(InteroperabilityDiagnostic::new(
                "SEG_TRACKING_ID_NOT_REPRESENTABLE",
                DiagnosticSeverity::Warning,
                format!("{path}.TrackingID"),
                DiagnosticDisposition::WouldDrop,
                "SEG Tracking ID has no ANN group attribute; Tracking UID is reused as the Annotation Group UID when valid",
            ));
        }
        if let Some(tracking_uid) = segment.tracking_uid() {
            match group.clone().with_uid(tracking_uid) {
                Ok(tracked) if group_uids.insert(tracking_uid.to_string()) => group = tracked,
                _ => diagnostics.push(InteroperabilityDiagnostic::new(
                    "SEG_TRACKING_UID_NOT_REUSABLE",
                    DiagnosticSeverity::Warning,
                    format!("{path}.TrackingUID"),
                    DiagnosticDisposition::WouldDrop,
                    "SEG Tracking UID is invalid or duplicated and cannot become the ANN Annotation Group UID",
                )),
            }
        } else {
            group_uids.insert(group.uid().to_string());
        }
        groups.push(group);
    }
    let blocking = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.blocks_roundtrip())
        .map(InteroperabilityDiagnostic::code)
        .collect::<Vec<_>>();
    if policy == SegToAnnConversionPolicy::RejectLoss && !blocking.is_empty() {
        return Err(Error::Unsupported(format!(
            "SEG-to-ANN projection would lose semantics ({}); use SegToAnnConversionPolicy::AllowLoss to receive groups with diagnostics",
            blocking.join(", ")
        )));
    }
    Ok(VectorizedAnnotations {
        groups,
        diagnostics,
    })
}

fn segment_indices_by_number(document: &SegmentationDocument) -> Result<HashMap<u16, usize>> {
    let mut indices = HashMap::with_capacity(document.segments.len());
    for (index, segment) in document.segments.iter().enumerate() {
        let number = segment.source_segment_number.unwrap_or(
            u16::try_from(index + 1)
                .map_err(|_| Error::InvalidInput("segment number exceeds US range".into()))?,
        );
        if indices.insert(number, index).is_some() {
            return Err(Error::InvalidInput(format!(
                "SEG contains duplicate segment number {number}"
            )));
        }
    }
    Ok(indices)
}
