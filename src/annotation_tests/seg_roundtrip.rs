use super::*;

#[test]
fn sparse_binary_segmentation_preserves_a_hole_and_omits_empty_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.dcm");
    let output = dir.path().join("mask.dcm");
    write_source_wsi(&source, 12, 8, 4, 4);
    let source_before = std::fs::read(&source).unwrap();
    let context = DicomAnnotationContext::from_source(&source).unwrap();
    let segment = SegmentationSegment::new(
        "Viable tumor",
        code("MORPH", "Morphologically abnormal structure"),
        code("VIABLE_TUMOR", "Viable tumor"),
        [48_000, 40_000, 20_000],
        vec![vec![
            Point2::new(0.0, 0.0),
            Point2::new(8.0, 0.0),
            Point2::new(8.0, 8.0),
            Point2::new(0.0, 8.0),
        ]],
        vec![vec![
            Point2::new(2.0, 2.0),
            Point2::new(6.0, 2.0),
            Point2::new(6.0, 6.0),
            Point2::new(2.0, 6.0),
        ]],
    )
    .unwrap();
    let segmentation = SegmentationDocument::binary(context.clone(), vec![segment]).unwrap();

    let frames = segmentation.rasterized_frames().unwrap();

    assert_eq!(
        frames.len(),
        4,
        "the empty right-most tile column must be omitted"
    );
    assert!(frames
        .iter()
        .all(|frame| frame.mask().iter().any(|value| *value)));
    assert!(
        !frames
            .iter()
            .find(|frame| frame.tile_col() == 0 && frame.tile_row() == 0)
            .unwrap()
            .mask()[2 * 4 + 2]
    );

    segmentation.write_seg(&output).unwrap();
    let imported = SegmentationDocument::read_seg(&output, &context).unwrap();
    assert_eq!(imported.rasterized_frames().unwrap(), frames);
    assert_eq!(
        dicom_object::open_file(&output)
            .unwrap()
            .element(tags::DIMENSION_ORGANIZATION_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "TILED_SPARSE"
    );
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
}

#[test]
fn segmentation_row_runs_merge_tile_boundaries_and_have_a_stable_digest() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let output = directory.path().join("segmentation.dcm");
    write_source_wsi(&source, 8, 4, 4, 4);
    let context = DicomAnnotationContext::from_source(&source).unwrap();
    let segmentation = SegmentationDocument::binary(
        context.clone(),
        vec![SegmentationSegment::new(
            "Tumor",
            code("MORPH", "Morphologically abnormal structure"),
            code("TUMOR", "Tumor"),
            [0, 0, 0],
            vec![vec![
                Point2::new(0.0, 0.0),
                Point2::new(8.0, 0.0),
                Point2::new(8.0, 4.0),
                Point2::new(0.0, 4.0),
            ]],
            Vec::new(),
        )
        .unwrap()],
    )
    .unwrap();

    let runs = segmentation.binary_runs().unwrap();
    assert_eq!(runs.len(), 4);
    assert!(runs.iter().enumerate().all(|(row, run)| {
        run.segment_number() == 1
            && run.row() == u32::try_from(row).unwrap()
            && run.column_start() == 0
            && run.length() == 8
    }));
    let digest = segmentation.mask_digest().unwrap();
    segmentation.write_seg(&output).unwrap();
    let imported = SegmentationDocument::read_seg(&output, &context).unwrap();
    assert_eq!(imported.binary_runs().unwrap(), runs);
    assert_eq!(imported.mask_digest().unwrap(), digest);
    assert_ne!(
        imported.revised().sop_instance_uid(),
        imported.sop_instance_uid()
    );
    assert_eq!(
        imported.revised().series_instance_uid(),
        imported.series_instance_uid()
    );
}

#[test]
fn fractional_segmentation_rewrite_fails_without_creating_output() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let input = directory.path().join("fractional.dcm");
    let output = directory.path().join("must-not-exist.dcm");
    write_source_wsi(&source, 4, 4, 4, 4);
    write_fractional_seg(&input, [1; 16]);
    let context = DicomAnnotationContext::from_source(&source).unwrap();
    let segmentation = SegmentationDocument::read_seg(&input, &context).unwrap();

    assert!(segmentation.write_seg(&output).is_err());
    assert!(!output.exists());
    assert!(!segmentation.fractional_runs().unwrap().is_empty());
}

#[test]
fn segmentation_import_requires_an_exact_source_sop_reference() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let segmentation_path = directory.path().join("segmentation.dcm");
    write_source_wsi(&source, 4, 4, 4, 4);
    write_fractional_seg(&segmentation_path, [1; 16]);
    let mut object = dicom_object::open_file(&segmentation_path).unwrap();
    let mut shared = object
        .take(tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE)
        .unwrap();
    shared.update_value(|value| {
        let shared = &mut value.items_mut().unwrap()[0];
        let mut derivations = shared.take(tags::DERIVATION_IMAGE_SEQUENCE).unwrap();
        derivations.update_value(|value| {
            let derivation = &mut value.items_mut().unwrap()[0];
            let mut sources = derivation.take(tags::SOURCE_IMAGE_SEQUENCE).unwrap();
            sources.update_value(|value| {
                value.items_mut().unwrap()[0].put(DataElement::new(
                    tags::REFERENCED_SOP_INSTANCE_UID,
                    VR::UI,
                    "2.25.999",
                ));
            });
            derivation.put(sources);
        });
        shared.put(derivations);
    });
    object.put(shared);
    object.write_to_file(&segmentation_path).unwrap();
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    assert!(SegmentationDocument::read_seg(&segmentation_path, &context).is_err());
}

#[test]
fn sparse_segmentation_work_is_bounded_by_annotated_tiles_not_slide_area() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("large-source.dcm");
    write_source_wsi(&source, 1_000_000, 1_000_000, 256, 256);
    let context = DicomAnnotationContext::from_source(&source).unwrap();
    let polygon = vec![
        Point2::new(8.0, 8.0),
        Point2::new(24.0, 8.0),
        Point2::new(24.0, 24.0),
        Point2::new(8.0, 24.0),
    ];
    let segmentation = SegmentationDocument::binary(
        context,
        vec![SegmentationSegment::new(
            "Small focus",
            code("MORPH", "Morphologically abnormal structure"),
            code("FOCUS", "Small focus"),
            [48_000, 40_000, 20_000],
            vec![polygon],
            Vec::new(),
        )
        .unwrap()],
    )
    .unwrap();

    let frames = segmentation.rasterized_frames().unwrap();

    assert_eq!(frames.len(), 1);
    assert_eq!((frames[0].tile_col(), frames[0].tile_row()), (0, 0));
    assert_eq!(frames[0].segment_number(), 1);
    assert_eq!(frames[0].dimensions(), (256, 256));
    assert_eq!(frames[0].mask().iter().filter(|value| **value).count(), 256);
}

#[test]
fn fractional_segmentation_import_preserves_raster_values_as_read_only() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let sidecar = directory.path().join("fractional-seg.dcm");
    write_source_wsi(&source, 4, 4, 4, 4);
    write_fractional_seg(
        &sidecar,
        [
            0, 32, 64, 96, 128, 160, 192, 224, 255, 224, 192, 160, 128, 96, 64, 32,
        ],
    );
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let imported = SegmentationDocument::read_seg(&sidecar, &context).unwrap();

    assert_eq!(imported.kind(), crate::SegmentationKind::Fractional);
    assert!(!imported.editable());
    let frames = imported.fractional_frames().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].segment_number(), 1);
    assert_eq!(frames[0].dimensions(), (4, 4));
    assert_eq!(frames[0].maximum_fractional_value(), 255);
    assert_eq!(frames[0].values()[8], 255);
    assert_eq!((frames[0].tile_col(), frames[0].tile_row()), (0, 0));
    let runs = imported.fractional_runs().unwrap();
    assert_eq!(runs.len(), 4);
    assert_eq!(runs[0].segment_number(), 1);
    assert_eq!(runs[0].row(), 0);
    assert_eq!(runs[0].column_start(), 1);
    assert_eq!(runs[0].maximum_fractional_value(), 255);
    assert_eq!(runs[0].values(), &[32, 64, 96]);
    assert_eq!(imported.mask_digest().unwrap().len(), 64);
    assert!(imported.binary_runs().is_err());
    assert!(imported.rasterized_frames().is_err());
    assert!(imported
        .vectorized_annotations(crate::SegToAnnConversionPolicy::RejectLoss)
        .is_err());
    assert!(imported
        .write_seg(directory.path().join("fractional-copy.dcm"))
        .is_err());
    assert!(imported.segment_for_number(1).is_some());
    assert!(imported.segment_for_number(2).is_none());
    assert_ne!(
        imported.revised().sop_instance_uid(),
        imported.sop_instance_uid()
    );
    let discovered = discover_sidecars(&context).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].kind(), SidecarKind::FractionalSegmentation);
}

#[test]
fn tiled_full_binary_segmentation_derives_frame_positions() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let segmentation_path = directory.path().join("tiled-full-seg.dcm");
    write_source_wsi(&source, 8, 4, 4, 4);
    let context = DicomAnnotationContext::from_source(&source).unwrap();
    let segmentation = SegmentationDocument::binary(
        context.clone(),
        vec![SegmentationSegment::new(
            "Tumor",
            code("MORPH", "Morphology"),
            code("TUMOR", "Tumor"),
            [0, 0, 0],
            vec![vec![
                Point2::new(0.0, 0.0),
                Point2::new(8.0, 0.0),
                Point2::new(8.0, 4.0),
                Point2::new(0.0, 4.0),
            ]],
            Vec::new(),
        )
        .unwrap()],
    )
    .unwrap();
    let expected_runs = segmentation.binary_runs().unwrap();
    segmentation.write_seg(&segmentation_path).unwrap();
    let mut object = dicom_object::open_file(&segmentation_path).unwrap();
    object
        .take(tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE)
        .unwrap();
    object.put(DataElement::new(
        tags::DIMENSION_ORGANIZATION_TYPE,
        VR::CS,
        "TILED_FULL",
    ));
    object.write_to_file(&segmentation_path).unwrap();

    let imported = SegmentationDocument::read_seg(&segmentation_path, &context).unwrap();

    assert_eq!(imported.binary_runs().unwrap(), expected_runs);
}

#[test]
fn tiled_full_fractional_segmentation_derives_frame_positions() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let segmentation_path = directory.path().join("tiled-full-fractional.dcm");
    write_source_wsi(&source, 4, 4, 4, 4);
    write_fractional_seg(&segmentation_path, [128; 16]);
    let mut object = dicom_object::open_file(&segmentation_path).unwrap();
    object
        .take(tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE)
        .unwrap();
    for element in [
        DataElement::new(tags::DIMENSION_ORGANIZATION_TYPE, VR::CS, "TILED_FULL"),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_ROWS,
            VR::UL,
            PrimitiveValue::from(4_u32),
        ),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_COLUMNS,
            VR::UL,
            PrimitiveValue::from(4_u32),
        ),
    ] {
        object.put(element);
    }
    object.write_to_file(&segmentation_path).unwrap();
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let imported = SegmentationDocument::read_seg(&segmentation_path, &context).unwrap();

    assert_eq!(imported.fractional_frames().unwrap().len(), 1);
    assert_eq!(
        imported.fractional_frames().unwrap()[0].values(),
        &[128; 16]
    );
}

#[test]
fn labelmap_background_segment_zero_is_inspectable_but_not_a_mask() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let segmentation_path = directory.path().join("labelmap.dcm");
    write_source_wsi(&source, 4, 4, 4, 4);
    write_fractional_seg(&segmentation_path, [0; 16]);
    let mut object = dicom_object::open_file(&segmentation_path).unwrap();
    object
        .take(tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE)
        .unwrap();
    object.take(tags::SEGMENTATION_FRACTIONAL_TYPE).unwrap();
    object.take(tags::MAXIMUM_FRACTIONAL_VALUE).unwrap();
    let mut segments = object.take(tags::SEGMENT_SEQUENCE).unwrap();
    segments.update_value(|value| {
        value.items_mut().unwrap()[0].put(DataElement::new(
            tags::SEGMENT_NUMBER,
            VR::US,
            PrimitiveValue::from(0_u16),
        ));
    });
    object.put(segments);
    for element in [
        DataElement::new(tags::SEGMENTATION_TYPE, VR::CS, "LABELMAP"),
        DataElement::new(tags::DIMENSION_ORGANIZATION_TYPE, VR::CS, "TILED_FULL"),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_ROWS,
            VR::UL,
            PrimitiveValue::from(4_u32),
        ),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_COLUMNS,
            VR::UL,
            PrimitiveValue::from(4_u32),
        ),
    ] {
        object.put(element);
    }
    object.write_to_file(&segmentation_path).unwrap();
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let imported = SegmentationDocument::read_seg(&segmentation_path, &context).unwrap();

    assert_eq!(imported.kind(), crate::SegmentationKind::LabelMap);
    assert_eq!(imported.segments()[0].source_segment_number(), Some(0));
    assert!(imported.binary_runs().unwrap().is_empty());
}
