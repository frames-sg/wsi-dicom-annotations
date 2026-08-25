use super::*;

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
