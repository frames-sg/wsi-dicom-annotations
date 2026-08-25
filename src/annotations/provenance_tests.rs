use dicom_dictionary_std::tags;

use crate::test_support::write_source_wsi;
use crate::{
    AnnotationDocument, AnnotationGroup, DerivedObjectProducer, DicomAnnotationContext, DicomCode,
    LinearMeasurementSpec, MeasurementReportSemantics, Point2, SegmentationDocument,
    SegmentationSegment, StructuredReportDocument, TrackingIdentity,
};

fn code(value: &str, meaning: &str) -> DicomCode {
    DicomCode::new(value, "99FRAMES", meaning).unwrap()
}

fn producer(series_number: i32) -> DerivedObjectProducer {
    DerivedObjectProducer::new(
        series_number,
        "Acme Pathology",
        "Acme Slide Workstation",
        "ACME-42",
        "7.3.1",
    )
    .unwrap()
    .with_series_description("Acme derived pathology objects")
    .unwrap()
}

fn assert_producer(path: &std::path::Path, expected_series: &str) {
    let object = dicom_object::open_file(path).unwrap();
    for (tag, expected) in [
        (tags::SERIES_NUMBER, expected_series),
        (tags::SERIES_DESCRIPTION, "Acme derived pathology objects"),
        (tags::MANUFACTURER, "Acme Pathology"),
        (tags::MANUFACTURER_MODEL_NAME, "Acme Slide Workstation"),
        (tags::DEVICE_SERIAL_NUMBER, "ACME-42"),
        (tags::SOFTWARE_VERSIONS, "7.3.1"),
    ] {
        assert_eq!(
            object.element(tag).unwrap().to_str().unwrap().trim(),
            expected
        );
    }
}

#[test]
fn caller_owned_producer_is_written_by_ann_seg_and_sr() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    write_source_wsi(&source_path, 16, 12, 4, 4);
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let category = code("MORPH", "Morphology");
    let property = code("TUMOR", "Tumor");

    let ann_path = directory.path().join("annotations.dcm");
    AnnotationDocument::new(
        source.clone(),
        vec![AnnotationGroup::points(
            "Cells",
            category.clone(),
            property.clone(),
            [1, 2, 3],
            vec![Point2::new(1.0, 1.0)],
        )
        .unwrap()],
    )
    .unwrap()
    .with_producer(producer(41))
    .write_ann(&ann_path)
    .unwrap();
    assert_producer(&ann_path, "41");

    let seg_path = directory.path().join("segmentation.dcm");
    SegmentationDocument::binary(
        source.clone(),
        vec![SegmentationSegment::new(
            "Tumor",
            category,
            property,
            [1, 2, 3],
            vec![vec![
                Point2::new(0.0, 0.0),
                Point2::new(4.0, 0.0),
                Point2::new(4.0, 4.0),
                Point2::new(0.0, 4.0),
            ]],
            Vec::new(),
        )
        .unwrap()],
    )
    .unwrap()
    .with_producer(producer(42))
    .write_seg(&seg_path)
    .unwrap();
    assert_producer(&seg_path, "42");

    let sr_path = directory.path().join("measurements.dcm");
    let measurement = LinearMeasurementSpec::new(
        TrackingIdentity::new("length-1", "2.25.42").unwrap(),
        code("MORPH", "Morphology"),
        code("TUMOR", "Tumor"),
        Point2::new(1.0, 1.0),
        Point2::new(3.0, 1.0),
    )
    .unwrap();
    StructuredReportDocument::from_linear_measurements(
        source,
        &MeasurementReportSemantics::pathology_v1(),
        &[measurement],
    )
    .unwrap()
    .with_producer(producer(43))
    .write_sr(&sr_path)
    .unwrap();
    assert_producer(&sr_path, "43");

    let rewritten_sr = directory.path().join("measurements-rewritten.dcm");
    StructuredReportDocument::read_sr(
        &sr_path,
        &DicomAnnotationContext::from_source(&source_path).unwrap(),
        None,
    )
    .unwrap()
    .write_sr(&rewritten_sr)
    .unwrap();
    assert_producer(&rewritten_sr, "43");
}

#[test]
fn producer_metadata_rejects_invalid_dicom_text() {
    assert!(DerivedObjectProducer::new(1, "Acme\\Other", "Model", "Serial", "1.0").is_err());
    assert!(DerivedObjectProducer::new(1, "Acme", "Model\0bad", "Serial", "1.0").is_err());
    assert!(producer(1).with_series_description("").is_err());
}

#[test]
fn generic_default_identifies_the_library_not_frames_dicom_viewer() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let output_path = directory.path().join("annotations.dcm");
    write_source_wsi(&source_path, 16, 12, 4, 4);
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    AnnotationDocument::new(
        source,
        vec![AnnotationGroup::points(
            "Cells",
            code("MORPH", "Morphology"),
            code("CELL", "Cell"),
            [1, 2, 3],
            vec![Point2::new(1.0, 1.0)],
        )
        .unwrap()],
    )
    .unwrap()
    .write_ann(&output_path)
    .unwrap();

    let object = dicom_object::open_file(output_path).unwrap();
    assert_ne!(
        object
            .element(tags::MANUFACTURER_MODEL_NAME)
            .unwrap()
            .to_str()
            .unwrap()
            .trim(),
        "DICOM Viewer"
    );
    assert_eq!(
        object
            .element(tags::MANUFACTURER_MODEL_NAME)
            .unwrap()
            .to_str()
            .unwrap()
            .trim(),
        "wsi-dicom-annotations"
    );
}
