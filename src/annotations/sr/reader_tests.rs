use super::*;

use crate::annotations::dicom_dataset::sequence;
use dicom_core::value::PrimitiveValue;
use dicom_core::{dicom_value, DataElement, VR};

fn code(value: &str, meaning: &str) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    item.put(DataElement::new(tags::CODE_VALUE, VR::SH, value));
    item.put(DataElement::new(
        tags::CODING_SCHEME_DESIGNATOR,
        VR::SH,
        "99TEST",
    ));
    item.put(DataElement::new(tags::CODE_MEANING, VR::LO, meaning));
    item
}

fn content_item(concept: &str, value_type: &str) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    item.put(DataElement::new(tags::VALUE_TYPE, VR::CS, value_type));
    item.put(sequence(
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        vec![code(concept, concept)],
    ));
    item
}

fn source_context() -> (tempfile::TempDir, DicomAnnotationContext) {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let source = directory.path().join("source.dcm");
    crate::annotation_tests::write_source_wsi(&source, 16, 12, 4, 4);
    let context = DicomAnnotationContext::from_source(&source)
        .expect("synthetic VL WSI context should parse");
    (directory, context)
}

#[test]
fn content_helpers_enforce_single_codes_templates_and_sequences() {
    let mut item = content_item("121012", "UIDREF");
    item.put(DataElement::new(tags::UID, VR::UI, "2.25.1"));
    assert!(has_concept(&item, "121012"));
    assert!(!has_concept(&item, "other"));
    assert_eq!(concept_value(&item).unwrap(), "121012");
    assert!(find_item(std::slice::from_ref(&item), "121012", None)
        .is_some_and(|found| std::ptr::eq(found, &item)));
    assert!(find_item(std::slice::from_ref(&item), "121012", Some("TEXT")).is_none());
    assert_eq!(
        find_text_value(std::slice::from_ref(&item), "121012", tags::UID).unwrap(),
        "2.25.1"
    );
    assert!(find_text_value(std::slice::from_ref(&item), "missing", tags::UID).is_err());

    let mut root = InMemDicomObject::new_empty();
    assert!(content_items(&root).is_err());
    assert!(optional_content_items(&root).is_empty());
    assert!(sequence_items(&root, tags::CONTENT_SEQUENCE).is_err());
    assert_eq!(template_identifier(&root).unwrap(), None);
    root.put(sequence(tags::CONTENT_SEQUENCE, vec![item.clone()]));
    assert_eq!(content_items(&root).unwrap().len(), 1);
    assert!(has_concept(&content_items(&root).unwrap()[0], "121012"));
    assert_eq!(optional_content_items(&root).len(), 1);

    let mut template = InMemDicomObject::new_empty();
    template.put(DataElement::new(tags::TEMPLATE_IDENTIFIER, VR::CS, "1500"));
    root.put(sequence(
        tags::CONTENT_TEMPLATE_SEQUENCE,
        vec![template.clone()],
    ));
    assert_eq!(template_identifier(&root).unwrap().as_deref(), Some("1500"));
    root.put(sequence(
        tags::CONTENT_TEMPLATE_SEQUENCE,
        vec![template.clone(), template],
    ));
    assert!(template_identifier(&root).is_err());

    let mut missing_value = content_item("ignored", "CODE");
    missing_value.put(sequence(
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        vec![InMemDicomObject::new_empty()],
    ));
    assert!(concept_value(&missing_value).is_err());
}

#[test]
fn document_flags_and_evidence_require_the_exact_research_source() {
    let (_directory, source) = source_context();
    let mut object = InMemDicomObject::new_empty();
    for (tag, value) in [
        (tags::COMPLETION_FLAG, "COMPLETE"),
        (tags::VERIFICATION_FLAG, "UNVERIFIED"),
        (tags::PRELIMINARY_FLAG, "PRELIMINARY"),
    ] {
        object.put(DataElement::new(tag, VR::CS, value));
    }
    assert!(validate_document_flags(&object).is_ok());
    object.put(DataElement::new(tags::COMPLETION_FLAG, VR::CS, "PARTIAL"));
    assert!(validate_document_flags(&object).is_err());
    object.put(DataElement::new(tags::COMPLETION_FLAG, VR::CS, "COMPLETE"));

    let mut reference = InMemDicomObject::new_empty();
    reference.put(DataElement::new(
        tags::REFERENCED_SOP_CLASS_UID,
        VR::UI,
        source.sop_class_uid(),
    ));
    reference.put(DataElement::new(
        tags::REFERENCED_SOP_INSTANCE_UID,
        VR::UI,
        source.sop_instance_uid(),
    ));
    let mut series = InMemDicomObject::new_empty();
    series.put(DataElement::new(
        tags::SERIES_INSTANCE_UID,
        VR::UI,
        source.series_instance_uid(),
    ));
    series.put(sequence(tags::REFERENCED_SOP_SEQUENCE, vec![reference]));
    let mut study = InMemDicomObject::new_empty();
    study.put(DataElement::new(
        tags::STUDY_INSTANCE_UID,
        VR::UI,
        source.study_instance_uid(),
    ));
    study.put(sequence(tags::REFERENCED_SERIES_SEQUENCE, vec![series]));
    object.put(sequence(
        tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE,
        vec![study],
    ));
    assert!(validate_source_evidence(&object, &source).is_ok());

    object.put(DataElement::new(
        tags::STUDY_INSTANCE_UID,
        VR::UI,
        "2.25.999",
    ));
    assert!(validate_source_evidence(&object, &source).is_ok());
    object.put(sequence(
        tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE,
        Vec::new(),
    ));
    assert!(validate_source_evidence(&object, &source).is_err());
}

#[test]
fn coordinate_and_measurement_readers_validate_triplets_units_and_frame_identity() {
    let (_directory, source) = source_context();
    for (graphic, expected) in [
        ("POINT", CoordinateGraphic::Point),
        ("MULTIPOINT", CoordinateGraphic::Multipoint),
        ("POLYLINE", CoordinateGraphic::Polyline),
        ("POLYGON", CoordinateGraphic::Polygon),
    ] {
        let mut item = content_item("111030", "SCOORD3D");
        item.put(DataElement::new(tags::GRAPHIC_TYPE, VR::CS, graphic));
        item.put(DataElement::new(
            tags::GRAPHIC_DATA,
            VR::FD,
            dicom_value!(F64, [0.0, 0.0, 0.0]),
        ));
        item.put(DataElement::new(
            tags::REFERENCED_FRAME_OF_REFERENCE_UID,
            VR::UI,
            source.frame_of_reference_uid().unwrap(),
        ));
        let coordinates = read_coordinates(&item, &source).unwrap();
        assert_eq!(coordinates.graphic, expected);
        assert_eq!(coordinates.points, vec![Point3::new(0.0, 0.0, 0.0)]);
    }

    let mut invalid = content_item("111030", "SCOORD3D");
    invalid.put(DataElement::new(tags::GRAPHIC_TYPE, VR::CS, "CIRCLE"));
    assert!(read_coordinates(&invalid, &source).is_err());
    invalid.put(DataElement::new(tags::GRAPHIC_TYPE, VR::CS, "POINT"));
    invalid.put(DataElement::new(
        tags::GRAPHIC_DATA,
        VR::FD,
        dicom_value!(F64, [0.0, 1.0]),
    ));
    assert!(read_coordinates(&invalid, &source).is_err());
    invalid.put(DataElement::new(
        tags::GRAPHIC_DATA,
        VR::FD,
        dicom_value!(F64, [0.0, 0.0, 0.0]),
    ));
    invalid.put(DataElement::new(
        tags::REFERENCED_FRAME_OF_REFERENCE_UID,
        VR::UI,
        "2.25.999",
    ));
    assert!(read_coordinates(&invalid, &source).is_err());

    let mut measured = InMemDicomObject::new_empty();
    measured.put(DataElement::new(tags::NUMERIC_VALUE, VR::DS, "2.5"));
    measured.put(sequence(
        tags::MEASUREMENT_UNITS_CODE_SEQUENCE,
        vec![code("mm", "millimeter")],
    ));
    let mut measurement = content_item("LENGTH", "NUM");
    measurement.put(sequence(tags::MEASURED_VALUE_SEQUENCE, vec![measured]));
    let parsed = read_measurement(&measurement, &source, &mut Vec::new()).unwrap();
    assert_eq!(parsed.value, 2.5);
    assert_eq!(parsed.unit.value(), "mm");
    assert!(parsed.coordinates.is_empty());

    measurement.put(sequence(tags::MEASURED_VALUE_SEQUENCE, Vec::new()));
    assert!(read_measurement(&measurement, &source, &mut Vec::new()).is_err());
}

#[test]
fn algorithm_group_and_segmentation_reference_readers_fail_closed_at_missing_links() {
    let (_directory, source) = source_context();
    assert!(read_algorithms(&[], &mut Vec::new()).unwrap().is_empty());

    let mut name = content_item("111001", "TEXT");
    name.put(DataElement::new(tags::TEXT_VALUE, VR::UT, "Model"));
    let mut version = content_item("111003", "TEXT");
    version.put(DataElement::new(tags::TEXT_VALUE, VR::UT, "1.0"));
    let mut family = content_item("111000", "CODE");
    family.put(sequence(
        tags::CONCEPT_CODE_SEQUENCE,
        vec![code("AI", "Artificial intelligence")],
    ));
    let mut name_code = content_item("111001", "CODE");
    name_code.put(sequence(
        tags::CONCEPT_CODE_SEQUENCE,
        vec![code("MODEL", "Model")],
    ));
    let mut parameters = content_item("111002", "TEXT");
    parameters.put(DataElement::new(tags::TEXT_VALUE, VR::UT, "threshold=0.5"));
    let mut algorithm_source = content_item("122405", "TEXT");
    algorithm_source.put(DataElement::new(
        tags::TEXT_VALUE,
        VR::UT,
        "https://example.invalid/model",
    ));
    let content = vec![
        name,
        version,
        family,
        name_code,
        parameters,
        algorithm_source,
    ];
    let algorithms = read_algorithms(&content, &mut Vec::new()).unwrap();
    assert_eq!(algorithms.len(), 1);
    assert_eq!(algorithms[0].name(), "Model");
    assert_eq!(
        find_code_value(&content, "111000", &mut Vec::new())
            .unwrap()
            .value(),
        "AI"
    );
    assert!(find_code_value(&content, "missing", &mut Vec::new()).is_err());

    let mut unexpected_group = InMemDicomObject::new_empty();
    let mut unexpected_template = InMemDicomObject::new_empty();
    unexpected_template.put(DataElement::new(tags::TEMPLATE_IDENTIFIER, VR::CS, "9999"));
    unexpected_group.put(sequence(
        tags::CONTENT_TEMPLATE_SEQUENCE,
        vec![unexpected_template],
    ));
    assert!(read_group(&unexpected_group, &source, None, &mut Vec::new()).is_err());

    let mut reference = InMemDicomObject::new_empty();
    reference.put(DataElement::new(
        tags::REFERENCED_SOP_CLASS_UID,
        VR::UI,
        uids::SEGMENTATION_STORAGE,
    ));
    reference.put(DataElement::new(
        tags::REFERENCED_SOP_INSTANCE_UID,
        VR::UI,
        "2.25.10",
    ));
    reference.put(DataElement::new(
        tags::REFERENCED_SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(1_u16),
    ));
    reference.put(DataElement::new(tags::REFERENCED_FRAME_NUMBER, VR::IS, "1"));
    let mut image = content_item("121214", "IMAGE");
    image.put(sequence(tags::REFERENCED_SOP_SEQUENCE, vec![reference]));
    let error = read_segmentation_reference(&image, "track", "2.25.11", None)
        .expect_err("missing supplied SEG should fail verification");
    assert!(error.to_string().contains("no SEG document was supplied"));
}

#[test]
fn measurement_and_coordinate_readers_reject_missing_or_mistyped_numeric_payloads() {
    let (_directory, source) = source_context();
    let mut measurement = content_item("LENGTH", "NUM");
    measurement.put(sequence(
        tags::MEASURED_VALUE_SEQUENCE,
        vec![InMemDicomObject::new_empty()],
    ));
    assert!(read_measurement(&measurement, &source, &mut Vec::new()).is_err());

    let mut invalid_value = InMemDicomObject::new_empty();
    invalid_value.put(DataElement::new(tags::NUMERIC_VALUE, VR::LO, "not-numeric"));
    invalid_value.put(sequence(
        tags::MEASUREMENT_UNITS_CODE_SEQUENCE,
        vec![code("mm", "millimeter")],
    ));
    measurement.put(sequence(tags::MEASURED_VALUE_SEQUENCE, vec![invalid_value]));
    assert!(read_measurement(&measurement, &source, &mut Vec::new()).is_err());

    let mut coordinates = content_item("111030", "SCOORD3D");
    coordinates.put(DataElement::new(tags::GRAPHIC_TYPE, VR::CS, "POINT"));
    assert!(read_coordinates(&coordinates, &source).is_err());
    coordinates.put(DataElement::new(tags::GRAPHIC_DATA, VR::LO, "invalid"));
    assert!(read_coordinates(&coordinates, &source).is_err());

    coordinates.put(DataElement::new(
        tags::GRAPHIC_DATA,
        VR::FD,
        dicom_value!(F64, [0.0, 0.0, 0.0]),
    ));
    coordinates.put(DataElement::new(
        tags::REFERENCED_FRAME_OF_REFERENCE_UID,
        VR::UI,
        source.frame_of_reference_uid().unwrap(),
    ));
    let mut measured = InMemDicomObject::new_empty();
    measured.put(DataElement::new(tags::NUMERIC_VALUE, VR::DS, "1.0"));
    measured.put(sequence(
        tags::MEASUREMENT_UNITS_CODE_SEQUENCE,
        vec![code("mm", "millimeter")],
    ));
    measurement.put(sequence(tags::MEASURED_VALUE_SEQUENCE, vec![measured]));
    measurement.put(sequence(tags::CONTENT_SEQUENCE, vec![coordinates]));
    assert_eq!(
        read_measurement(&measurement, &source, &mut Vec::new())
            .unwrap()
            .coordinates
            .len(),
        1
    );
}

#[test]
fn segmentation_references_validate_numbers_tracking_and_exact_sparse_frames() {
    use crate::{DicomCode, Point2, SegmentationSegment};

    let (_directory, source) = source_context();
    let segment = SegmentationSegment::new(
        "Tumor",
        DicomCode::new("MORPH", "99TEST", "Morphology").unwrap(),
        DicomCode::new("TUMOR", "99TEST", "Tumor").unwrap(),
        [1, 2, 3],
        vec![vec![
            Point2::new(0.0, 0.0),
            Point2::new(4.0, 0.0),
            Point2::new(4.0, 4.0),
            Point2::new(0.0, 4.0),
        ]],
        Vec::new(),
    )
    .unwrap()
    .with_tracking("track", "2.25.11")
    .unwrap();
    let segmentation = SegmentationDocument::binary(source.clone(), vec![segment]).unwrap();
    let image_for = |reference| {
        let mut image = content_item("121214", "IMAGE");
        image.put(sequence(tags::REFERENCED_SOP_SEQUENCE, vec![reference]));
        image
    };
    let mut reference = InMemDicomObject::new_empty();
    reference.put(DataElement::new(
        tags::REFERENCED_SOP_CLASS_UID,
        VR::UI,
        uids::SEGMENTATION_STORAGE,
    ));
    reference.put(DataElement::new(
        tags::REFERENCED_SOP_INSTANCE_UID,
        VR::UI,
        segmentation.sop_instance_uid(),
    ));
    reference.put(DataElement::new(
        tags::REFERENCED_SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(1_u16),
    ));
    reference.put(DataElement::new(tags::REFERENCED_FRAME_NUMBER, VR::IS, "1"));
    let parsed = read_segmentation_reference(
        &image_for(reference.clone()),
        "track",
        "2.25.11",
        Some(&segmentation),
    )
    .unwrap();
    assert_eq!(parsed.segment_number, 1);
    assert_eq!(parsed.frame_numbers, vec![1]);

    let mut missing_segment = reference.clone();
    missing_segment
        .take(tags::REFERENCED_SEGMENT_NUMBER)
        .unwrap();
    assert!(read_segmentation_reference(
        &image_for(missing_segment),
        "track",
        "2.25.11",
        Some(&segmentation),
    )
    .is_err());
    let mut invalid_segment = reference.clone();
    invalid_segment.put(DataElement::new(
        tags::REFERENCED_SEGMENT_NUMBER,
        VR::LO,
        "invalid",
    ));
    assert!(read_segmentation_reference(
        &image_for(invalid_segment),
        "track",
        "2.25.11",
        Some(&segmentation),
    )
    .is_err());
    let mut missing_frames = reference.clone();
    missing_frames.take(tags::REFERENCED_FRAME_NUMBER).unwrap();
    assert!(read_segmentation_reference(
        &image_for(missing_frames),
        "track",
        "2.25.11",
        Some(&segmentation),
    )
    .is_err());
    let mut invalid_frames = reference.clone();
    invalid_frames.put(DataElement::new(
        tags::REFERENCED_FRAME_NUMBER,
        VR::LO,
        "invalid",
    ));
    assert!(read_segmentation_reference(
        &image_for(invalid_frames),
        "track",
        "2.25.11",
        Some(&segmentation),
    )
    .is_err());
    let mut unknown_segment = reference;
    unknown_segment.put(DataElement::new(
        tags::REFERENCED_SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(2_u16),
    ));
    assert!(read_segmentation_reference(
        &image_for(unknown_segment),
        "track",
        "2.25.11",
        Some(&segmentation),
    )
    .is_err());
}

#[test]
fn algorithm_and_file_readers_report_missing_required_links() {
    let mut name = content_item("111001", "TEXT");
    name.put(DataElement::new(tags::TEXT_VALUE, VR::UT, "Model"));
    assert!(read_algorithms(std::slice::from_ref(&name), &mut Vec::new()).is_err());
    let mut version = content_item("111003", "TEXT");
    version.put(DataElement::new(tags::TEXT_VALUE, VR::UT, "1.0"));
    assert!(read_algorithms(&[name, version], &mut Vec::new()).is_err());

    let (_directory, source) = source_context();
    assert!(read_sr(Path::new("missing-report.dcm"), &source, None).is_err());
}
