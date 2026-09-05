use super::*;
use dicom_core::Tag;

#[test]
fn volume_ann_preserves_exact_source_identity_and_image_reference_on_round_trip() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let first_path = directory.path().join("annotations.dcm");
    let round_trip_path = directory.path().join("annotations-round-trip.dcm");
    write_source_with_container(&source_path, "SLIDE-IDENTITY-EXACT");
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();

    point_document(source.clone())
        .write_ann(&first_path)
        .unwrap();
    assert_volume_identity_and_reference(
        &first_path,
        &source,
        Some("1.2.826.0.1.3680043.10.777.104"),
        Some("SLIDE-IDENTITY-EXACT"),
    );

    AnnotationDocument::read_ann(&first_path, &source)
        .unwrap()
        .revised()
        .write_ann(&round_trip_path)
        .unwrap();
    assert_volume_identity_and_reference(
        &round_trip_path,
        &source,
        Some("1.2.826.0.1.3680043.10.777.104"),
        Some("SLIDE-IDENTITY-EXACT"),
    );
}

#[test]
fn volume_ann_does_not_invent_absent_source_identity() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let output_path = directory.path().join("annotations.dcm");
    write_source_wsi(&source_path, 16, 12, 4, 4);
    let mut source_object = dicom_object::open_file(&source_path).unwrap();
    source_object.take(tags::FRAME_OF_REFERENCE_UID).unwrap();
    source_object.write_to_file(&source_path).unwrap();
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();

    point_document(source.clone())
        .write_ann(&output_path)
        .unwrap();
    assert_volume_identity_and_reference(&output_path, &source, None, None);
}

#[test]
fn revised_volume_ann_preserves_absent_input_frame_of_reference_uid() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let input_path = directory.path().join("input.dcm");
    let output_path = directory.path().join("output.dcm");
    write_source_with_container(&source_path, "SOURCE-CONTAINER");
    write_native_ann(
        &input_path,
        "2D",
        Some("VOLUME"),
        None,
        &[(AnnotationGraphicType::Point, vec![1.0, 1.0], vec![], None)],
    );
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();

    AnnotationDocument::read_ann(&input_path, &source)
        .unwrap()
        .revised()
        .write_ann_with_loss_policy(&output_path, true)
        .unwrap();

    let output = dicom_object::open_file(&output_path).unwrap();
    assert!(output.get(tags::FRAME_OF_REFERENCE_UID).is_none());
    assert!(output.get(tags::CONTAINER_IDENTIFIER).is_none());
}

#[test]
fn volume_ann_rejects_malformed_or_changed_source_identity_before_publication() {
    for (name, tag, vr, value) in [
        (
            "malformed-frame",
            tags::FRAME_OF_REFERENCE_UID,
            VR::UI,
            "not-a-dicom-uid",
        ),
        (
            "malformed-container",
            tags::CONTAINER_IDENTIFIER,
            VR::LO,
            "SLIDE\\SECOND-VALUE",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.dcm");
        let output_path = directory.path().join(format!("{name}.dcm"));
        write_source_wsi(&source_path, 16, 12, 4, 4);
        put_source_value(&source_path, tag, vr, value);
        let source = DicomAnnotationContext::from_source(&source_path).unwrap();

        assert!(point_document(source).write_ann(&output_path).is_err());
        assert!(!output_path.exists(), "{name}");
    }

    for (name, tag, vr, changed_value) in [
        (
            "changed-frame",
            tags::FRAME_OF_REFERENCE_UID,
            VR::UI,
            "2.25.9999",
        ),
        (
            "changed-container",
            tags::CONTAINER_IDENTIFIER,
            VR::LO,
            "DIFFERENT-SLIDE",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.dcm");
        let output_path = directory.path().join(format!("{name}.dcm"));
        write_source_with_container(&source_path, "ORIGINAL-SLIDE");
        let source = DicomAnnotationContext::from_source(&source_path).unwrap();
        let document = point_document(source);
        put_source_value(&source_path, tag, vr, changed_value);

        assert!(document.write_ann(&output_path).is_err());
        assert!(!output_path.exists(), "{name}");
    }
}

#[test]
fn volume_ann_reader_rejects_malformed_or_conflicting_source_identity() {
    for (name, tag, vr, value) in [
        ("frame", tags::FRAME_OF_REFERENCE_UID, VR::UI, "2.25.9999"),
        (
            "container",
            tags::CONTAINER_IDENTIFIER,
            VR::LO,
            "DIFFERENT-SLIDE",
        ),
        (
            "malformed-frame",
            tags::FRAME_OF_REFERENCE_UID,
            VR::UI,
            "not-a-dicom-uid",
        ),
        (
            "malformed-container",
            tags::CONTAINER_IDENTIFIER,
            VR::LO,
            "SLIDE\\SECOND-VALUE",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.dcm");
        let annotation_path = directory.path().join(format!("{name}.dcm"));
        write_source_with_container(&source_path, "ORIGINAL-SLIDE");
        write_native_ann(
            &annotation_path,
            "2D",
            Some("VOLUME"),
            None,
            &[(AnnotationGraphicType::Point, vec![1.0, 1.0], vec![], None)],
        );
        let mut annotation_object = dicom_object::open_file(&annotation_path).unwrap();
        annotation_object.put(DataElement::new(tag, vr, value));
        annotation_object.write_to_file(&annotation_path).unwrap();
        let source = DicomAnnotationContext::from_source(&source_path).unwrap();

        assert!(AnnotationDocument::read_ann(&annotation_path, &source).is_err());
    }
}

fn point_document(source: DicomAnnotationContext) -> AnnotationDocument {
    AnnotationDocument::new(
        source,
        vec![AnnotationGroup::points(
            "Cells",
            code("MORPH", "Morphology"),
            code("CELL", "Cell"),
            [0, 0, 0],
            vec![Point2::new(1.0, 1.0)],
        )
        .unwrap()],
    )
    .unwrap()
}

fn write_source_with_container(path: &std::path::Path, container_identifier: &str) {
    write_source_wsi(path, 16, 12, 4, 4);
    put_source_value(
        path,
        tags::CONTAINER_IDENTIFIER,
        VR::LO,
        container_identifier,
    );
}

fn put_source_value(path: &std::path::Path, tag: Tag, vr: VR, value: &str) {
    let mut source_object = dicom_object::open_file(path).unwrap();
    source_object.put(DataElement::new(tag, vr, value));
    source_object.write_to_file(path).unwrap();
}

fn assert_volume_identity_and_reference(
    path: &std::path::Path,
    source: &DicomAnnotationContext,
    expected_frame_of_reference_uid: Option<&str>,
    expected_container_identifier: Option<&str>,
) {
    let object = dicom_object::open_file(path).unwrap();
    assert_eq!(
        object
            .element(tags::PIXEL_ORIGIN_INTERPRETATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "VOLUME"
    );
    let references = object
        .element(tags::REFERENCED_IMAGE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(
        references[0]
            .element(tags::REFERENCED_SOP_CLASS_UID)
            .unwrap()
            .to_str()
            .unwrap(),
        source.sop_class_uid()
    );
    assert_eq!(
        references[0]
            .element(tags::REFERENCED_SOP_INSTANCE_UID)
            .unwrap()
            .to_str()
            .unwrap(),
        source.sop_instance_uid()
    );
    assert_eq!(
        object
            .get(tags::FRAME_OF_REFERENCE_UID)
            .map(|element| element.to_str().unwrap().into_owned()),
        expected_frame_of_reference_uid.map(str::to_string)
    );
    assert_eq!(
        object
            .get(tags::CONTAINER_IDENTIFIER)
            .map(|element| element.to_str().unwrap().into_owned()),
        expected_container_identifier.map(str::to_string)
    );
}
