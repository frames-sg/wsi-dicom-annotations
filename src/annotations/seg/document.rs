use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use dicom_dictionary_std::{tags, uids};
use sha2::{Digest, Sha256};

use super::raster::{
    merge_binary_runs, merge_fractional_runs, rasterize_segments, scan_nonzero_ranges, RunBudget,
};
use super::read::{
    read_binary_frames, read_fractional_frames, read_labelmap_frames, read_segments,
    validate_source_reference,
};
use super::write::add_segmentation_attributes;
use super::{
    BinaryMaskRun, BinarySegmentationFrame, FractionalMaskRun, FractionalSegmentationFrame,
    SegToAnnConversionPolicy, SegmentationDocument, SegmentationKind, SegmentationSegment,
    VectorizedAnnotations, MAX_SEG_FILE_BYTES, MAX_VECTORIZED_RUNS,
};
use crate::annotations::context::DicomAnnotationContext;
use crate::annotations::derived_object::{
    add_common_instance_reference, build_common_object, DerivedObjectProducer,
};
use crate::annotations::dicom_dataset::{new_dicom_uid, optional_string, required_string};
use crate::annotations::dicom_file::{
    atomic_write_dicom, enforce_file_limit, ensure_sidecar_destination,
};
use crate::annotations::model::{
    AnnotationGroup, DiagnosticDisposition, DiagnosticSeverity, InteroperabilityDiagnostic, Point2,
};
use crate::{Error, Result};

impl SegmentationDocument {
    pub fn binary(
        source: DicomAnnotationContext,
        segments: Vec<SegmentationSegment>,
    ) -> Result<Self> {
        if segments.is_empty() || segments.len() > usize::from(u16::MAX) {
            return Err(Error::InvalidInput(
                "a binary SEG document must contain 1..=65535 segments".into(),
            ));
        }
        for segment in &segments {
            for point in segment
                .outer_polygons()
                .iter()
                .chain(segment.exclusion_polygons())
                .flatten()
            {
                source.validate_point(point.x, point.y)?;
            }
        }
        Ok(Self {
            source,
            kind: SegmentationKind::Binary,
            sop_instance_uid: new_dicom_uid(),
            series_instance_uid: new_dicom_uid(),
            content_label: "WSI_SEGMENTATION".into(),
            content_description: "WSI binary segmentation".into(),
            content_creator_name: None,
            producer: DerivedObjectProducer::library_default(9201, "WSI segmentations"),
            segments,
            imported_binary_frames: None,
            imported_fractional_frames: None,
            diagnostics: Vec::new(),
        })
    }

    pub fn read_seg(path: impl AsRef<Path>, source: &DicomAnnotationContext) -> Result<Self> {
        let path = path.as_ref();
        enforce_file_limit(path, MAX_SEG_FILE_BYTES, "SEG")?;
        let object = dicom_object::open_file(path).map_err(|error| Error::DicomRead {
            path: path.to_path_buf(),
            source: Box::new(error),
        })?;
        let sop_class = object.meta().media_storage_sop_class_uid();
        if sop_class != uids::SEGMENTATION_STORAGE
            && sop_class != uids::LABEL_MAP_SEGMENTATION_STORAGE
        {
            return Err(Error::Unsupported(format!(
                "{} is not a DICOM Segmentation instance",
                path.display()
            )));
        }
        validate_source_reference(&object, source)?;
        let segmentation_type = required_string(&object, tags::SEGMENTATION_TYPE)?;
        let kind = match segmentation_type.as_str() {
            "BINARY" => SegmentationKind::Binary,
            "LABELMAP" => SegmentationKind::LabelMap,
            "FRACTIONAL" => SegmentationKind::Fractional,
            other => {
                return Err(Error::Unsupported(format!(
                    "SEG type {other:?} is not recognized"
                )))
            }
        };
        if !matches!(
            object.meta().transfer_syntax(),
            uids::IMPLICIT_VR_LITTLE_ENDIAN | uids::EXPLICIT_VR_LITTLE_ENDIAN
        ) {
            return Err(Error::Unsupported(
                "compressed or big-endian SEG Pixel Data is not supported".into(),
            ));
        }
        let mut diagnostics = Vec::new();
        let segments = read_segments(
            &object,
            &mut diagnostics,
            kind == SegmentationKind::LabelMap,
        )?;
        let (imported_binary_frames, imported_fractional_frames) = match kind {
            SegmentationKind::Binary => {
                (Some(read_binary_frames(&object, source, &segments)?), None)
            }
            SegmentationKind::LabelMap => (
                Some(read_labelmap_frames(&object, source, &segments)?),
                None,
            ),
            SegmentationKind::Fractional => (
                None,
                Some(read_fractional_frames(&object, source, &segments)?),
            ),
        };
        Ok(Self {
            source: source.clone(),
            kind,
            sop_instance_uid: required_string(&object, tags::SOP_INSTANCE_UID)?,
            series_instance_uid: required_string(&object, tags::SERIES_INSTANCE_UID)?,
            content_label: required_string(&object, tags::CONTENT_LABEL)?,
            content_description: optional_string(&object, tags::CONTENT_DESCRIPTION)
                .unwrap_or_default(),
            content_creator_name: optional_string(&object, tags::CONTENT_CREATOR_NAME),
            producer: DerivedObjectProducer::read(&object),
            segments,
            imported_binary_frames,
            imported_fractional_frames,
            diagnostics,
        })
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

    pub fn rasterized_frames(&self) -> Result<Vec<BinarySegmentationFrame>> {
        Ok(self.binary_frames()?.into_owned())
    }

    fn binary_frames(&self) -> Result<Cow<'_, [BinarySegmentationFrame]>> {
        if let Some(frames) = &self.imported_binary_frames {
            return Ok(Cow::Borrowed(frames));
        }
        if self.kind != SegmentationKind::Binary {
            return Err(Error::Unsupported(
                "fractional SEG is a read-only raster overlay and cannot be vectorized as binary"
                    .into(),
            ));
        }
        Ok(Cow::Owned(rasterize_segments(
            &self.source,
            &self.segments,
        )?))
    }

    pub fn binary_runs(&self) -> Result<Vec<BinaryMaskRun>> {
        if self.kind == SegmentationKind::Fractional {
            return Err(Error::Unsupported(
                "fractional SEG does not contain binary mask runs".into(),
            ));
        }
        let frames = self.binary_frames()?;
        let mut runs = Vec::new();
        let mut budget = RunBudget::new(MAX_VECTORIZED_RUNS);
        for frame in frames.iter() {
            let base_column = frame
                .tile_col
                .checked_mul(u32::from(frame.width))
                .ok_or_else(|| Error::InvalidInput("SEG column position overflows".into()))?;
            let base_row = frame
                .tile_row
                .checked_mul(u32::from(frame.height))
                .ok_or_else(|| Error::InvalidInput("SEG row position overflows".into()))?;
            for row in 0..usize::from(frame.height) {
                let row_start = row * usize::from(frame.width);
                let values = &frame.mask[row_start..row_start + usize::from(frame.width)];
                let row = u32::try_from(row)
                    .map_err(|_| Error::InvalidInput("SEG row index exceeds DICOM range".into()))?;
                let absolute_row = base_row
                    .checked_add(row)
                    .ok_or_else(|| Error::InvalidInput("SEG row position overflows".into()))?;
                scan_nonzero_ranges(
                    values,
                    |value| *value,
                    &mut budget,
                    |range| {
                        let start = u32::try_from(range.start).map_err(|_| {
                            Error::InvalidInput("SEG column index exceeds DICOM range".into())
                        })?;
                        let length = u32::try_from(range.len()).map_err(|_| {
                            Error::InvalidInput("SEG run length exceeds DICOM range".into())
                        })?;
                        let column_start = base_column.checked_add(start).ok_or_else(|| {
                            Error::InvalidInput("SEG column position overflows".into())
                        })?;
                        runs.push(BinaryMaskRun {
                            segment_number: frame.segment_number,
                            row: absolute_row,
                            column_start,
                            length,
                        });
                        Ok(())
                    },
                )?;
            }
        }
        runs.sort_by_key(|run| (run.segment_number, run.row, run.column_start));
        merge_binary_runs(runs)
    }

    pub fn fractional_runs(&self) -> Result<Vec<FractionalMaskRun>> {
        let frames = self.fractional_frames().ok_or_else(|| {
            Error::Unsupported("SEG does not contain fractional mask values".into())
        })?;
        let mut runs = Vec::new();
        let mut budget = RunBudget::new(MAX_VECTORIZED_RUNS);
        for frame in frames {
            let base_column = frame
                .tile_col
                .checked_mul(u32::from(frame.width))
                .ok_or_else(|| Error::InvalidInput("SEG column position overflows".into()))?;
            let base_row = frame
                .tile_row
                .checked_mul(u32::from(frame.height))
                .ok_or_else(|| Error::InvalidInput("SEG row position overflows".into()))?;
            for row in 0..usize::from(frame.height) {
                let row_start = row * usize::from(frame.width);
                let values = &frame.values[row_start..row_start + usize::from(frame.width)];
                let row_u32 = u32::try_from(row)
                    .map_err(|_| Error::InvalidInput("SEG row index exceeds DICOM range".into()))?;
                let absolute_row = base_row
                    .checked_add(row_u32)
                    .ok_or_else(|| Error::InvalidInput("SEG row position overflows".into()))?;
                scan_nonzero_ranges(
                    values,
                    |value| *value != 0,
                    &mut budget,
                    |range| {
                        let start_u32 = u32::try_from(range.start).map_err(|_| {
                            Error::InvalidInput("SEG column index exceeds DICOM range".into())
                        })?;
                        let column_start = base_column.checked_add(start_u32).ok_or_else(|| {
                            Error::InvalidInput("SEG column position overflows".into())
                        })?;
                        runs.push(FractionalMaskRun {
                            segment_number: frame.segment_number,
                            row: absolute_row,
                            column_start,
                            maximum_fractional_value: frame.maximum_fractional_value,
                            values: values[range].to_vec(),
                        });
                        Ok(())
                    },
                )?;
            }
        }
        runs.sort_by_key(|run| (run.segment_number, run.row, run.column_start));
        merge_fractional_runs(runs)
    }

    pub fn mask_digest(&self) -> Result<String> {
        let mut digest = Sha256::new();
        digest.update(b"dicom-viewer-seg-runs-v1\0");
        let (columns, rows) = self.source.total_pixel_matrix_dimensions();
        digest.update(columns.to_le_bytes());
        digest.update(rows.to_le_bytes());
        digest.update([match self.kind {
            SegmentationKind::Binary => 0,
            SegmentationKind::LabelMap => 1,
            SegmentationKind::Fractional => 2,
        }]);
        if self.kind == SegmentationKind::Fractional {
            for run in self.fractional_runs()? {
                digest.update(run.segment_number.to_le_bytes());
                digest.update(run.row.to_le_bytes());
                digest.update(run.column_start.to_le_bytes());
                digest.update(run.maximum_fractional_value.to_le_bytes());
                digest.update(
                    u32::try_from(run.values.len())
                        .map_err(|_| {
                            Error::InvalidInput(
                                "fractional SEG run length exceeds DICOM range".into(),
                            )
                        })?
                        .to_le_bytes(),
                );
                for value in run.values {
                    digest.update(value.to_le_bytes());
                }
            }
        } else {
            for run in self.binary_runs()? {
                digest.update(run.segment_number.to_le_bytes());
                digest.update(run.row.to_le_bytes());
                digest.update(run.column_start.to_le_bytes());
                digest.update(run.length.to_le_bytes());
            }
        }
        Ok(format!("{:x}", digest.finalize()))
    }

    #[must_use]
    pub fn fractional_frames(&self) -> Option<&[FractionalSegmentationFrame]> {
        self.imported_fractional_frames.as_deref()
    }

    pub fn write_seg(&self, path: impl AsRef<Path>) -> Result<()> {
        self.write_seg_with_loss_policy(path, false)
    }

    pub fn write_seg_with_loss_policy(
        &self,
        path: impl AsRef<Path>,
        allow_lossy: bool,
    ) -> Result<()> {
        if self.kind != SegmentationKind::Binary {
            return Err(Error::Unsupported(
                "only binary SEG export is supported".into(),
            ));
        }
        if !allow_lossy {
            let blocking = self
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.blocks_roundtrip())
                .map(InteroperabilityDiagnostic::code)
                .collect::<Vec<_>>();
            if !blocking.is_empty() {
                return Err(Error::Unsupported(format!(
                    "SEG rewrite would lose semantics ({}); pass the explicit lossy override to continue",
                    blocking.join(", ")
                )));
            }
        }
        let path = path.as_ref();
        ensure_sidecar_destination(path, self.source.source_path())?;
        let frames = self.rasterized_frames()?;
        if frames.is_empty() {
            return Err(Error::InvalidInput(
                "SEG contains no non-empty frames".into(),
            ));
        }
        let mut object = build_common_object(
            &self.source,
            uids::SEGMENTATION_STORAGE,
            &self.sop_instance_uid,
            &self.series_instance_uid,
            "SEG",
            &self.producer,
        )?;
        add_segmentation_attributes(&mut object, self, &frames)?;
        add_common_instance_reference(&mut object, &self.source);
        atomic_write_dicom(
            path,
            object,
            uids::SEGMENTATION_STORAGE,
            &self.sop_instance_uid,
        )
    }

    #[must_use]
    pub fn revised(&self) -> Self {
        let mut revised = self.clone();
        revised.sop_instance_uid = new_dicom_uid();
        revised
    }

    /// Projects binary or label-map pixels into exact pixel-edge ANN rectangles.
    ///
    /// Representable coded semantics are preserved. Identity that ANN cannot encode
    /// is reported through typed diagnostics and rejected unless `AllowLoss` is used.
    pub fn vectorized_annotations(
        &self,
        policy: SegToAnnConversionPolicy,
    ) -> Result<VectorizedAnnotations> {
        if self.kind == SegmentationKind::Fractional {
            return Err(Error::Unsupported(
                "fractional SEG remains a read-only raster overlay".into(),
            ));
        }
        let segment_indices = self.segment_indices_by_number()?;
        let runs = self.binary_runs()?;
        let mut polygon_counts = vec![0_usize; self.segments.len()];
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
        let mut diagnostics = self.diagnostics.clone();
        diagnostics.push(InteroperabilityDiagnostic::normalized(
            "SEG_RASTER_VECTORIZED",
            "$.PixelData",
            "projected the exact SEG raster into canonical pixel-edge ANN rectangles",
        ));
        let mut groups = Vec::with_capacity(self.segments.len());
        let mut group_uids = std::collections::BTreeSet::new();
        for (index, (segment, polygons)) in
            self.segments.iter().zip(polygons_by_segment).enumerate()
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

    #[must_use]
    pub const fn kind(&self) -> SegmentationKind {
        self.kind
    }

    #[must_use]
    pub const fn editable(&self) -> bool {
        !matches!(self.kind, SegmentationKind::Fractional)
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
    pub fn segments(&self) -> &[SegmentationSegment] {
        &self.segments
    }

    #[must_use]
    pub fn series_instance_uid(&self) -> &str {
        &self.series_instance_uid
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

    #[must_use]
    pub fn diagnostics(&self) -> &[InteroperabilityDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn segment_for_number(&self, number: u16) -> Option<&SegmentationSegment> {
        self.segment_index_for_number(number)
            .and_then(|index| self.segments.get(index))
    }

    fn segment_index_for_number(&self, number: u16) -> Option<usize> {
        self.segments
            .iter()
            .enumerate()
            .position(|(index, segment)| {
                segment
                    .source_segment_number
                    .unwrap_or_else(|| u16::try_from(index + 1).unwrap_or(u16::MAX))
                    == number
            })
    }

    fn segment_indices_by_number(&self) -> Result<HashMap<u16, usize>> {
        let mut indices = HashMap::with_capacity(self.segments.len());
        for (index, segment) in self.segments.iter().enumerate() {
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
}
