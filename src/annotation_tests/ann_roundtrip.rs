use super::*;

#[test]
fn annotation_writers_preserve_required_specimen_and_deidentification_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let ann_path = directory.path().join("annotations.dcm");
    let seg_path = directory.path().join("segmentation.dcm");
    let rewritten_ann_path = directory.path().join("annotations-rewritten.dcm");
    let rewritten_seg_path = directory.path().join("segmentation-rewritten.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    let mut source_object = dicom_object::open_file(&source).unwrap();
    source_object.put(DataElement::new(
        tags::PATIENT_IDENTITY_REMOVED,
        VR::CS,
        "YES",
    ));
    source_object.put(DataElement::new(
        tags::DEIDENTIFICATION_METHOD,
        VR::LO,
        "Synthetic fixture",
    ));
    source_object.put(DataElement::new(
        tags::POSITION_REFERENCE_INDICATOR,
        VR::LO,
        "SLIDE_CORNER",
    ));
    add_specimen_metadata(&mut source_object);
    source_object.put(DataElement::new(
        tags::ACQUISITION_CONTEXT_SEQUENCE,
        VR::SQ,
        Value::from(DataSetSequence::new(Vec::new(), Length::UNDEFINED)),
    ));
    source_object.write_to_file(&source).unwrap();

    let context = DicomAnnotationContext::from_source(&source).unwrap();
    let category = code("MORPH", "Morphology");
    let property = code("TUMOR", "Tumor");
    let polygon = vec![
        Point2::new(0.0, 0.0),
        Point2::new(4.0, 0.0),
        Point2::new(4.0, 4.0),
        Point2::new(0.0, 4.0),
    ];
    AnnotationDocument::new(
        context.clone(),
        vec![AnnotationGroup::points(
            "Cell",
            category.clone(),
            property.clone(),
            [0, 0, 0],
            vec![Point2::new(1.0, 1.0)],
        )
        .unwrap()],
    )
    .unwrap()
    .write_ann(&ann_path)
    .unwrap();
    SegmentationDocument::binary(
        context.clone(),
        vec![SegmentationSegment::new(
            "Tumor",
            category,
            property,
            [0, 0, 0],
            vec![polygon],
            Vec::new(),
        )
        .unwrap()],
    )
    .unwrap()
    .write_seg(&seg_path)
    .unwrap();

    for path in [&ann_path, &seg_path] {
        let output = dicom_object::open_file(path).unwrap();
        assert!(output.element(tags::LATERALITY).is_err());
    }

    source_object.put(DataElement::new(tags::LATERALITY, VR::CS, "L"));
    source_object.write_to_file(&source).unwrap();

    for path in [&ann_path, &seg_path] {
        let mut object = dicom_object::open_file(path).unwrap();
        for (tag, vr, value) in [
            (tags::SERIES_NUMBER, VR::IS, "42"),
            (
                tags::SERIES_DESCRIPTION,
                VR::LO,
                "Imported annotation series",
            ),
            (tags::MANUFACTURER, VR::LO, "Synthetic Manufacturer"),
            (tags::MANUFACTURER_MODEL_NAME, VR::LO, "Synthetic Model"),
            (tags::DEVICE_SERIAL_NUMBER, VR::LO, "SERIAL-42"),
            (tags::SOFTWARE_VERSIONS, VR::LO, "42.0"),
        ] {
            object.put(DataElement::new(tag, vr, value));
        }
        object.write_to_file(path).unwrap();
    }

    AnnotationDocument::read_ann(&ann_path, &context)
        .unwrap()
        .revised()
        .write_ann(&rewritten_ann_path)
        .unwrap();
    SegmentationDocument::read_seg(&seg_path, &context)
        .unwrap()
        .revised()
        .write_seg(&rewritten_seg_path)
        .unwrap();

    for path in [rewritten_ann_path, rewritten_seg_path] {
        let output = dicom_object::open_file(path).unwrap();
        assert_eq!(
            output.element(tags::LATERALITY).unwrap().to_str().unwrap(),
            "L"
        );
        assert!(output
            .element(tags::ISSUER_OF_THE_CONTAINER_IDENTIFIER_SEQUENCE)
            .is_ok());
        assert!(output.element(tags::CONTAINER_TYPE_CODE_SEQUENCE).is_ok());
        assert!(output.element(tags::ACQUISITION_CONTEXT_SEQUENCE).is_err());
        assert_eq!(
            output
                .element(tags::PATIENT_IDENTITY_REMOVED)
                .unwrap()
                .to_str()
                .unwrap(),
            "YES"
        );
        assert_eq!(
            output
                .element(tags::DEIDENTIFICATION_METHOD)
                .unwrap()
                .to_str()
                .unwrap(),
            "Synthetic fixture"
        );
        for (tag, expected) in [
            (tags::SERIES_NUMBER, "42"),
            (tags::SERIES_DESCRIPTION, "Imported annotation series"),
            (tags::MANUFACTURER, "Synthetic Manufacturer"),
            (tags::MANUFACTURER_MODEL_NAME, "Synthetic Model"),
            (tags::DEVICE_SERIAL_NUMBER, "SERIAL-42"),
            (tags::SOFTWARE_VERSIONS, "42.0"),
        ] {
            assert_eq!(output.element(tag).unwrap().to_str().unwrap(), expected);
        }
        if output.element(tags::MODALITY).unwrap().to_str().unwrap() == "SEG" {
            assert_eq!(
                output
                    .element(tags::POSITION_REFERENCE_INDICATOR)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "SLIDE_CORNER"
            );
        }
    }
}

#[test]
fn extended_semantic_metadata_round_trips_through_ann_and_seg() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let ann_path = directory.path().join("annotations.dcm");
    let seg_path = directory.path().join("segmentation.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let property = DicomCode::new_long(
        "a-code-value-longer-than-sixteen-bytes",
        "99FRAMES",
        "Qualified tumor property",
    )
    .unwrap()
    .with_coding_scheme_version("2026")
    .unwrap()
    .with_context_identifier("99FRAMES-CID", "99FRAMES", "20260813")
    .unwrap()
    .with_context_uid("2.25.101", "2.25.102")
    .unwrap();
    let modifier = DicomCode::new_urn(
        "urn:frames:annotation:modifier:viable",
        "99FRAMES",
        "Viable",
    )
    .unwrap();
    let algorithm = AlgorithmIdentification::new(
        code("AI", "Artificial intelligence"),
        "Frames Tumor Model",
        "1.2.3",
    )
    .unwrap()
    .with_name_code(code("MODEL", "Frames Tumor Model"))
    .with_parameters("threshold=0.5")
    .unwrap()
    .with_source("https://example.invalid/model-card")
    .unwrap();

    let group = AnnotationGroup::points(
        "Automatic cells",
        code("MORPH", "Morphologically abnormal structure"),
        property.clone(),
        [65_535, 32_768, 32_768],
        vec![Point2::new(1.0, 2.0)],
    )
    .unwrap()
    .with_description("Generated by the locked research model")
    .unwrap()
    .with_generation(GenerationType::Automatic, vec![algorithm.clone()])
    .unwrap()
    .with_property_type_modifiers(vec![modifier.clone()])
    .with_anatomic_regions(vec![code("LUNG", "Lung")])
    .with_primary_anatomic_structures(vec![code("BRONCHUS", "Bronchus")])
    .with_referenced_optical_paths(vec!["OPTICAL-1".into()])
    .unwrap();
    AnnotationDocument::new(context.clone(), vec![group])
        .unwrap()
        .write_ann(&ann_path)
        .unwrap();

    let imported_ann = AnnotationDocument::read_ann(&ann_path, &context).unwrap();
    let imported_group = &imported_ann.groups()[0];
    assert_eq!(
        imported_group.description(),
        "Generated by the locked research model"
    );
    assert_eq!(imported_group.generation_type(), GenerationType::Automatic);
    assert_eq!(
        imported_group.algorithms(),
        std::slice::from_ref(&algorithm)
    );
    assert_eq!(imported_group.property_type(), &property);
    assert_eq!(
        imported_group.property_type().value_kind(),
        DicomCodeValueKind::Long
    );
    assert_eq!(
        imported_group.property_type_modifiers(),
        std::slice::from_ref(&modifier)
    );
    assert_eq!(imported_group.anatomic_regions(), &[code("LUNG", "Lung")]);
    assert_eq!(
        imported_group.primary_anatomic_structures(),
        &[code("BRONCHUS", "Bronchus")]
    );
    assert_eq!(imported_group.referenced_optical_paths(), &["OPTICAL-1"]);
    assert!(!imported_group.applies_to_all_optical_paths());
    assert!(imported_ann.diagnostics().is_empty());

    let polygon = vec![
        Point2::new(0.0, 0.0),
        Point2::new(4.0, 0.0),
        Point2::new(4.0, 4.0),
        Point2::new(0.0, 4.0),
    ];
    let segment = SegmentationSegment::new(
        "Automatic tumor",
        code("MORPH", "Morphologically abnormal structure"),
        property,
        [48_000, 40_000, 20_000],
        vec![polygon],
        Vec::new(),
    )
    .unwrap()
    .with_description("Binary tumor mask")
    .unwrap()
    .with_generation(GenerationType::Automatic, vec![algorithm.clone()])
    .unwrap()
    .with_property_type_modifiers(vec![modifier])
    .with_tracking("lesion-1", "2.25.103")
    .unwrap()
    .with_anatomic_regions(vec![code("LUNG", "Lung")]);
    SegmentationDocument::binary(context.clone(), vec![segment])
        .unwrap()
        .write_seg(&seg_path)
        .unwrap();

    let imported_seg = SegmentationDocument::read_seg(&seg_path, &context).unwrap();
    let imported_segment = &imported_seg.segments()[0];
    assert_eq!(imported_segment.description(), "Binary tumor mask");
    assert_eq!(
        imported_segment.generation_type(),
        GenerationType::Automatic
    );
    assert_eq!(imported_segment.algorithms(), &[algorithm]);
    assert_eq!(imported_segment.tracking_id(), Some("lesion-1"));
    assert_eq!(imported_segment.tracking_uid(), Some("2.25.103"));
    assert_eq!(imported_segment.anatomic_regions(), &[code("LUNG", "Lung")]);
    assert!(imported_seg.diagnostics().is_empty());
}

#[test]
fn ann_rewrite_rejects_known_semantic_loss_without_explicit_override() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let input = directory.path().join("input.dcm");
    let rejected_output = directory.path().join("rejected.dcm");
    let lossy_output = directory.path().join("lossy.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    let context = DicomAnnotationContext::from_source(&source).unwrap();
    AnnotationDocument::new(
        context.clone(),
        vec![AnnotationGroup::points(
            "Cells",
            code("MORPH", "Morphologically abnormal structure"),
            code("CELL", "Cell"),
            [0, 0, 0],
            vec![Point2::new(1.0, 1.0)],
        )
        .unwrap()],
    )
    .unwrap()
    .write_ann(&input)
    .unwrap();

    let mut object = dicom_object::open_file(&input).unwrap();
    let mut groups = object.take(tags::ANNOTATION_GROUP_SEQUENCE).unwrap();
    groups.update_value(|value| {
        let group = &mut value.items_mut().unwrap()[0];
        let mut property = group
            .take(tags::ANNOTATION_PROPERTY_TYPE_CODE_SEQUENCE)
            .unwrap();
        property.update_value(|value| {
            let code_item = &mut value.items_mut().unwrap()[0];
            let mut equivalent = InMemDicomObject::new_empty();
            equivalent.put(DataElement::new(tags::CODE_VALUE, VR::SH, "ALT"));
            equivalent.put(DataElement::new(
                tags::CODING_SCHEME_DESIGNATOR,
                VR::SH,
                "99FRAMES",
            ));
            equivalent.put(DataElement::new(
                tags::CODE_MEANING,
                VR::LO,
                "Equivalent cell",
            ));
            code_item.put(DataElement::new(
                tags::EQUIVALENT_CODE_SEQUENCE,
                VR::SQ,
                Value::from(DataSetSequence::new(vec![equivalent], Length::UNDEFINED)),
            ));
        });
        group.put(property);
    });
    object.put(groups);
    object.write_to_file(&input).unwrap();

    let imported = AnnotationDocument::read_ann(&input, &context).unwrap();
    assert_eq!(
        imported.diagnostics()[0].code(),
        "EQUIVALENT_CODE_SEQUENCE_WOULD_DROP"
    );
    assert!(imported.write_ann(&rejected_output).is_err());
    assert!(!rejected_output.exists());
    imported
        .write_ann_with_loss_policy(&lossy_output, true)
        .unwrap();
    assert!(lossy_output.exists());
}

#[test]
fn frame_relative_ann_round_trips_every_graphic_type_and_frame_reference() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let input = directory.path().join("frame-input.dcm");
    let output = directory.path().join("frame-output.dcm");
    write_source_wsi(&source, 8, 8, 4, 4);
    write_native_ann(
        &input,
        "2D",
        Some("FRAME"),
        Some(2),
        &[
            (
                AnnotationGraphicType::Point,
                vec![1.0, 1.0, 2.0, 2.0],
                vec![],
                None,
            ),
            (
                AnnotationGraphicType::Polyline,
                vec![0.0, 0.0, 1.0, 1.0, 2.0, 1.0],
                vec![1],
                None,
            ),
            (
                AnnotationGraphicType::Polygon,
                vec![0.0, 0.0, 0.0, 3.0, 3.0, 3.0, 3.0, 0.0],
                vec![1],
                None,
            ),
            (
                AnnotationGraphicType::Ellipse,
                vec![0.0, 1.0, 4.0, 1.0, 2.0, 0.0, 2.0, 2.0],
                vec![],
                None,
            ),
            (
                AnnotationGraphicType::Rectangle,
                vec![0.0, 0.0, 3.0, 0.0, 3.0, 2.0, 0.0, 2.0],
                vec![],
                None,
            ),
        ],
    );
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let imported = AnnotationDocument::read_ann(&input, &context).unwrap();
    assert_eq!(imported.referenced_frame_number(), Some(2));
    assert_eq!(
        imported
            .groups()
            .iter()
            .map(|group| group.geometry().graphic_type())
            .collect::<Vec<_>>(),
        vec![
            AnnotationGraphicType::Point,
            AnnotationGraphicType::Polyline,
            AnnotationGraphicType::Polygon,
            AnnotationGraphicType::Ellipse,
            AnnotationGraphicType::Rectangle,
        ]
    );

    imported.revised().write_ann(&output).unwrap();
    let round_tripped = AnnotationDocument::read_ann(&output, &context).unwrap();
    assert_eq!(round_tripped.referenced_frame_number(), Some(2));
    assert_eq!(round_tripped.groups(), imported.groups());
}

#[test]
fn three_dimensional_ann_preserves_common_z_and_xyz_and_normalizes_uniform_xyz() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let input = directory.path().join("3d-input.dcm");
    let output = directory.path().join("3d-output.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    write_native_ann(
        &input,
        "3D",
        None,
        None,
        &[
            (
                AnnotationGraphicType::Point,
                vec![0.001, 0.002, 0.003, 0.004],
                vec![],
                Some(vec![0.005]),
            ),
            (
                AnnotationGraphicType::Polyline,
                vec![0.0, 0.0, 0.001, 0.002, 0.0, 0.002],
                vec![1],
                None,
            ),
            (
                AnnotationGraphicType::Point,
                vec![0.001, 0.001, 0.007, 0.002, 0.002, 0.007],
                vec![],
                None,
            ),
        ],
    );
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let imported = AnnotationDocument::read_ann(&input, &context).unwrap();
    assert_eq!(imported.coordinate_type(), "3D");
    assert_eq!(imported.groups()[0].common_z_coordinates(), &[0.005]);
    assert!(matches!(
        imported.groups()[1].geometry(),
        AnnotationGeometry::ReadOnly {
            coordinate_dimensions: 3,
            ..
        }
    ));
    assert_eq!(imported.groups()[2].common_z_coordinates(), &[0.007]);
    assert!(matches!(
        imported.groups()[2].geometry(),
        AnnotationGeometry::ReadOnly {
            coordinate_dimensions: 2,
            ..
        }
    ));
    assert!(imported
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code() == "UNIFORM_Z_FACTORED"));

    imported.revised().write_ann(&output).unwrap();
    let round_tripped = AnnotationDocument::read_ann(&output, &context).unwrap();
    assert_eq!(round_tripped.groups(), imported.groups());
}

#[test]
fn ann_rejects_non_monotonic_primitive_indices() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    let input = directory.path().join("invalid-indices.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    write_native_ann(
        &input,
        "2D",
        Some("VOLUME"),
        None,
        &[(
            AnnotationGraphicType::Polyline,
            vec![0.0, 0.0, 1.0, 1.0, 2.0, 1.0, 3.0, 2.0],
            vec![1, 1],
            None,
        )],
    );
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let result = AnnotationDocument::read_ann(&input, &context);

    assert!(result.is_err());
}

#[test]
fn ann_round_trip_preserves_points_polygons_codes_color_and_measurements() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.dcm");
    let output = dir.path().join("annotations.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    let source_before = std::fs::read(&source).unwrap();
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let mut cells = AnnotationGroup::points(
        "Tumor cells",
        code("MORPH", "Morphologically abnormal structure"),
        code("CELL", "Tumor cell"),
        [65_535, 32_768, 32_768],
        vec![Point2::new(1.25, 2.5), Point2::new(9.0, 4.0)],
    )
    .unwrap();
    cells
        .add_measurement(AnnotationMeasurement::new(
            code("AREA", "Cell area"),
            DicomCode::new("um2", "UCUM", "square micrometer").unwrap(),
            vec![12.5, 18.75],
        ))
        .unwrap();
    let regions = AnnotationGroup::polygons(
        "Viable tumor",
        code("MORPH", "Morphologically abnormal structure"),
        code("VIABLE_TUMOR", "Viable tumor"),
        [48_000, 40_000, 20_000],
        vec![
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(0.0, 4.0),
                Point2::new(4.0, 4.0),
                Point2::new(4.0, 0.0),
            ],
            vec![
                Point2::new(8.0, 2.0),
                Point2::new(8.0, 6.0),
                Point2::new(12.0, 6.0),
                Point2::new(12.0, 2.0),
            ],
        ],
    )
    .unwrap();
    let original = AnnotationDocument::new(context.clone(), vec![cells, regions]).unwrap();

    original.write_ann(&output).unwrap();
    let imported = AnnotationDocument::read_ann(&output, &context).unwrap();

    assert_eq!(imported.groups().len(), 2);
    assert_eq!(imported.groups()[0].point_annotations().unwrap().len(), 2);
    assert_eq!(
        imported.groups()[0].measurements()[0].values(),
        &[12.5, 18.75]
    );
    assert_eq!(imported.groups()[1].polygon_annotations().unwrap().len(), 2);
    assert_eq!(
        imported.groups()[1].recommended_display_cielab(),
        [48_000, 40_000, 20_000]
    );
    assert_eq!(
        imported.source().sop_instance_uid(),
        context.sop_instance_uid()
    );

    let raw = dicom_object::open_file(&output).unwrap();
    assert_eq!(
        raw.meta().transfer_syntax(),
        uids::EXPLICIT_VR_LITTLE_ENDIAN
    );
    assert_eq!(
        raw.element(tags::MODALITY).unwrap().to_str().unwrap(),
        "ANN"
    );
    assert_eq!(
        raw.element(tags::ANNOTATION_COORDINATE_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "2D"
    );
    assert_eq!(
        raw.element(tags::PIXEL_ORIGIN_INTERPRETATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "VOLUME"
    );
    let groups = raw
        .element(tags::ANNOTATION_GROUP_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    assert_eq!(
        groups[1]
            .element(tags::LONG_PRIMITIVE_POINT_INDEX_LIST)
            .unwrap()
            .to_multi_int::<u32>()
            .unwrap(),
        vec![1, 9]
    );
    let coordinates = groups[1]
        .element(tags::DOUBLE_POINT_COORDINATES_DATA)
        .unwrap()
        .to_multi_float64()
        .unwrap();
    let polygon = coordinates[0..8]
        .chunks_exact(2)
        .map(|point| Point2::new(point[0], point[1]))
        .collect::<Vec<_>>();
    assert!(
        polygon_signed_area(&polygon) > 0.0,
        "DICOM pixel polygons must be clockwise on screen"
    );
    assert_ne!(
        coordinates[0..2],
        coordinates[6..8],
        "ANN closes polygons implicitly"
    );
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
}

#[test]
fn editing_an_imported_ann_creates_a_new_sop_instance_uid() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    let context = DicomAnnotationContext::from_source(&source).unwrap();
    let original = AnnotationDocument::new(
        context,
        vec![AnnotationGroup::points(
            "Cells",
            code("MORPH", "Morphologically abnormal structure"),
            code("CELL", "Cell"),
            [0, 0, 0],
            vec![Point2::new(1.0, 1.0)],
        )
        .unwrap()],
    )
    .unwrap();

    let revised = original.revised();

    assert_ne!(revised.sop_instance_uid(), original.sop_instance_uid());
    assert_eq!(
        revised.predecessor_sop_instance_uid(),
        Some(original.sop_instance_uid())
    );
    assert_eq!(revised.groups()[0].uid(), original.groups()[0].uid());
}

#[test]
fn core_sidecar_writers_refuse_to_replace_the_source_wsi() {
    let directory = tempfile::tempdir().unwrap();
    let ann_source = directory.path().join("ann-source.dcm");
    write_source_wsi(&ann_source, 16, 12, 4, 4);
    let ann_source_bytes = std::fs::read(&ann_source).unwrap();
    let ann_context = DicomAnnotationContext::from_source(&ann_source).unwrap();
    let annotations = AnnotationDocument::new(
        ann_context,
        vec![AnnotationGroup::points(
            "Cells",
            code("MORPH", "Morphologically abnormal structure"),
            code("CELL", "Cell"),
            [0, 0, 0],
            vec![Point2::new(1.0, 1.0)],
        )
        .unwrap()],
    )
    .unwrap();
    assert!(annotations.write_ann(&ann_source).is_err());
    assert_eq!(std::fs::read(&ann_source).unwrap(), ann_source_bytes);

    let seg_source = directory.path().join("seg-source.dcm");
    write_source_wsi(&seg_source, 16, 12, 4, 4);
    let seg_source_bytes = std::fs::read(&seg_source).unwrap();
    let seg_context = DicomAnnotationContext::from_source(&seg_source).unwrap();
    let segmentation = SegmentationDocument::binary(
        seg_context,
        vec![SegmentationSegment::new(
            "Tumor",
            code("MORPH", "Morphologically abnormal structure"),
            code("TUMOR", "Tumor"),
            [0, 0, 0],
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
    .unwrap();
    assert!(segmentation.write_seg(&seg_source).is_err());
    assert_eq!(std::fs::read(&seg_source).unwrap(), seg_source_bytes);
}
