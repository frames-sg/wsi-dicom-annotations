use std::borrow::Cow;
use std::path::Path;

use dicom_dictionary_std::{tags, uids};
use sha2::{Digest, Sha256};

use super::conversion;
use super::raster::rasterize_segments;
use super::read::{
    read_binary_frames, read_fractional_frames, read_labelmap_frames, read_segments,
    validate_source_reference,
};
use super::runs;
use super::write::add_segmentation_attributes;
use super::{
    BinaryMaskRun, BinarySegmentationFrame, FractionalMaskRun, FractionalSegmentationFrame,
    SegToAnnConversionPolicy, SegmentationDocument, SegmentationKind, SegmentationSegment,
    VectorizedAnnotations, MAX_SEG_FILE_BYTES,
};
use crate::annotations::context::DicomAnnotationContext;
use crate::annotations::derived_object::{
    add_common_instance_reference, build_common_object, DerivedObjectProducer,
};
use crate::annotations::dicom_dataset::{new_dicom_uid, optional_string, required_string};
use crate::annotations::dicom_file::{
    atomic_write_dicom, enforce_file_limit, ensure_sidecar_destination,
};
use crate::annotations::model::InteroperabilityDiagnostic;
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

    pub(super) fn binary_frames(&self) -> Result<Cow<'_, [BinarySegmentationFrame]>> {
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
        runs::binary_runs(self)
    }

    pub fn fractional_runs(&self) -> Result<Vec<FractionalMaskRun>> {
        runs::fractional_runs(self)
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
        conversion::vectorized_annotations(self, policy)
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
}
