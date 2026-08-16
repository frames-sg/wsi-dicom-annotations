use std::path::Path;

use dicom_core::value::{DataSetSequence, PrimitiveValue, Value};
use dicom_core::{DataElement, Length, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

use crate::{
    discover_sidecars, polygon_signed_area, AlgorithmIdentification, AnnotationDocument,
    AnnotationGeometry, AnnotationGraphicType, AnnotationGroup, AnnotationMeasurement,
    DicomAnnotationContext, DicomCode, DicomCodeValueKind, GenerationType, Point2,
    SegmentationDocument, SegmentationSegment, SidecarKind,
};

fn code(value: &str, meaning: &str) -> DicomCode {
    DicomCode::new(value, "99FRAMES", meaning).unwrap()
}

fn add_specimen_metadata(object: &mut dicom_object::DefaultDicomObject) {
    let sequence = |tag, items| {
        DataElement::new(
            tag,
            VR::SQ,
            Value::from(DataSetSequence::new(items, Length::UNDEFINED)),
        )
    };
    let mut container_type = InMemDicomObject::new_empty();
    container_type.put(DataElement::new(tags::CODE_VALUE, VR::SH, "433466003"));
    container_type.put(DataElement::new(
        tags::CODING_SCHEME_DESIGNATOR,
        VR::SH,
        "SCT",
    ));
    container_type.put(DataElement::new(
        tags::CODE_MEANING,
        VR::LO,
        "Microscope slide",
    ));
    let mut specimen = InMemDicomObject::new_empty();
    specimen.put(DataElement::new(
        tags::SPECIMEN_IDENTIFIER,
        VR::LO,
        "SLIDE-1",
    ));
    specimen.put(DataElement::new(
        tags::SPECIMEN_UID,
        VR::UI,
        "2.25.100000000000000000000000000000006",
    ));
    specimen.put(sequence(
        tags::ISSUER_OF_THE_SPECIMEN_IDENTIFIER_SEQUENCE,
        Vec::new(),
    ));
    specimen.put(sequence(tags::SPECIMEN_PREPARATION_SEQUENCE, Vec::new()));

    object.put(DataElement::new(
        tags::CONTAINER_IDENTIFIER,
        VR::LO,
        "SLIDE-1",
    ));
    object.put(sequence(
        tags::ISSUER_OF_THE_CONTAINER_IDENTIFIER_SEQUENCE,
        Vec::new(),
    ));
    object.put(sequence(
        tags::CONTAINER_TYPE_CODE_SEQUENCE,
        vec![container_type],
    ));
    object.put(sequence(
        tags::SPECIMEN_DESCRIPTION_SEQUENCE,
        vec![specimen],
    ));
}

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

#[test]
fn sidecar_discovery_stops_before_ann_coordinate_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.dcm");
    let sidecar = dir.path().join("annotations.dcm");
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
    .write_ann(&sidecar)
    .unwrap();
    assert_eq!(discover_sidecars(&context).unwrap().len(), 1);
    let mut bytes = std::fs::read(&sidecar).unwrap();
    let group_header = [0x6A, 0x00, 0x02, 0x00, b'S', b'Q', 0, 0];
    let group_offset = bytes
        .windows(group_header.len())
        .position(|candidate| candidate == group_header)
        .expect("ANN should contain Annotation Group Sequence");
    bytes.truncate(group_offset + 12);
    std::fs::write(&sidecar, bytes).unwrap();

    let discovered = discover_sidecars(&context).unwrap();

    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].kind(), SidecarKind::Annotation);
    assert_eq!(discovered[0].path(), sidecar);
    assert!(!discovered[0].sop_instance_uid().is_empty());
    assert!(discovered[0].series_instance_uid().is_some());
    assert!(AnnotationDocument::read_ann(discovered[0].path(), &context).is_err());
}

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
    assert!(imported.vectorized_annotation_groups().is_err());
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

#[test]
#[ignore = "requires optional dciodvfy, dcmdump, and dcm2json command-line tools"]
fn exported_ann_and_seg_pass_external_dicom_validation() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.dcm");
    let ann_path = dir.path().join("annotations.dcm");
    let revised_ann_path = dir.path().join("annotations-revised.dcm");
    let frame_input_path = dir.path().join("frame-input.dcm");
    let frame_ann_path = dir.path().join("frame-annotations.dcm");
    let three_d_input_path = dir.path().join("3d-input.dcm");
    let three_d_ann_path = dir.path().join("3d-annotations.dcm");
    let seg_path = dir.path().join("segmentation.dcm");
    write_source_wsi(&source, 16, 12, 4, 4);
    let mut source_object = dicom_object::open_file(&source).unwrap();
    add_specimen_metadata(&mut source_object);
    source_object.write_to_file(&source).unwrap();
    let context = DicomAnnotationContext::from_source(&source).unwrap();
    let category = code("MORPH", "Morphologically abnormal structure");
    let property = code("VIABLE_TUMOR", "Viable tumor");
    let polygon = vec![
        Point2::new(0.0, 0.0),
        Point2::new(8.0, 0.0),
        Point2::new(8.0, 8.0),
        Point2::new(0.0, 8.0),
    ];
    let annotations = AnnotationDocument::new(
        context.clone(),
        vec![AnnotationGroup::polygons(
            "Viable tumor",
            category.clone(),
            property.clone(),
            [48_000, 40_000, 20_000],
            vec![polygon.clone()],
        )
        .unwrap()],
    )
    .unwrap();
    annotations.write_ann(&ann_path).unwrap();
    annotations.revised().write_ann(&revised_ann_path).unwrap();
    write_native_ann(
        &frame_input_path,
        "2D",
        Some("FRAME"),
        Some(2),
        &[(
            AnnotationGraphicType::Rectangle,
            vec![0.0, 0.0, 3.0, 0.0, 3.0, 2.0, 0.0, 2.0],
            vec![],
            None,
        )],
    );
    AnnotationDocument::read_ann(&frame_input_path, &context)
        .unwrap()
        .revised()
        .write_ann(&frame_ann_path)
        .unwrap();
    write_native_ann(
        &three_d_input_path,
        "3D",
        None,
        None,
        &[(
            AnnotationGraphicType::Point,
            vec![0.001, 0.002],
            vec![],
            Some(vec![0.0]),
        )],
    );
    AnnotationDocument::read_ann(&three_d_input_path, &context)
        .unwrap()
        .revised()
        .write_ann(&three_d_ann_path)
        .unwrap();
    SegmentationDocument::binary(
        context,
        vec![SegmentationSegment::new(
            "Viable tumor",
            category,
            property,
            [48_000, 40_000, 20_000],
            vec![polygon],
            Vec::new(),
        )
        .unwrap()],
    )
    .unwrap()
    .write_seg(&seg_path)
    .unwrap();

    for path in [
        ann_path,
        revised_ann_path,
        frame_ann_path,
        three_d_ann_path,
        seg_path,
    ] {
        for command in ["dciodvfy", "dcmdump", "dcm2json"] {
            let output = std::process::Command::new(command)
                .arg(&path)
                .output()
                .unwrap_or_else(|_| panic!("{command} should be installed for this ignored test"));
            assert!(
                output.status.success(),
                "{command} failed for {}:\n{}{}",
                path.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if command == "dcm2json" {
                assert_eq!(
                    output
                        .stdout
                        .iter()
                        .copied()
                        .find(|byte| !byte.is_ascii_whitespace()),
                    Some(b'{'),
                    "dcm2json output should begin with a JSON object"
                );
            }
        }
    }
}

type NativeAnnGroup = (AnnotationGraphicType, Vec<f64>, Vec<u32>, Option<Vec<f64>>);

fn write_native_ann(
    path: &Path,
    coordinate_type: &str,
    pixel_origin: Option<&str>,
    referenced_frame: Option<u32>,
    groups: &[NativeAnnGroup],
) {
    const SOP_UID: &str = "1.2.826.0.1.3680043.10.777.301";
    const SERIES_UID: &str = "1.2.826.0.1.3680043.10.777.302";
    const STUDY_UID: &str = "1.2.826.0.1.3680043.10.777.103";
    const FOR_UID: &str = "1.2.826.0.1.3680043.10.777.104";
    const SOURCE_SOP_UID: &str = "1.2.826.0.1.3680043.10.777.101";
    let sequence = |tag, items| {
        DataElement::new(
            tag,
            VR::SQ,
            Value::from(DataSetSequence::new(items, Length::UNDEFINED)),
        )
    };
    let code_item = |value: &'static str, meaning: &'static str| {
        let mut item = InMemDicomObject::new_empty();
        item.put(DataElement::new(tags::CODE_VALUE, VR::SH, value));
        item.put(DataElement::new(
            tags::CODING_SCHEME_DESIGNATOR,
            VR::SH,
            "99FRAMES",
        ));
        item.put(DataElement::new(tags::CODE_MEANING, VR::LO, meaning));
        item
    };
    let group_items = groups
        .iter()
        .enumerate()
        .map(|(index, (graphic_type, coordinates, indices, common_z))| {
            let dimensions = if coordinate_type == "3D" && common_z.is_none() {
                3
            } else {
                2
            };
            let annotation_count = match graphic_type {
                AnnotationGraphicType::Point => coordinates.len() / dimensions,
                AnnotationGraphicType::Polygon | AnnotationGraphicType::Polyline => indices.len(),
                AnnotationGraphicType::Ellipse | AnnotationGraphicType::Rectangle => {
                    coordinates.len() / (dimensions * 4)
                }
            };
            let mut item = InMemDicomObject::new_empty();
            item.put(DataElement::new(
                tags::ANNOTATION_GROUP_NUMBER,
                VR::US,
                PrimitiveValue::from(u16::try_from(index + 1).unwrap()),
            ));
            item.put(DataElement::new(
                tags::ANNOTATION_GROUP_UID,
                VR::UI,
                format!("2.25.{}", 400 + index),
            ));
            item.put(DataElement::new(
                tags::ANNOTATION_GROUP_LABEL,
                VR::LO,
                format!("Group {index}"),
            ));
            item.put(DataElement::new(
                tags::ANNOTATION_GROUP_GENERATION_TYPE,
                VR::CS,
                "MANUAL",
            ));
            item.put(sequence(
                tags::ANNOTATION_PROPERTY_CATEGORY_CODE_SEQUENCE,
                vec![code_item("MORPH", "Morphology")],
            ));
            item.put(sequence(
                tags::ANNOTATION_PROPERTY_TYPE_CODE_SEQUENCE,
                vec![code_item("TUMOR", "Tumor")],
            ));
            item.put(DataElement::new(
                tags::NUMBER_OF_ANNOTATIONS,
                VR::UL,
                PrimitiveValue::from(u32::try_from(annotation_count).unwrap()),
            ));
            item.put(DataElement::new(
                tags::GRAPHIC_TYPE,
                VR::CS,
                graphic_type.dicom_value(),
            ));
            item.put(DataElement::new(
                tags::ANNOTATION_APPLIES_TO_ALL_OPTICAL_PATHS,
                VR::CS,
                "YES",
            ));
            if coordinate_type == "3D" {
                item.put(DataElement::new(
                    tags::ANNOTATION_APPLIES_TO_ALL_Z_PLANES,
                    VR::CS,
                    "NO",
                ));
            }
            if let Some(common_z) = common_z {
                item.put(DataElement::new(
                    tags::COMMON_Z_COORDINATE_VALUE,
                    VR::FD,
                    PrimitiveValue::F64(common_z.clone().into()),
                ));
            }
            item.put(DataElement::new(
                tags::DOUBLE_POINT_COORDINATES_DATA,
                VR::OD,
                PrimitiveValue::F64(coordinates.clone().into()),
            ));
            if !indices.is_empty() {
                item.put(DataElement::new(
                    tags::LONG_PRIMITIVE_POINT_INDEX_LIST,
                    VR::OL,
                    PrimitiveValue::U32(indices.clone().into()),
                ));
            }
            item
        })
        .collect();
    let mut object = InMemDicomObject::new_empty();
    for element in [
        DataElement::new(
            tags::SOP_CLASS_UID,
            VR::UI,
            uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE,
        ),
        DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, SOP_UID),
        DataElement::new(tags::STUDY_INSTANCE_UID, VR::UI, STUDY_UID),
        DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, SERIES_UID),
        DataElement::new(tags::SERIES_NUMBER, VR::IS, "1"),
        DataElement::new(tags::MANUFACTURER, VR::LO, "Synthetic Manufacturer"),
        DataElement::new(tags::MANUFACTURER_MODEL_NAME, VR::LO, "Synthetic Model"),
        DataElement::new(tags::DEVICE_SERIAL_NUMBER, VR::LO, "SYNTHETIC"),
        DataElement::new(tags::SOFTWARE_VERSIONS, VR::LO, "1"),
        DataElement::new(tags::CONTENT_LABEL, VR::CS, "TEST_ANNOTATION"),
        DataElement::new(tags::ANNOTATION_COORDINATE_TYPE, VR::CS, coordinate_type),
    ] {
        object.put(element);
    }
    if coordinate_type == "2D" {
        object.put(DataElement::new(
            tags::PIXEL_ORIGIN_INTERPRETATION,
            VR::CS,
            pixel_origin.unwrap(),
        ));
        let mut reference = InMemDicomObject::new_empty();
        reference.put(DataElement::new(
            tags::REFERENCED_SOP_CLASS_UID,
            VR::UI,
            uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE,
        ));
        reference.put(DataElement::new(
            tags::REFERENCED_SOP_INSTANCE_UID,
            VR::UI,
            SOURCE_SOP_UID,
        ));
        if let Some(frame) = referenced_frame {
            reference.put(DataElement::new(
                tags::REFERENCED_FRAME_NUMBER,
                VR::IS,
                frame.to_string(),
            ));
        }
        object.put(sequence(tags::REFERENCED_IMAGE_SEQUENCE, vec![reference]));
    } else {
        object.put(DataElement::new(
            tags::FRAME_OF_REFERENCE_UID,
            VR::UI,
            FOR_UID,
        ));
    }
    object.put(sequence(tags::ANNOTATION_GROUP_SEQUENCE, group_items));
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE)
                .media_storage_sop_instance_uid(SOP_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap()
        .write_to_file(path)
        .unwrap();
}

fn write_fractional_seg(path: &Path, pixels: [u8; 16]) {
    const SOP_UID: &str = "1.2.826.0.1.3680043.10.777.201";
    const SERIES_UID: &str = "1.2.826.0.1.3680043.10.777.202";
    const STUDY_UID: &str = "1.2.826.0.1.3680043.10.777.103";
    const FOR_UID: &str = "1.2.826.0.1.3680043.10.777.104";
    const SOURCE_SOP_UID: &str = "1.2.826.0.1.3680043.10.777.101";
    let code_item = |value: &'static str, meaning: &'static str| {
        let mut item = InMemDicomObject::new_empty();
        item.put(DataElement::new(tags::CODE_VALUE, VR::SH, value));
        item.put(DataElement::new(
            tags::CODING_SCHEME_DESIGNATOR,
            VR::SH,
            "SCT",
        ));
        item.put(DataElement::new(tags::CODE_MEANING, VR::LO, meaning));
        item
    };
    let sequence = |tag, items| {
        DataElement::new(
            tag,
            VR::SQ,
            Value::from(DataSetSequence::new(items, Length::UNDEFINED)),
        )
    };
    let mut segment = InMemDicomObject::new_empty();
    segment.put(DataElement::new(
        tags::SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(1_u16),
    ));
    segment.put(DataElement::new(tags::SEGMENT_LABEL, VR::LO, "Probability"));
    segment.put(sequence(
        tags::SEGMENTED_PROPERTY_CATEGORY_CODE_SEQUENCE,
        vec![code_item("49755003", "Morphologically abnormal structure")],
    ));
    segment.put(sequence(
        tags::SEGMENTED_PROPERTY_TYPE_CODE_SEQUENCE,
        vec![code_item("108369006", "Neoplasm")],
    ));
    segment.put(DataElement::new(
        tags::RECOMMENDED_DISPLAY_CIE_LAB_VALUE,
        VR::US,
        PrimitiveValue::from([48_000_u16, 32_768, 32_768]),
    ));
    let mut segment_identification = InMemDicomObject::new_empty();
    segment_identification.put(DataElement::new(
        tags::REFERENCED_SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(1_u16),
    ));
    let mut plane = InMemDicomObject::new_empty();
    plane.put(DataElement::new(
        tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(1_i32),
    ));
    plane.put(DataElement::new(
        tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(1_i32),
    ));
    let mut frame = InMemDicomObject::new_empty();
    frame.put(sequence(
        tags::SEGMENT_IDENTIFICATION_SEQUENCE,
        vec![segment_identification],
    ));
    frame.put(sequence(tags::PLANE_POSITION_SLIDE_SEQUENCE, vec![plane]));
    let mut source_reference = InMemDicomObject::new_empty();
    source_reference.put(DataElement::new(
        tags::REFERENCED_SOP_CLASS_UID,
        VR::UI,
        uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE,
    ));
    source_reference.put(DataElement::new(
        tags::REFERENCED_SOP_INSTANCE_UID,
        VR::UI,
        SOURCE_SOP_UID,
    ));
    let mut derivation = InMemDicomObject::new_empty();
    derivation.put(sequence(
        tags::SOURCE_IMAGE_SEQUENCE,
        vec![source_reference],
    ));
    let mut shared = InMemDicomObject::new_empty();
    shared.put(sequence(tags::DERIVATION_IMAGE_SEQUENCE, vec![derivation]));
    let mut object = InMemDicomObject::new_empty();
    for element in [
        DataElement::new(tags::SOP_CLASS_UID, VR::UI, uids::SEGMENTATION_STORAGE),
        DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, SOP_UID),
        DataElement::new(tags::STUDY_INSTANCE_UID, VR::UI, STUDY_UID),
        DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, SERIES_UID),
        DataElement::new(tags::FRAME_OF_REFERENCE_UID, VR::UI, FOR_UID),
        DataElement::new(tags::CONTENT_LABEL, VR::CS, "TEST_SEGMENTATION"),
        DataElement::new(tags::SEGMENTATION_TYPE, VR::CS, "FRACTIONAL"),
        DataElement::new(tags::SEGMENTATION_FRACTIONAL_TYPE, VR::CS, "PROBABILITY"),
        DataElement::new(
            tags::MAXIMUM_FRACTIONAL_VALUE,
            VR::US,
            PrimitiveValue::from(255_u16),
        ),
        DataElement::new(tags::ROWS, VR::US, PrimitiveValue::from(4_u16)),
        DataElement::new(tags::COLUMNS, VR::US, PrimitiveValue::from(4_u16)),
        DataElement::new(tags::NUMBER_OF_FRAMES, VR::IS, "1"),
        DataElement::new(tags::BITS_ALLOCATED, VR::US, PrimitiveValue::from(8_u16)),
        DataElement::new(tags::BITS_STORED, VR::US, PrimitiveValue::from(8_u16)),
        DataElement::new(tags::HIGH_BIT, VR::US, PrimitiveValue::from(7_u16)),
    ] {
        object.put(element);
    }
    object.put(sequence(tags::SEGMENT_SEQUENCE, vec![segment]));
    object.put(sequence(
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![shared],
    ));
    object.put(sequence(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![frame],
    ));
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        PrimitiveValue::from(pixels.to_vec()),
    ));
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::SEGMENTATION_STORAGE)
                .media_storage_sop_instance_uid(SOP_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap()
        .write_to_file(path)
        .unwrap();
}

pub(crate) fn write_source_wsi(
    path: &Path,
    width: u32,
    height: u32,
    tile_width: u16,
    tile_height: u16,
) {
    write_source_wsi_with_spacing(path, width, height, tile_width, tile_height, 0.00025);
}

pub(crate) fn write_source_wsi_with_spacing(
    path: &Path,
    width: u32,
    height: u32,
    tile_width: u16,
    tile_height: u16,
    spacing: f64,
) {
    const SOP_UID: &str = "1.2.826.0.1.3680043.10.777.101";
    const SERIES_UID: &str = "1.2.826.0.1.3680043.10.777.102";
    const STUDY_UID: &str = "1.2.826.0.1.3680043.10.777.103";
    const FOR_UID: &str = "1.2.826.0.1.3680043.10.777.104";
    let mut origin = InMemDicomObject::new_empty();
    origin.put(DataElement::new(
        tags::X_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        "0",
    ));
    origin.put(DataElement::new(
        tags::Y_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        "0",
    ));
    let frame_count = width
        .div_ceil(u32::from(tile_width))
        .saturating_mul(height.div_ceil(u32::from(tile_height)));
    let mut object = InMemDicomObject::new_empty();
    for element in [
        DataElement::new(
            tags::SOP_CLASS_UID,
            VR::UI,
            uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE,
        ),
        DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, SOP_UID),
        DataElement::new(tags::STUDY_INSTANCE_UID, VR::UI, STUDY_UID),
        DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, SERIES_UID),
        DataElement::new(tags::FRAME_OF_REFERENCE_UID, VR::UI, FOR_UID),
        DataElement::new(tags::PATIENT_NAME, VR::PN, "Research^Slide"),
        DataElement::new(tags::PATIENT_ID, VR::LO, "R-1"),
        DataElement::new(tags::STUDY_DATE, VR::DA, "20260804"),
        DataElement::new(tags::STUDY_TIME, VR::TM, "120000"),
        DataElement::new(tags::STUDY_ID, VR::SH, "STUDY-1"),
        DataElement::new(tags::ACCESSION_NUMBER, VR::SH, ""),
        DataElement::new(tags::MANUFACTURER, VR::LO, "Frames"),
        DataElement::new(tags::MANUFACTURER_MODEL_NAME, VR::LO, "Synthetic WSI"),
        DataElement::new(tags::DEVICE_SERIAL_NUMBER, VR::LO, "TEST"),
        DataElement::new(tags::SOFTWARE_VERSIONS, VR::LO, "0.1"),
        DataElement::new(tags::DIMENSION_ORGANIZATION_TYPE, VR::CS, "TILED_FULL"),
        DataElement::new(tags::NUMBER_OF_FRAMES, VR::IS, frame_count.to_string()),
        DataElement::new(
            tags::NUMBER_OF_OPTICAL_PATHS,
            VR::UL,
            PrimitiveValue::from(1_u32),
        ),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_FOCAL_PLANES,
            VR::UL,
            PrimitiveValue::from(1_u32),
        ),
        DataElement::new(tags::ROWS, VR::US, PrimitiveValue::from(tile_height)),
        DataElement::new(tags::COLUMNS, VR::US, PrimitiveValue::from(tile_width)),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_ROWS,
            VR::UL,
            PrimitiveValue::from(height),
        ),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_COLUMNS,
            VR::UL,
            PrimitiveValue::from(width),
        ),
        DataElement::new(tags::IMAGE_ORIENTATION_SLIDE, VR::DS, "1\\0\\0\\0\\1\\0"),
        DataElement::new(tags::PIXEL_SPACING, VR::DS, format!("{spacing}\\{spacing}")),
        DataElement::new(tags::SLICE_THICKNESS, VR::DS, "0.001"),
    ] {
        object.put(element);
    }
    object.put(DataElement::new(
        tags::TOTAL_PIXEL_MATRIX_ORIGIN_SEQUENCE,
        VR::SQ,
        Value::from(DataSetSequence::new(vec![origin], Length::UNDEFINED)),
    ));
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        PrimitiveValue::from(vec![
            0_u8;
            usize::from(tile_width) * usize::from(tile_height) * 3
        ]),
    ));
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE)
                .media_storage_sop_instance_uid(SOP_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap()
        .write_to_file(path)
        .unwrap();
}

fn write_sparse_source_wsi(path: &Path) {
    write_source_wsi(path, 8, 8, 4, 4);
    let mut object = dicom_object::open_file(path).unwrap();
    object.put(DataElement::new(
        tags::DIMENSION_ORGANIZATION_TYPE,
        VR::CS,
        "TILED_SPARSE",
    ));
    object.put(DataElement::new(tags::NUMBER_OF_FRAMES, VR::IS, "2"));
    let frames: Vec<_> = [(1_i32, 1_i32), (5, 5)]
        .into_iter()
        .map(|(column, row)| {
            let mut plane = InMemDicomObject::new_empty();
            plane.put(DataElement::new(
                tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
                VR::SL,
                PrimitiveValue::from(column),
            ));
            plane.put(DataElement::new(
                tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
                VR::SL,
                PrimitiveValue::from(row),
            ));
            let mut frame = InMemDicomObject::new_empty();
            frame.put(DataElement::new(
                tags::PLANE_POSITION_SLIDE_SEQUENCE,
                VR::SQ,
                Value::from(DataSetSequence::new(vec![plane], Length::UNDEFINED)),
            ));
            frame
        })
        .collect();
    object.put(DataElement::new(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        VR::SQ,
        Value::from(DataSetSequence::new(frames, Length::UNDEFINED)),
    ));
    object.write_to_file(path).unwrap();
}
