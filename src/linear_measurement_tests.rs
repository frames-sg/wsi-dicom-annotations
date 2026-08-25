use crate::test_support::write_source_wsi_with_spacing;
use crate::{
    AnnotationScheme, DicomAnnotationContext, LinearMeasurementSpec, MeasurementReportSemantics,
    Point2, StructuredReportDocument, StructuredReportReferenceKind, TrackingIdentity,
};

#[test]
fn linear_measurement_sr_preserves_tracking_endpoints_and_anisotropic_distance() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    write_source_wsi_with_spacing(&source_path, 32, 24, 8, 8, 0.000_25);
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let class = AnnotationScheme::general_pathology_v1()
        .class("neoplasm")
        .unwrap()
        .clone();
    let tracking = TrackingIdentity::new("F-000001", "2.25.123456789").unwrap();
    let spec = LinearMeasurementSpec::new(
        tracking.clone(),
        class.category().clone(),
        class.property_type().clone(),
        Point2::new(4.0, 4.0),
        Point2::new(7.0, 8.0),
    )
    .unwrap();
    let semantics = MeasurementReportSemantics::pathology_v1();

    let first = StructuredReportDocument::from_linear_measurements(
        source.clone(),
        &semantics,
        std::slice::from_ref(&spec),
    )
    .unwrap();
    let second =
        StructuredReportDocument::from_linear_measurements(source.clone(), &semantics, &[spec])
            .unwrap();
    assert_ne!(first.sop_instance_uid(), second.sop_instance_uid());
    assert_eq!(first.groups()[0].tracking_id(), tracking.id());
    assert_eq!(first.groups()[0].tracking_uid(), tracking.uid());
    assert_eq!(
        first.groups()[0].reference_kind(),
        StructuredReportReferenceKind::MeasurementCoordinates
    );
    let measurement = &first.groups()[0].measurements()[0];
    assert!((measurement.value() - 0.001_25).abs() < 1e-12);
    assert_eq!(measurement.concept().value(), "410668003");
    assert_eq!(measurement.unit().value(), "mm");
    assert_eq!(measurement.coordinates()[0].points().len(), 2);

    let output = directory.path().join("measurements.dcm");
    first.write_sr(&output).unwrap();
    let restored = StructuredReportDocument::read_sr(&output, &source, None).unwrap();
    assert_eq!(restored.groups()[0].tracking_id(), tracking.id());
    assert_eq!(restored.groups()[0].tracking_uid(), tracking.uid());
    assert!((restored.groups()[0].measurements()[0].value() - 0.001_25).abs() < 1e-12);
    assert_eq!(
        restored.groups()[0].measurements()[0].coordinates()[0]
            .points()
            .len(),
        2
    );
}

#[test]
fn linear_measurement_sr_rejects_zero_length_and_missing_measurements() {
    let scheme = AnnotationScheme::general_pathology_v1();
    let class = scheme.class("tissue").unwrap();
    let point = Point2::new(2.0, 2.0);
    let error = LinearMeasurementSpec::new(
        TrackingIdentity::new("F-000001", "2.25.1").unwrap(),
        class.category().clone(),
        class.property_type().clone(),
        point,
        point,
    )
    .expect_err("zero-length ruler");
    assert!(error.to_string().contains("distinct endpoints"));
}
