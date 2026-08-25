use super::*;
use crate::annotations::dicom_dataset::{put_text, sequence};
use crate::annotations::dicom_file::atomic_write_dicom;
use dicom_core::VR;
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

const ANN_UID: &str = "1.2.826.0.1.3680043.10.777.9501";
const SEG_UID: &str = "1.2.826.0.1.3680043.10.777.9502";
const SR_UID: &str = "1.2.826.0.1.3680043.10.777.9504";

#[test]
fn object_kind_uses_file_meta_and_rejects_non_dicom_or_unrelated_classes() {
    let directory = tempfile::tempdir().unwrap();
    let ann = directory.path().join("ann.dcm");
    let seg = directory.path().join("seg.dcm");
    let unrelated = directory.path().join("wsi.dcm");
    atomic_write_dicom(
        &ann,
        InMemDicomObject::new_empty(),
        uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE,
        ANN_UID,
    )
    .unwrap();
    atomic_write_dicom(
        &seg,
        InMemDicomObject::new_empty(),
        uids::SEGMENTATION_STORAGE,
        SEG_UID,
    )
    .unwrap();
    atomic_write_dicom(
        &unrelated,
        InMemDicomObject::new_empty(),
        uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE,
        "1.2.826.0.1.3680043.10.777.9503",
    )
    .unwrap();

    assert_eq!(
        annotation_object_kind(&ann).unwrap(),
        AnnotationObjectKind::Annotation
    );
    assert_eq!(
        annotation_object_kind(&seg).unwrap(),
        AnnotationObjectKind::Segmentation
    );
    assert!(annotation_object_kind(&unrelated).is_err());
    assert!(annotation_object_kind(directory.path().join("missing.dcm")).is_err());
    let short = directory.path().join("short.dcm");
    std::fs::write(&short, b"not dicom").unwrap();
    assert!(annotation_object_kind(&short).is_err());
}

#[test]
fn source_reference_paths_cover_direct_series_and_shared_derivation_forms() {
    let source_uid = "1.2.826.0.1.3680043.10.777.9510";
    let mut reference = InMemDicomObject::new_empty();
    put_text(
        &mut reference,
        tags::REFERENCED_SOP_INSTANCE_UID,
        VR::UI,
        source_uid,
    );

    let mut ann_data = InMemDicomObject::new_empty();
    ann_data.put(sequence(
        tags::REFERENCED_IMAGE_SEQUENCE,
        vec![reference.clone()],
    ));
    let ann = ann_data
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE)
                .media_storage_sop_instance_uid(ANN_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap();
    assert!(references_source(&ann, SidecarKind::Annotation, source_uid));
    assert!(!references_source(
        &ann,
        SidecarKind::Annotation,
        "2.25.404"
    ));

    let mut series = InMemDicomObject::new_empty();
    series.put(sequence(
        tags::REFERENCED_INSTANCE_SEQUENCE,
        vec![reference.clone()],
    ));
    let mut seg_data = InMemDicomObject::new_empty();
    seg_data.put(sequence(tags::REFERENCED_SERIES_SEQUENCE, vec![series]));
    let seg = seg_data
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::SEGMENTATION_STORAGE)
                .media_storage_sop_instance_uid(SEG_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap();
    assert!(references_source(
        &seg,
        SidecarKind::BinarySegmentation,
        source_uid
    ));

    let mut derivation = InMemDicomObject::new_empty();
    derivation.put(sequence(tags::SOURCE_IMAGE_SEQUENCE, vec![reference]));
    let mut shared = InMemDicomObject::new_empty();
    shared.put(sequence(tags::DERIVATION_IMAGE_SEQUENCE, vec![derivation]));
    let mut shared_data = InMemDicomObject::new_empty();
    shared_data.put(sequence(
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![shared],
    ));
    let shared_seg = shared_data
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::SEGMENTATION_STORAGE)
                .media_storage_sop_instance_uid(SEG_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap();
    assert!(shared_derivation_references_source(&shared_seg, source_uid));
    assert!(references_source(
        &shared_seg,
        SidecarKind::FractionalSegmentation,
        source_uid
    ));

    let mut sr_series = InMemDicomObject::new_empty();
    let mut sr_reference = InMemDicomObject::new_empty();
    put_text(
        &mut sr_reference,
        tags::REFERENCED_SOP_INSTANCE_UID,
        VR::UI,
        source_uid,
    );
    sr_series.put(sequence(tags::REFERENCED_SOP_SEQUENCE, vec![sr_reference]));
    let mut sr_study = InMemDicomObject::new_empty();
    sr_study.put(sequence(tags::REFERENCED_SERIES_SEQUENCE, vec![sr_series]));
    let mut sr_data = InMemDicomObject::new_empty();
    sr_data.put(sequence(
        tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE,
        vec![sr_study],
    ));
    let sr = sr_data
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::COMPREHENSIVE3_DSR_STORAGE)
                .media_storage_sop_instance_uid(SR_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap();
    assert!(references_source(
        &sr,
        SidecarKind::StructuredReport,
        source_uid
    ));
}

#[test]
fn discovery_returns_only_referencing_sidecars_with_stable_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    crate::test_support::write_source_wsi(&source_path, 16, 12, 4, 4);
    let context = DicomAnnotationContext::from_source(&source_path).unwrap();

    let mut reference = InMemDicomObject::new_empty();
    put_text(
        &mut reference,
        tags::REFERENCED_SOP_INSTANCE_UID,
        VR::UI,
        context.sop_instance_uid(),
    );
    let mut annotation = InMemDicomObject::new_empty();
    annotation.put(sequence(
        tags::REFERENCED_IMAGE_SEQUENCE,
        vec![reference.clone()],
    ));
    put_text(
        &mut annotation,
        tags::SERIES_INSTANCE_UID,
        VR::UI,
        "1.2.826.0.1.3680043.10.777.9520",
    );
    let ann_path = directory.path().join("a-ann.dcm");
    atomic_write_dicom(
        &ann_path,
        annotation,
        uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE,
        ANN_UID,
    )
    .unwrap();
    let mut series = InMemDicomObject::new_empty();
    series.put(sequence(tags::REFERENCED_SOP_SEQUENCE, vec![reference]));
    let mut study = InMemDicomObject::new_empty();
    study.put(sequence(tags::REFERENCED_SERIES_SEQUENCE, vec![series]));
    let mut report = InMemDicomObject::new_empty();
    report.put(sequence(
        tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE,
        vec![study],
    ));
    let sr_path = directory.path().join("b-sr.dcm");
    atomic_write_dicom(&sr_path, report, uids::COMPREHENSIVE3_DSR_STORAGE, SR_UID).unwrap();
    std::fs::write(directory.path().join("ignored.txt"), b"plain text").unwrap();

    let sidecars = discover_sidecars(&context).unwrap();
    assert_eq!(sidecars.len(), 2);
    assert_eq!(sidecars[0].path(), ann_path);
    assert_eq!(sidecars[0].kind(), SidecarKind::Annotation);
    assert_eq!(sidecars[0].sop_instance_uid(), ANN_UID);
    assert_eq!(
        sidecars[0].series_instance_uid(),
        Some("1.2.826.0.1.3680043.10.777.9520")
    );
    assert_eq!(sidecars[1].path(), sr_path);
    assert_eq!(sidecars[1].kind(), SidecarKind::StructuredReport);
    assert_eq!(sidecars[1].sop_instance_uid(), SR_UID);
}
