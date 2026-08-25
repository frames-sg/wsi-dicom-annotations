use super::*;

#[test]
fn slide_coordinates_project_back_to_total_matrix_pixels() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    write_source_wsi(&source, 100, 100, 4, 4);
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let point = context.slide_coordinate_to_pixel(0.001, 0.002).unwrap();

    assert!((point.x - 4.0).abs() < 1e-9);
    assert!((point.y - 8.0).abs() < 1e-9);
}

#[test]
fn frame_and_slide_coordinates_canonicalize_to_level_zero_pixels() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let canonical = directory.path().join("canonical.dcm");
    let frame_ann = directory.path().join("frame-ann.dcm");
    let three_d_ann = directory.path().join("3d-ann.dcm");
    write_source_wsi_with_spacing(&source, 8, 8, 4, 4, 0.00025);
    write_source_wsi_with_spacing(&canonical, 16, 16, 4, 4, 0.000125);
    write_native_ann(
        &frame_ann,
        "2D",
        Some("FRAME"),
        Some(2),
        &[(AnnotationGraphicType::Point, vec![1.0, 2.0], vec![], None)],
    );
    write_native_ann(
        &three_d_ann,
        "3D",
        None,
        None,
        &[(
            AnnotationGraphicType::Point,
            vec![0.00125, 0.0005],
            vec![],
            Some(vec![0.0]),
        )],
    );
    let source_context = DicomAnnotationContext::from_source(&source).unwrap();
    let canonical_context = DicomAnnotationContext::from_source(&canonical).unwrap();

    let total_pixel = source_context
        .frame_coordinate_to_total_pixel(2, 1.0, 2.0)
        .unwrap();
    assert_eq!(total_pixel, Point2::new(5.0, 2.0));
    let slide = source_context
        .pixel_to_slide_coordinate(total_pixel.x, total_pixel.y)
        .unwrap();
    assert!((slide.x - 0.00125).abs() < 1e-12);
    assert!((slide.y - 0.0005).abs() < 1e-12);
    assert_eq!(
        source_context
            .slide_coordinate_to_pixel3(slide.x, slide.y, slide.z)
            .unwrap(),
        total_pixel
    );
    let annotations = AnnotationDocument::read_ann(&frame_ann, &source_context).unwrap();
    let canonical_pixel = annotations
        .canonical_level0_pixel(&canonical_context, 1.0, 2.0, None)
        .unwrap();
    assert!((canonical_pixel.x - 10.0).abs() < 1e-9);
    assert!((canonical_pixel.y - 4.0).abs() < 1e-9);
    let three_d = AnnotationDocument::read_ann(&three_d_ann, &source_context).unwrap();
    let three_d_pixel = three_d
        .canonical_level0_pixel(&canonical_context, 0.00125, 0.0005, Some(0.0))
        .unwrap();
    assert!((three_d_pixel.x - 10.0).abs() < 1e-9);
    assert!((three_d_pixel.y - 4.0).abs() < 1e-9);
}

#[test]
fn sparse_frame_projection_uses_per_frame_plane_position() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("sparse-source.dcm");
    write_sparse_source_wsi(&source);
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let point = context
        .frame_coordinate_to_total_pixel(2, 1.0, 2.0)
        .unwrap();

    assert_eq!(point, Point2::new(5.0, 6.0));
}
