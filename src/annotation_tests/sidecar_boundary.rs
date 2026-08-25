use super::*;

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
