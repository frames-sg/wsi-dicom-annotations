use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use dicom_core::Tag;
use dicom_dictionary_std::{tags, uids};
use dicom_object::file::ReadPreamble;
use dicom_object::meta::FileMetaTable;
use dicom_object::OpenFileOptions;

use crate::{Error, Result};

use super::context::DicomAnnotationContext;
use super::dicom_dataset::optional_string;

const MAX_SIDECAR_CANDIDATES: usize = 10_000;
const MAX_SIDECAR_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarKind {
    Annotation,
    BinarySegmentation,
    FractionalSegmentation,
    LabelMapSegmentation,
    StructuredReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationObjectKind {
    Annotation,
    Segmentation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarMetadata {
    path: PathBuf,
    kind: SidecarKind,
    sop_instance_uid: String,
    series_instance_uid: Option<String>,
}

impl SidecarMetadata {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn kind(&self) -> SidecarKind {
        self.kind
    }

    #[must_use]
    pub fn sop_instance_uid(&self) -> &str {
        &self.sop_instance_uid
    }

    #[must_use]
    pub fn series_instance_uid(&self) -> Option<&str> {
        self.series_instance_uid.as_deref()
    }
}

pub fn annotation_object_kind(path: impl AsRef<Path>) -> Result<AnnotationObjectKind> {
    let path = path.as_ref();
    let length = std::fs::metadata(path)
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if !(132..=MAX_SIDECAR_FILE_BYTES).contains(&length) {
        return Err(Error::InvalidInput(format!(
            "annotation object is {length} bytes; supported range is 132..={MAX_SIDECAR_FILE_BYTES}"
        )));
    }
    let mut file = std::fs::File::open(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut preamble = [0_u8; 132];
    file.read_exact(&mut preamble).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if &preamble[128..] != b"DICM" {
        return Err(Error::InvalidInput(
            "annotation object has no DICOM preamble".into(),
        ));
    }
    file.seek(SeekFrom::Start(128))
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let meta = FileMetaTable::from_reader(&mut file).map_err(|error| {
        Error::InvalidInput(format!("invalid annotation object file meta: {error}"))
    })?;
    match meta.media_storage_sop_class_uid() {
        uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE => Ok(AnnotationObjectKind::Annotation),
        uids::SEGMENTATION_STORAGE | uids::LABEL_MAP_SEGMENTATION_STORAGE => {
            Ok(AnnotationObjectKind::Segmentation)
        }
        sop_class => Err(Error::Unsupported(format!(
            "SOP Class {sop_class} is not ANN or SEG"
        ))),
    }
}

/// Discover local DICOM annotation sidecars without reading ANN coordinate arrays
/// or SEG functional groups and Pixel Data.
pub fn discover_sidecars(source: &DicomAnnotationContext) -> Result<Vec<SidecarMetadata>> {
    let directory = source
        .source_path()
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|source_error| Error::Io {
        path: directory.to_path_buf(),
        source: source_error,
    })? {
        let entry = entry.map_err(|source_error| Error::Io {
            path: directory.to_path_buf(),
            source: source_error,
        })?;
        let path = entry.path();
        if !path.is_file() || path == source.source_path() {
            continue;
        }
        if candidates.len() >= MAX_SIDECAR_CANDIDATES {
            return Err(Error::InvalidInput(format!(
                "sidecar directory contains more than {MAX_SIDECAR_CANDIDATES} files"
            )));
        }
        candidates.push(path);
    }
    candidates.sort();
    let mut sidecars = candidates
        .into_iter()
        .filter_map(|path| inspect_sidecar(&path, source))
        .collect::<Result<Vec<_>>>()?;
    sidecars.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(sidecars)
}

fn inspect_sidecar(
    path: &Path,
    source: &DicomAnnotationContext,
) -> Option<Result<SidecarMetadata>> {
    let length = std::fs::metadata(path).ok()?.len();
    if !(132..=MAX_SIDECAR_FILE_BYTES).contains(&length) {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let mut preamble = [0_u8; 132];
    if file.read_exact(&mut preamble).is_err() || &preamble[128..] != b"DICM" {
        return None;
    }
    if file.seek(SeekFrom::Start(128)).is_err() {
        return None;
    }
    let meta = match FileMetaTable::from_reader(&mut file) {
        Ok(meta) => meta,
        Err(_) => return None,
    };
    let (kind, stop_tag) =
        if meta.media_storage_sop_class_uid() == uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE {
            (SidecarKind::Annotation, tags::ANNOTATION_GROUP_SEQUENCE)
        } else if meta.media_storage_sop_class_uid() == uids::SEGMENTATION_STORAGE {
            (
                SidecarKind::BinarySegmentation,
                tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
            )
        } else if meta.media_storage_sop_class_uid() == uids::LABEL_MAP_SEGMENTATION_STORAGE {
            (
                SidecarKind::LabelMapSegmentation,
                tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
            )
        } else if meta.media_storage_sop_class_uid() == uids::COMPREHENSIVE3_DSR_STORAGE {
            (SidecarKind::StructuredReport, tags::CONTENT_SEQUENCE)
        } else {
            return None;
        };
    let object = match OpenFileOptions::new()
        .read_until(stop_tag)
        .read_preamble(ReadPreamble::Always)
        .open_file(path)
    {
        Ok(object) => object,
        Err(_) => return None,
    };
    let kind = if kind == SidecarKind::BinarySegmentation
        && optional_string(&object, tags::SEGMENTATION_TYPE).as_deref() == Some("FRACTIONAL")
    {
        SidecarKind::FractionalSegmentation
    } else {
        kind
    };
    if !references_source(&object, kind, source.sop_instance_uid()) {
        return None;
    }
    Some(Ok(SidecarMetadata {
        path: path.to_path_buf(),
        kind,
        sop_instance_uid: object.meta().media_storage_sop_instance_uid().to_string(),
        series_instance_uid: optional_string(&object, tags::SERIES_INSTANCE_UID),
    }))
}

fn references_source(
    object: &dicom_object::DefaultDicomObject,
    kind: SidecarKind,
    source_uid: &str,
) -> bool {
    if kind == SidecarKind::Annotation {
        return object
            .get(tags::REFERENCED_IMAGE_SEQUENCE)
            .and_then(|element| element.items())
            .into_iter()
            .flatten()
            .any(|item| {
                optional_string(item, tags::REFERENCED_SOP_INSTANCE_UID).as_deref()
                    == Some(source_uid)
            })
            || sequence_contains_reference(object, tags::REFERENCED_SERIES_SEQUENCE, source_uid);
    }
    if kind == SidecarKind::StructuredReport {
        return evidence_contains_reference(
            object,
            tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE,
            source_uid,
        ) || evidence_contains_reference(
            object,
            tags::PERTINENT_OTHER_EVIDENCE_SEQUENCE,
            source_uid,
        );
    }
    sequence_contains_reference(object, tags::REFERENCED_SERIES_SEQUENCE, source_uid)
        || shared_derivation_references_source(object, source_uid)
}

fn evidence_contains_reference(
    object: &dicom_object::DefaultDicomObject,
    evidence_tag: Tag,
    source_uid: &str,
) -> bool {
    object
        .get(evidence_tag)
        .and_then(|element| element.items())
        .into_iter()
        .flatten()
        .flat_map(|study| {
            study
                .get(tags::REFERENCED_SERIES_SEQUENCE)
                .and_then(|element| element.items())
                .unwrap_or_default()
        })
        .flat_map(|series| {
            series
                .get(tags::REFERENCED_SOP_SEQUENCE)
                .and_then(|element| element.items())
                .unwrap_or_default()
        })
        .any(|item| {
            optional_string(item, tags::REFERENCED_SOP_INSTANCE_UID).as_deref() == Some(source_uid)
        })
}

fn shared_derivation_references_source(
    object: &dicom_object::DefaultDicomObject,
    source_uid: &str,
) -> bool {
    object
        .get(tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE)
        .and_then(|element| element.items())
        .into_iter()
        .flatten()
        .flat_map(|shared| {
            shared
                .get(tags::DERIVATION_IMAGE_SEQUENCE)
                .and_then(|element| element.items())
                .unwrap_or_default()
        })
        .flat_map(|derivation| {
            derivation
                .get(tags::SOURCE_IMAGE_SEQUENCE)
                .and_then(|element| element.items())
                .unwrap_or_default()
        })
        .any(|reference| {
            optional_string(reference, tags::REFERENCED_SOP_INSTANCE_UID).as_deref()
                == Some(source_uid)
        })
}

fn sequence_contains_reference(
    object: &dicom_object::DefaultDicomObject,
    tag: Tag,
    source_uid: &str,
) -> bool {
    object
        .get(tag)
        .and_then(|element| element.items())
        .into_iter()
        .flatten()
        .flat_map(|series| {
            series
                .get(tags::REFERENCED_INSTANCE_SEQUENCE)
                .and_then(|element| element.items())
                .unwrap_or_default()
        })
        .any(|item| {
            optional_string(item, tags::REFERENCED_SOP_INSTANCE_UID).as_deref() == Some(source_uid)
        })
}

#[cfg(test)]
#[path = "sidecar_tests.rs"]
mod tests;
