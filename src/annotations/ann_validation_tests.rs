use std::path::Path;

use dicom_core::{DataElement, VR};
use dicom_dictionary_std::tags;

use crate::annotations::dicom_dataset::sequence;
use crate::test_support::write_source_wsi;
use crate::{AnnotationGroup, DicomAnnotationContext, DicomCode, Point2};

use super::AnnotationDocument;

fn code(value: &str, meaning: &str) -> DicomCode {
    DicomCode::new(value, "99FRAMES", meaning).unwrap()
}

fn point_group(x: f64, y: f64) -> AnnotationGroup {
    AnnotationGroup::points(
        "Cells",
        code("MORPH", "Morphology"),
        code("CELL", "Cell"),
        [1, 2, 3],
        vec![Point2::new(x, y)],
    )
    .unwrap()
}

fn document(source_path: &Path) -> AnnotationDocument {
    write_source_wsi(source_path, 16, 12, 4, 4);
    let source = DicomAnnotationContext::from_source(source_path).unwrap();
    AnnotationDocument::new(source, vec![point_group(1.0, 1.0)]).unwrap()
}

#[test]
fn checked_document_edits_reject_bounds_and_duplicate_uids_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let mut annotations = document(&source);
    let original = annotations.clone();

    assert!(annotations
        .replace_group(0, point_group(17.0, 1.0))
        .is_err());
    assert_eq!(annotations, original);

    let duplicate = annotations.groups()[0].clone();
    assert!(annotations
        .replace_groups(vec![duplicate.clone(), duplicate])
        .is_err());
    assert_eq!(annotations, original);
}

#[test]
fn invalid_internal_document_is_rejected_before_destination_publication() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let destination = directory.path().join("invalid.dcm");
    let mut annotations = document(&source);
    annotations.groups.clear();

    assert!(annotations.write_ann(&destination).is_err());
    assert!(!destination.exists());
}

#[test]
fn valid_checked_document_edit_writes_and_round_trips() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let destination = directory.path().join("valid.dcm");
    let mut annotations = document(&source);
    annotations.replace_group(0, point_group(8.0, 6.0)).unwrap();

    annotations.write_ann(&destination).unwrap();
    let imported = AnnotationDocument::read_ann(&destination, annotations.source()).unwrap();
    assert_eq!(
        imported.groups()[0].point_annotations(),
        Some([Point2::new(8.0, 6.0)].as_slice())
    );
}

#[test]
fn new_2d_documents_reject_z_constraints() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    let source = DicomAnnotationContext::from_source(&source).unwrap();
    let group = point_group(1.0, 1.0)
        .with_common_z_coordinates(vec![0.0])
        .unwrap();

    let error = AnnotationDocument::new(source, vec![group]).unwrap_err();
    assert!(error.to_string().contains("2D ANN"));
    assert!(error.to_string().contains("Z-plane"));
}

#[test]
fn new_documents_reject_unknown_optical_paths() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    let source = DicomAnnotationContext::from_source(&source).unwrap();
    let group = point_group(1.0, 1.0)
        .with_referenced_optical_paths(vec!["UNKNOWN".into()])
        .unwrap();

    let error = AnnotationDocument::new(source, vec![group]).unwrap_err();
    assert!(error.to_string().contains("UNKNOWN"));
    assert!(error.to_string().contains("source WSI"));
}

#[test]
fn foreign_ann_with_unknown_optical_path_remains_readable_but_not_rewritable() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let ann_path = directory.path().join("foreign.dcm");
    write_source_wsi(&source_path, 16, 12, 4, 4);
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let group = point_group(1.0, 1.0)
        .with_referenced_optical_paths(vec!["OPTICAL-1".into()])
        .unwrap();
    AnnotationDocument::new(source.clone(), vec![group])
        .unwrap()
        .write_ann(&ann_path)
        .unwrap();
    let mut object = dicom_object::open_file(&ann_path).unwrap();
    let mut groups = object
        .get(tags::ANNOTATION_GROUP_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()
        .to_vec();
    groups[0].put(DataElement::new(
        tags::REFERENCED_OPTICAL_PATH_IDENTIFIER,
        VR::SH,
        "UNKNOWN",
    ));
    object.put(sequence(tags::ANNOTATION_GROUP_SEQUENCE, groups));
    object.write_to_file(&ann_path).unwrap();

    let imported = AnnotationDocument::read_ann(&ann_path, &source).unwrap();
    assert_eq!(
        imported.groups()[0].referenced_optical_paths(),
        &["UNKNOWN"]
    );
    assert!(imported
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code() == "UNKNOWN_OPTICAL_PATH_IDENTIFIER"));
    assert!(imported
        .write_ann(directory.path().join("rewrite.dcm"))
        .is_err());
}
