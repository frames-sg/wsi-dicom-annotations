use dicom_core::{DataElement, VR};
use dicom_dictionary_std::tags;

use crate::test_support::{write_source_wsi, write_source_wsi_with_spacing};
use crate::{
    AnnotationGraphicType, CoordinateGraphic, DerivedObjectProducer, DicomAnnotationContext,
    PathologyAnnotationSet, PathologyCoordinateSpace, PathologyDicomDocuments,
    PathologyDicomTarget, PathologyGeometryKind, PathologyPreviewGeometry,
    StructuredReportDocument, StructuredReportReferenceKind,
};

const MAPPING: &str = r#"
{
  "schema_version": 1,
  "labels": {
    "viable_tumor": {
      "category": {
        "code_value": "MORPH",
        "coding_scheme_designator": "99WSI",
        "code_meaning": "Morphologically abnormal structure"
      },
      "property_type": {
        "code_value": "TUMOR",
        "coding_scheme_designator": "99WSI",
        "code_meaning": "Tumor"
      },
      "generation_type": "MANUAL",
      "property_type_modifiers": [],
      "anatomic_regions": [],
      "recommended_display_cielab": [39321, 38036, 35466],
      "segment_label": "Viable tumor"
    }
  },
  "measurements": {},
  "qualitative_evaluations": {}
}
"#;

fn source_fixture() -> (tempfile::TempDir, DicomAnnotationContext) {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    write_source_wsi(&source_path, 16, 12, 4, 4);
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    (directory, source)
}

fn parse_fixture(
    geojson: &[u8],
    mapping: &[u8],
    source: &DicomAnnotationContext,
    allow_unmapped_properties: bool,
) -> crate::Result<PathologyAnnotationSet> {
    PathologyAnnotationSet::from_json(
        geojson,
        mapping,
        source,
        source,
        PathologyCoordinateSpace::Level0Pixels,
        allow_unmapped_properties,
    )
}

#[test]
fn pathology_preview_preserves_mixed_geometry_holes_identity_and_display_semantics() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type":"FeatureCollection",
      "features":[
        {
          "type":"Feature",
          "id":"2.25.701",
          "geometry":{"type":"MultiPoint","coordinates":[[1,1],[2,2]]},
          "properties":{"name":"Cells","classification":{"name":"viable_tumor"}}
        },
        {
          "type":"Feature",
          "id":"2.25.702",
          "geometry":{"type":"MultiLineString","coordinates":[[[3,1],[5,1]],[[3,2],[5,3]]]},
          "properties":{"name":"Boundary","classification":{"name":"viable_tumor"}}
        },
        {
          "type":"Feature",
          "id":"2.25.703",
          "geometry":{"type":"Polygon","coordinates":[
            [[6,1],[14,1],[14,10],[6,10],[6,1]],
            [[8,3],[8,6],[11,6],[11,3],[8,3]]
          ]},
          "properties":{"name":"Region","classification":{"name":"viable_tumor"}}
        }
      ]
    }
    "#;
    let annotations = parse_fixture(geojson, MAPPING.as_bytes(), &source, false).unwrap();

    let preview = annotations.into_preview();

    assert_eq!(preview.features().len(), 3);
    assert_eq!(preview.features()[0].tracking_id(), "2.25.701");
    assert_eq!(preview.features()[0].label(), "Cells");
    assert_eq!(
        preview.features()[0].recommended_display_cielab(),
        [39321, 38036, 35466]
    );
    let PathologyPreviewGeometry::Points(points) = preview.features()[0].geometry() else {
        panic!("multipoint preview should remain points");
    };
    assert_eq!(points.len(), 2);
    let PathologyPreviewGeometry::Lines(lines) = preview.features()[1].geometry() else {
        panic!("multiline preview should remain lines");
    };
    assert_eq!(lines.len(), 2);
    let PathologyPreviewGeometry::Polygons(polygons) = preview.features()[2].geometry() else {
        panic!("polygon preview should remain polygons");
    };
    assert_eq!(polygons.len(), 1);
    assert_eq!(polygons[0].holes().len(), 1);
}

#[test]
fn viewer_geojson_profile_normalizes_alias_closure_winding_and_uuid() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type": "FeatureCollection",
      "features": [{
        "type": "Feature",
        "id": "550e8400-e29b-41d4-a716-446655440000",
        "geometry": {
          "type": "Polygon",
          "coordinates": [[[1,1],[1,5],[6,5],[6,1],[1,1]]]
        },
        "properties": {
          "objectType": "annotation",
          "name": "Tumor 1",
          "coordinate_space": "level-0_pixels",
          "classification": {"name": "viable_tumor"}
        }
      }]
    }
    "#;

    let annotations = parse_fixture(geojson, MAPPING.as_bytes(), &source, false).unwrap();

    assert_eq!(annotations.feature_count(), 1);
    assert_eq!(
        annotations.feature_geometry_kind(0),
        Some(PathologyGeometryKind::Polygon)
    );
    assert_eq!(
        annotations.feature_tracking_id(0),
        Some("550e8400-e29b-41d4-a716-446655440000")
    );
    assert!(annotations
        .feature_tracking_uid(0)
        .is_some_and(|uid| uid.starts_with("2.25.")));
    let diagnostic_codes: Vec<_> = annotations
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code())
        .collect();
    assert_eq!(
        diagnostic_codes,
        [
            "GEOJSON_COORDINATE_SPACE_ALIAS",
            "GEOJSON_RING_CLOSURE_REMOVED",
            "GEOJSON_WINDING_NORMALIZED"
        ]
    );
}

#[test]
fn geojson_profile_rejects_conflicting_aliases_and_unmapped_properties() {
    let (_directory, source) = source_fixture();
    let conflicting = br#"
    {
      "type": "FeatureCollection",
      "features": [{
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [1,2]},
        "properties": {
          "objectType": "annotation",
          "object_type": "detection",
          "classification": {"name": "viable_tumor"}
        }
      }]
    }
    "#;
    let error = parse_fixture(conflicting, MAPPING.as_bytes(), &source, false).unwrap_err();
    assert!(error
        .to_string()
        .contains("objectType and object_type disagree"));

    let unknown_property = br#"
    {
      "type": "FeatureCollection",
      "features": [{
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [1,2]},
        "properties": {
          "classification": {"name": "viable_tumor"},
          "confidence": 0.9
        }
      }]
    }
    "#;
    let error = parse_fixture(unknown_property, MAPPING.as_bytes(), &source, false).unwrap_err();
    assert!(error.to_string().contains("unmapped property confidence"));
}

#[test]
fn geojson_profile_rejects_duplicate_semantic_property_sources() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type": "FeatureCollection",
      "features": [{
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [1,2]},
        "properties": {
          "classification": {"name": "viable_tumor"},
          "metadata": {"grade": "high"},
          "grade": "low"
        }
      }]
    }
    "#;

    let error = parse_fixture(geojson, MAPPING.as_bytes(), &source, true).unwrap_err();

    assert!(error
        .to_string()
        .contains("property grade is declared more than once"));
}

#[test]
fn geojson_profile_rejects_duplicate_json_object_keys() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type": "FeatureCollection",
      "features": [{
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [1,2]},
        "properties": {
          "classification": {"name": "viable_tumor"},
          "metadata": {"grade": "high", "grade": "low"}
        }
      }]
    }
    "#;

    let error = parse_fixture(geojson, MAPPING.as_bytes(), &source, true).unwrap_err();

    assert!(error
        .to_string()
        .contains("duplicate JSON object key grade"));
}

#[test]
fn geojson_profile_rejects_invalid_polygon_hole_topology() {
    let (_directory, source) = source_fixture();
    let invalid_holes = [
        "[[[1,1],[10,1],[10,10],[1,10],[1,1]],[[11,2],[13,2],[13,4],[11,4],[11,2]]]",
        "[[[1,1],[10,1],[10,10],[1,10],[1,1]],[[8,3],[12,3],[12,5],[8,5],[8,3]]]",
        "[[[1,1],[12,1],[12,11],[1,11],[1,1]],[[2,2],[7,2],[7,7],[2,7],[2,2]],[[5,5],[10,5],[10,9],[5,9],[5,5]]]",
    ];

    for coordinates in invalid_holes {
        let geojson = format!(
            r#"{{
              "type":"FeatureCollection",
              "features":[{{
                "type":"Feature",
                "geometry":{{"type":"Polygon","coordinates":{coordinates}}},
                "properties":{{"classification":{{"name":"viable_tumor"}}}}
              }}]
            }}"#
        );
        let error =
            parse_fixture(geojson.as_bytes(), MAPPING.as_bytes(), &source, false).unwrap_err();

        assert!(error.to_string().contains("invalid polygon hole topology"));
    }
}

#[test]
fn geojson_profile_rejects_unclosed_polygon_rings() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type":"FeatureCollection",
      "features":[{
        "type":"Feature",
        "geometry":{"type":"Polygon","coordinates":[[[1,1],[5,1],[5,5],[1,5]]]},
        "properties":{"classification":{"name":"viable_tumor"}}
      }]
    }
    "#;

    let error = parse_fixture(geojson, MAPPING.as_bytes(), &source, false).unwrap_err();

    assert!(error.to_string().contains("must repeat its first position"));
}

#[test]
fn geojson_profile_rejects_overlapping_multipolygon_components() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type":"FeatureCollection",
      "features":[{
        "type":"Feature",
        "geometry":{"type":"MultiPolygon","coordinates":[
          [[[1,1],[8,1],[8,8],[1,8],[1,1]]],
          [[[6,6],[12,6],[12,10],[6,10],[6,6]]]
        ]},
        "properties":{"classification":{"name":"viable_tumor"}}
      }]
    }
    "#;

    let error = parse_fixture(geojson, MAPPING.as_bytes(), &source, false).unwrap_err();

    assert!(error
        .to_string()
        .contains("invalid MultiPolygon component topology"));
}

#[test]
fn ann_conversion_preserves_feature_groups_and_all_supported_graphics() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type": "FeatureCollection",
      "features": [
        {"type":"Feature","id":"2.25.1","geometry":{"type":"Point","coordinates":[1,1]},"properties":{"classification":{"name":"viable_tumor"}}},
        {"type":"Feature","id":"2.25.2","geometry":{"type":"MultiPoint","coordinates":[[2,1],[3,1]]},"properties":{"classification":{"name":"viable_tumor"}}},
        {"type":"Feature","id":"2.25.3","geometry":{"type":"LineString","coordinates":[[1,2],[3,2],[4,3]]},"properties":{"classification":{"name":"viable_tumor"}}},
        {"type":"Feature","id":"2.25.4","geometry":{"type":"MultiLineString","coordinates":[[[1,3],[2,4]],[[3,4],[4,5]]]},"properties":{"classification":{"name":"viable_tumor"}}},
        {"type":"Feature","id":"2.25.5","geometry":{"type":"Polygon","coordinates":[[[5,1],[9,1],[9,4],[5,4],[5,1]]]},"properties":{"classification":{"name":"viable_tumor"}}},
        {"type":"Feature","id":"2.25.6","geometry":{"type":"MultiPolygon","coordinates":[[[[5,5],[7,5],[7,7],[5,7],[5,5]]],[[[9,5],[11,5],[11,7],[9,7],[9,5]]]]},"properties":{"classification":{"name":"viable_tumor"}}}
      ]
    }
    "#;
    let annotations = parse_fixture(geojson, MAPPING.as_bytes(), &source, false).unwrap();

    let ann = annotations.to_ann().unwrap();

    assert_eq!(ann.groups().len(), 6);
    assert_eq!(
        ann.groups()
            .iter()
            .map(|group| group.geometry().graphic_type())
            .collect::<Vec<_>>(),
        [
            AnnotationGraphicType::Point,
            AnnotationGraphicType::Point,
            AnnotationGraphicType::Polyline,
            AnnotationGraphicType::Polyline,
            AnnotationGraphicType::Polygon,
            AnnotationGraphicType::Polygon,
        ]
    );
    assert_eq!(
        ann.groups()
            .iter()
            .map(|group| group.uid())
            .collect::<Vec<_>>(),
        ["2.25.1", "2.25.2", "2.25.3", "2.25.4", "2.25.5", "2.25.6"]
    );
    assert_eq!(ann.groups()[3].annotation_count(), 2);
    assert_eq!(ann.groups()[5].annotation_count(), 2);
}

#[test]
fn ann_conversion_rejects_polygon_holes_before_output() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type": "FeatureCollection",
      "features": [{
        "type":"Feature",
        "id":"2.25.9",
        "geometry":{"type":"Polygon","coordinates":[
          [[1,1],[10,1],[10,10],[1,10],[1,1]],
          [[3,3],[3,6],[6,6],[6,3],[3,3]]
        ]},
        "properties":{"classification":{"name":"viable_tumor"}}
      }]
    }
    "#;
    let annotations = parse_fixture(geojson, MAPPING.as_bytes(), &source, false).unwrap();

    let error = annotations.to_ann().unwrap_err();

    assert!(error
        .to_string()
        .contains("ANN cannot represent polygon interior rings; select SEG"));
}

#[test]
fn ann_and_sr_compose_for_multipoint_feature_measurements() {
    let (_directory, source) = source_fixture();
    let mapping = mapping_with_sr();
    let geojson = br#"
    {
      "type":"FeatureCollection",
      "features":[{
        "type":"Feature",
        "id":"2.25.19",
        "geometry":{"type":"MultiPoint","coordinates":[[1,1],[2,2]]},
        "properties":{
          "classification":{"name":"viable_tumor"},
          "measurements":{"Area":2.0}
        }
      }]
    }
    "#;
    let annotations = parse_fixture(geojson, &mapping, &source, false).unwrap();

    assert!(annotations.to_ann().is_err());
    let ann = annotations.to_ann_with_companion_sr().unwrap();
    let sr = annotations.to_sr(None).unwrap();

    assert!(ann.groups()[0].measurements().is_empty());
    assert_eq!(sr.groups()[0].measurements().len(), 1);
    assert_eq!(
        sr.groups()[0].reference_kind(),
        StructuredReportReferenceKind::MeasurementCoordinates
    );
}

#[test]
fn seg_conversion_preserves_holes_components_overlap_and_tracking() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type": "FeatureCollection",
      "features": [
        {
          "type":"Feature",
          "id":"2.25.20",
          "geometry":{"type":"MultiPolygon","coordinates":[
            [[[1,1],[8,1],[8,8],[1,8],[1,1]],[[3,3],[3,6],[6,6],[6,3],[3,3]]],
            [[[10,1],[12,1],[12,3],[10,3],[10,1]]]
          ]},
          "properties":{"name":"First","classification":{"name":"viable_tumor"}}
        },
        {
          "type":"Feature",
          "id":"2.25.21",
          "geometry":{"type":"Polygon","coordinates":[[[6,6],[10,6],[10,10],[6,10],[6,6]]]},
          "properties":{"name":"Second","classification":{"name":"viable_tumor"}}
        }
      ]
    }
    "#;
    let annotations = parse_fixture(geojson, MAPPING.as_bytes(), &source, false).unwrap();

    let seg = annotations.to_seg(false).unwrap();
    let runs = seg.binary_runs().unwrap();

    assert_eq!(seg.segments().len(), 2);
    assert_eq!(seg.segments()[0].tracking_id(), Some("2.25.20"));
    assert_eq!(seg.segments()[0].tracking_uid(), Some("2.25.20"));
    assert_eq!(seg.segments()[0].outer_polygons().len(), 2);
    assert_eq!(seg.segments()[0].exclusion_polygons().len(), 1);
    assert!(mask_contains(&runs, 1, 2, 2));
    assert!(!mask_contains(&runs, 1, 4, 4));
    assert!(mask_contains(&runs, 1, 10, 2));
    assert!(mask_contains(&runs, 1, 7, 7));
    assert!(mask_contains(&runs, 2, 7, 7));
}

#[test]
fn seg_conversion_transforms_level_zero_geometry_to_a_lower_pyramid_source() {
    let directory = tempfile::tempdir().unwrap();
    let canonical_path = directory.path().join("level-zero.dcm");
    let source_path = directory.path().join("lower-level.dcm");
    write_source_wsi_with_spacing(&canonical_path, 16, 12, 4, 4, 0.00025);
    write_source_wsi_with_spacing(&source_path, 8, 6, 4, 3, 0.0005);
    let mut lower = dicom_object::open_file(&source_path).unwrap();
    lower.put(DataElement::new(
        tags::SOP_INSTANCE_UID,
        VR::UI,
        "1.2.826.0.1.3680043.10.777.201",
    ));
    lower.put(DataElement::new(
        tags::SERIES_INSTANCE_UID,
        VR::UI,
        "1.2.826.0.1.3680043.10.777.202",
    ));
    lower.write_to_file(&source_path).unwrap();
    let canonical = DicomAnnotationContext::from_source(&canonical_path).unwrap();
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let geojson = br#"
    {
      "type":"FeatureCollection",
      "features":[{
        "type":"Feature",
        "id":"2.25.23",
        "geometry":{"type":"Polygon","coordinates":[[[2,2],[8,2],[8,8],[2,8],[2,2]]]},
        "properties":{"classification":{"name":"viable_tumor"}}
      }]
    }
    "#;
    let annotations = PathologyAnnotationSet::from_json(
        geojson,
        MAPPING.as_bytes(),
        &source,
        &canonical,
        PathologyCoordinateSpace::Level0Pixels,
        false,
    )
    .unwrap();

    let runs = annotations.to_seg(false).unwrap().binary_runs().unwrap();

    assert!(mask_contains(&runs, 1, 1, 1));
    assert!(mask_contains(&runs, 1, 3, 3));
    assert!(!mask_contains(&runs, 1, 4, 3));
}

#[test]
fn multipolygon_component_can_fill_another_components_hole() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type": "FeatureCollection",
      "features": [{
        "type":"Feature",
        "id":"2.25.22",
        "geometry":{"type":"MultiPolygon","coordinates":[
          [[[1,1],[10,1],[10,10],[1,10],[1,1]],[[3,3],[3,7],[7,7],[7,3],[3,3]]],
          [[[4,4],[6,4],[6,6],[4,6],[4,4]]]
        ]},
        "properties":{"classification":{"name":"viable_tumor"}}
      }]
    }
    "#;
    let annotations = parse_fixture(geojson, MAPPING.as_bytes(), &source, false).unwrap();

    let runs = annotations.to_seg(false).unwrap().binary_runs().unwrap();

    assert!(!mask_contains(&runs, 1, 3, 3));
    assert!(mask_contains(&runs, 1, 4, 4));
}

#[test]
fn seg_conversion_rejects_non_area_geometry() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type": "FeatureCollection",
      "features": [{
        "type":"Feature",
        "id":"2.25.30",
        "geometry":{"type":"Point","coordinates":[1,1]},
        "properties":{"classification":{"name":"viable_tumor"}}
      }]
    }
    "#;
    let annotations = parse_fixture(geojson, MAPPING.as_bytes(), &source, false).unwrap();

    let error = annotations.to_seg(false).unwrap_err();

    assert!(error
        .to_string()
        .contains("SEG accepts only Polygon and MultiPolygon"));
}

fn mask_contains(
    runs: &[crate::BinaryMaskRun],
    segment_number: u16,
    column: u32,
    row: u32,
) -> bool {
    runs.iter().any(|run| {
        run.segment_number() == segment_number
            && run.row() == row
            && column >= run.column_start()
            && column < run.column_start() + run.length()
    })
}

#[test]
fn sr_direct_roi_roundtrips_tracking_measurements_evaluations_and_status() {
    let (directory, source) = source_fixture();
    let sr_path = directory.path().join("report.dcm");
    let mapping = mapping_with_sr();
    let geojson = br#"
    {
      "type":"FeatureCollection",
      "features":[{
        "type":"Feature",
        "id":"2.25.40",
        "geometry":{"type":"Polygon","coordinates":[[[1,1],[5,1],[5,5],[1,5],[1,1]]]},
        "properties":{
          "classification":{"name":"viable_tumor"},
          "measurements":{"Area":16.0},
          "metadata":{"grade":"high"}
        }
      }]
    }
    "#;
    let annotations = parse_fixture(geojson, &mapping, &source, false).unwrap();

    assert!(annotations.to_ann().is_err());
    assert_eq!(
        annotations
            .to_ann_with_companion_sr()
            .unwrap()
            .groups()
            .len(),
        1
    );

    let sr = annotations.to_sr(None).unwrap();
    sr.write_sr(&sr_path).unwrap();
    let raw = dicom_object::open_file(&sr_path).unwrap();
    for tag in [
        dicom_dictionary_std::tags::FRAME_OF_REFERENCE_UID,
        dicom_dictionary_std::tags::CONTAINER_IDENTIFIER,
        dicom_dictionary_std::tags::ISSUER_OF_THE_CONTAINER_IDENTIFIER_SEQUENCE,
        dicom_dictionary_std::tags::CONTAINER_TYPE_CODE_SEQUENCE,
        dicom_dictionary_std::tags::SPECIMEN_DESCRIPTION_SEQUENCE,
    ] {
        assert!(raw.get(tag).is_none(), "unexpected SR attribute {tag}");
    }
    let imported = StructuredReportDocument::read_sr(&sr_path, &source, None).unwrap();

    assert_eq!(imported.completion_flag(), "COMPLETE");
    assert_eq!(imported.verification_flag(), "UNVERIFIED");
    assert_eq!(imported.preliminary_flag(), "PRELIMINARY");
    assert_eq!(
        imported.report_title().meaning(),
        "Imaging Measurement Report"
    );
    assert_eq!(
        imported.procedures_reported()[0].meaning(),
        "Histopathology procedure"
    );
    assert_eq!(imported.groups().len(), 1);
    let group = &imported.groups()[0];
    assert_eq!(group.tracking_id(), "2.25.40");
    assert_eq!(group.tracking_uid(), "2.25.40");
    assert_eq!(
        group.reference_kind(),
        StructuredReportReferenceKind::Polygon
    );
    assert_eq!(group.measurements()[0].value(), 16.0);
    assert_eq!(group.measurements()[0].concept().value(), "AREA");
    assert_eq!(group.finding_category().value(), "MORPH");
    assert_eq!(group.finding_type().value(), "TUMOR");
    let coordinates = group
        .region_coordinates()
        .expect("direct ROI should retain its SCOORD3D reference");
    assert_eq!(coordinates.graphic(), CoordinateGraphic::Polygon);
    assert_eq!(coordinates.points().len(), 5);
    assert_eq!(coordinates.points().first(), coordinates.points().last());
    assert_eq!(
        coordinates.frame_of_reference_uid(),
        source.frame_of_reference_uid().unwrap()
    );
    assert_eq!(group.qualitative_evaluations()[0].value().value(), "HIGH");
}

#[test]
fn sr_uses_seg_reference_for_holes_with_matching_tracking() {
    let (_directory, source) = source_fixture();
    let mapping = mapping_with_sr();
    let geojson = br#"
    {
      "type":"FeatureCollection",
      "features":[{
        "type":"Feature",
        "id":"2.25.41",
        "geometry":{"type":"Polygon","coordinates":[
          [[1,1],[10,1],[10,10],[1,10],[1,1]],
          [[3,3],[3,6],[6,6],[6,3],[3,3]]
        ]},
        "properties":{"classification":{"name":"viable_tumor"}}
      }]
    }
    "#;
    let annotations = parse_fixture(geojson, &mapping, &source, false).unwrap();
    let seg = annotations.to_seg(true).unwrap();

    let sr = annotations.to_sr(Some(&seg)).unwrap();
    let group = &sr.groups()[0];

    assert_eq!(
        group.reference_kind(),
        StructuredReportReferenceKind::Segmentation
    );
    assert_eq!(
        group.referenced_segmentation_uid(),
        Some(seg.sop_instance_uid())
    );
    assert_eq!(group.referenced_segment_number(), Some(1));
    assert_eq!(
        group.tracking_id(),
        seg.segments()[0].tracking_id().unwrap()
    );
    assert_eq!(
        group.tracking_uid(),
        seg.segments()[0].tracking_uid().unwrap()
    );
}

#[test]
fn shared_pathology_documents_build_and_verify_a_seg_referenced_sr_bundle() {
    let (directory, source) = source_fixture();
    let seg_path = directory.path().join("seg.dcm");
    let sr_path = directory.path().join("sr.dcm");
    let mapping = mapping_with_sr();
    let geojson = br#"
    {
      "type":"FeatureCollection",
      "features":[{
        "type":"Feature",
        "id":"2.25.704",
        "geometry":{"type":"Polygon","coordinates":[
          [[1,1],[10,1],[10,10],[1,10],[1,1]],
          [[3,3],[3,6],[6,6],[6,3],[3,3]]
        ]},
        "properties":{"classification":{"name":"viable_tumor"}}
      }]
    }
    "#;
    let annotations = parse_fixture(geojson, &mapping, &source, false).unwrap();

    let producer = |series_number, description| {
        DerivedObjectProducer::new(
            series_number,
            "Frames",
            "DICOM Viewer",
            "not-applicable",
            "0.1.0",
        )
        .unwrap()
        .with_series_description(description)
        .unwrap()
    };
    let documents = PathologyDicomDocuments::build(
        &annotations,
        &[PathologyDicomTarget::Seg, PathologyDicomTarget::Sr],
    )
    .unwrap()
    .with_producers(
        producer(9101, "WSI annotations"),
        producer(9201, "WSI segmentations"),
        producer(9301, "WSI measurement reports"),
    );

    assert!(documents.ann().is_none());
    assert!(documents.seg().is_some());
    assert!(documents.sr().is_some());
    assert_eq!(documents.seg().unwrap().producer().manufacturer(), "Frames");
    assert_eq!(
        documents.sr().unwrap().producer().manufacturer_model_name(),
        "DICOM Viewer"
    );
    let missing_ann_path = directory.path().join("missing-ann.dcm");
    assert!(matches!(
        documents
            .write_and_verify(PathologyDicomTarget::Ann, &missing_ann_path, &source)
            .unwrap_err(),
        crate::PathologyDocumentWriteError::MissingTarget
    ));
    assert!(!missing_ann_path.exists());
    documents
        .write_and_verify(PathologyDicomTarget::Seg, &seg_path, &source)
        .unwrap();
    documents
        .write_and_verify(PathologyDicomTarget::Sr, &sr_path, &source)
        .unwrap();
    assert_eq!(
        StructuredReportDocument::read_sr(&sr_path, &source, documents.seg())
            .unwrap()
            .groups()[0]
            .referenced_segmentation_uid(),
        documents.seg().map(|seg| seg.sop_instance_uid())
    );

    assert!(PathologyDicomDocuments::build(&annotations, &[]).is_err());
    assert!(PathologyDicomDocuments::build(
        &annotations,
        &[PathologyDicomTarget::Seg, PathologyDicomTarget::Seg]
    )
    .is_err());
}

#[test]
fn pathology_semantic_digest_includes_sr_report_context() {
    let (_directory, source) = source_fixture();
    let geojson = br#"
    {
      "type":"FeatureCollection",
      "features":[{
        "type":"Feature",
        "id":"2.25.52",
        "geometry":{"type":"Point","coordinates":[2,3]},
        "properties":{"classification":{"name":"viable_tumor"}}
      }]
    }
    "#;
    let first_mapping = mapping_with_sr();
    let mut second_mapping: serde_json::Value = serde_json::from_slice(&first_mapping).unwrap();
    second_mapping["sr"]["report_title"]["code_meaning"] =
        serde_json::json!("Pathology Measurement Report");
    let first = parse_fixture(geojson, &first_mapping, &source, false).unwrap();
    let second = parse_fixture(
        geojson,
        &serde_json::to_vec(&second_mapping).unwrap(),
        &source,
        false,
    )
    .unwrap();

    assert_ne!(first.semantic_sha256(), second.semantic_sha256());
}

fn mapping_with_sr() -> Vec<u8> {
    let mut mapping: serde_json::Value = serde_json::from_str(MAPPING).unwrap();
    mapping["measurements"]["Area"] = serde_json::json!({
        "concept": {
            "code_value": "AREA",
            "coding_scheme_designator": "99WSI",
            "code_meaning": "Area"
        },
        "unit": {
            "code_value": "mm2",
            "coding_scheme_designator": "UCUM",
            "code_meaning": "square millimeter"
        }
    });
    mapping["qualitative_evaluations"]["grade"] = serde_json::json!({
        "concept": {
            "code_value": "GRADE",
            "coding_scheme_designator": "99WSI",
            "code_meaning": "Grade"
        },
        "values": {
            "high": {
                "code_value": "HIGH",
                "coding_scheme_designator": "99WSI",
                "code_meaning": "High"
            }
        }
    });
    mapping["sr"] = serde_json::json!({
        "report_title": {
            "code_value": "126000",
            "coding_scheme_designator": "DCM",
            "code_meaning": "Imaging Measurement Report"
        },
        "procedures_reported": [{
            "code_value": "P5-09051",
            "coding_scheme_designator": "SRT",
            "code_meaning": "Histopathology procedure"
        }]
    });
    serde_json::to_vec(&mapping).unwrap()
}
