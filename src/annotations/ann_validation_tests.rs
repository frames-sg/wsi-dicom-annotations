use std::path::Path;

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
