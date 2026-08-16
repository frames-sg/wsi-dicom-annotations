use std::path::Path;

use super::PathologyAnnotationSet;
use crate::{
    AnnotationDocument, DicomAnnotationContext, Error, Result, SegmentationDocument,
    StructuredReportDocument,
};

#[derive(Debug, thiserror::Error)]
pub enum PathologyDocumentWriteError {
    #[error("selected pathology DICOM target document is missing")]
    MissingTarget,
    #[error("could not write staged DICOM {target}: {source}")]
    Write {
        target: &'static str,
        #[source]
        source: Error,
    },
    #[error("could not reread staged DICOM {target}: {source}")]
    Verification {
        target: &'static str,
        #[source]
        source: Error,
    },
    #[error("reread {0} semantics differ from the staged document")]
    Mismatch(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathologyDicomTarget {
    Ann,
    Seg,
    Sr,
}

impl PathologyDicomTarget {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ann => "ann",
            Self::Seg => "seg",
            Self::Sr => "sr",
        }
    }

    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Ann => "ann.dcm",
            Self::Seg => "seg.dcm",
            Self::Sr => "sr.dcm",
        }
    }
}

/// A target-consistent set of pathology-derived DICOM documents.
///
/// Construction enforces companion-object semantics once for both the desktop
/// viewer and the headless converter. In particular, SR may preserve semantics
/// omitted from ANN and may reference the exact SEG built in this set.
pub struct PathologyDicomDocuments {
    ann: Option<AnnotationDocument>,
    seg: Option<SegmentationDocument>,
    sr: Option<StructuredReportDocument>,
}

impl PathologyDicomDocuments {
    pub fn build(
        annotations: &PathologyAnnotationSet,
        targets: &[PathologyDicomTarget],
    ) -> Result<Self> {
        validate_targets(targets)?;
        let wants_ann = targets.contains(&PathologyDicomTarget::Ann);
        let wants_seg = targets.contains(&PathologyDicomTarget::Seg);
        let wants_sr = targets.contains(&PathologyDicomTarget::Sr);
        let ann = wants_ann
            .then(|| {
                if wants_sr {
                    annotations.to_ann_with_companion_sr()
                } else {
                    annotations.to_ann()
                }
            })
            .transpose()?;
        let seg = wants_seg
            .then(|| annotations.to_seg(wants_sr))
            .transpose()?;
        let sr = wants_sr
            .then(|| annotations.to_sr(seg.as_ref()))
            .transpose()?;
        Ok(Self { ann, seg, sr })
    }

    #[must_use]
    pub const fn ann(&self) -> Option<&AnnotationDocument> {
        self.ann.as_ref()
    }

    #[must_use]
    pub const fn seg(&self) -> Option<&SegmentationDocument> {
        self.seg.as_ref()
    }

    #[must_use]
    pub const fn sr(&self) -> Option<&StructuredReportDocument> {
        self.sr.as_ref()
    }

    /// Write one selected document, reread it, and compare the semantics used
    /// by the converter. Callers should write into a private staging area.
    pub fn write_and_verify(
        &self,
        target: PathologyDicomTarget,
        path: &Path,
        source: &DicomAnnotationContext,
    ) -> std::result::Result<(), PathologyDocumentWriteError> {
        match target {
            PathologyDicomTarget::Ann => {
                let expected = self.ann.as_ref().ok_or_else(missing_document)?;
                expected
                    .write_ann(path)
                    .map_err(|source| write_error("ANN", source))?;
                let actual = AnnotationDocument::read_ann(path, source)
                    .map_err(|source| verification_error("ANN", source))?;
                if actual.groups().len() != expected.groups().len()
                    || actual.sop_instance_uid() != expected.sop_instance_uid()
                {
                    return Err(verification_mismatch("ANN"));
                }
            }
            PathologyDicomTarget::Seg => {
                let expected = self.seg.as_ref().ok_or_else(missing_document)?;
                expected
                    .write_seg(path)
                    .map_err(|source| write_error("SEG", source))?;
                let actual = SegmentationDocument::read_seg(path, source)
                    .map_err(|source| verification_error("SEG", source))?;
                if actual.segments().len() != expected.segments().len()
                    || actual.sop_instance_uid() != expected.sop_instance_uid()
                    || actual
                        .binary_runs()
                        .map_err(|source| verification_error("SEG", source))?
                        != expected
                            .binary_runs()
                            .map_err(|source| verification_error("SEG", source))?
                {
                    return Err(verification_mismatch("SEG"));
                }
            }
            PathologyDicomTarget::Sr => {
                let expected = self.sr.as_ref().ok_or_else(missing_document)?;
                expected
                    .write_sr(path)
                    .map_err(|source| write_error("SR", source))?;
                let actual = StructuredReportDocument::read_sr(path, source, self.seg.as_ref())
                    .map_err(|source| verification_error("SR", source))?;
                if !sr_semantics_match(&actual, expected) {
                    return Err(verification_mismatch("SR"));
                }
            }
        }
        Ok(())
    }
}

fn validate_targets(targets: &[PathologyDicomTarget]) -> Result<()> {
    if targets.is_empty() {
        return Err(Error::InvalidInput(
            "pathology conversion requires at least one DICOM target".into(),
        ));
    }
    if targets
        .iter()
        .enumerate()
        .any(|(index, target)| targets[..index].contains(target))
    {
        return Err(Error::InvalidInput(
            "pathology conversion targets must be unique".into(),
        ));
    }
    Ok(())
}

fn sr_semantics_match(
    actual: &StructuredReportDocument,
    expected: &StructuredReportDocument,
) -> bool {
    actual.sop_instance_uid() == expected.sop_instance_uid()
        && actual.series_instance_uid() == expected.series_instance_uid()
        && actual.completion_flag() == expected.completion_flag()
        && actual.verification_flag() == expected.verification_flag()
        && actual.preliminary_flag() == expected.preliminary_flag()
        && actual.groups().len() == expected.groups().len()
        && actual
            .groups()
            .iter()
            .zip(expected.groups())
            .all(|(actual, expected)| {
                actual.tracking_id() == expected.tracking_id()
                    && actual.tracking_uid() == expected.tracking_uid()
                    && actual.reference_kind() == expected.reference_kind()
                    && actual.referenced_segmentation_uid()
                        == expected.referenced_segmentation_uid()
                    && actual.referenced_segment_number() == expected.referenced_segment_number()
                    && actual.measurements().len() == expected.measurements().len()
                    && actual
                        .measurements()
                        .iter()
                        .zip(expected.measurements())
                        .all(|(actual, expected)| {
                            actual.concept() == expected.concept()
                                && actual.unit() == expected.unit()
                                && actual.value() == expected.value()
                        })
                    && actual.qualitative_evaluations() == expected.qualitative_evaluations()
            })
}

fn missing_document() -> PathologyDocumentWriteError {
    PathologyDocumentWriteError::MissingTarget
}

fn write_error(target: &'static str, source: Error) -> PathologyDocumentWriteError {
    PathologyDocumentWriteError::Write { target, source }
}

fn verification_error(target: &'static str, source: Error) -> PathologyDocumentWriteError {
    PathologyDocumentWriteError::Verification { target, source }
}

fn verification_mismatch(target: &'static str) -> PathologyDocumentWriteError {
    PathologyDocumentWriteError::Mismatch(target)
}
