use super::{
    annotation_class_concept_key, dicom_cielab_to_srgb, srgb_to_dicom_cielab,
    AnnotationClassGeometry, AnnotationScheme, DicomCode, TrackingIdentity,
};

#[test]
fn public_concept_key_ignores_display_meaning_but_keeps_geometry() {
    let category = DicomCode::new("49755003", "SCT", "First display meaning").unwrap();
    let property = DicomCode::new("108369006", "SCT", "Neoplasm").unwrap();
    let renamed_category =
        DicomCode::new("49755003", "SCT", "A different display meaning").unwrap();

    let region =
        annotation_class_concept_key(AnnotationClassGeometry::Region, &category, &property, &[]);
    let renamed = annotation_class_concept_key(
        AnnotationClassGeometry::Region,
        &renamed_category,
        &property,
        &[],
    );
    let point =
        annotation_class_concept_key(AnnotationClassGeometry::Point, &category, &property, &[]);

    assert_eq!(region, renamed);
    assert_ne!(region, point);
}

#[test]
fn general_pathology_scheme_is_exact_qualified_and_roundtrips() {
    let scheme = AnnotationScheme::general_pathology_v1();

    assert_eq!(scheme.schema_version(), 1);
    assert_eq!(scheme.id(), "org.frames.general-pathology");
    assert_eq!(scheme.version(), 1);
    assert_eq!(scheme.display_name(), "General Pathology");
    assert_eq!(scheme.classes().len(), 8);

    let expected = [
        (
            "tissue",
            "Tissue",
            AnnotationClassGeometry::Region,
            [0x99, 0x99, 0x99],
            "85756007",
            "85756007",
        ),
        (
            "neoplasm",
            "Neoplasm",
            AnnotationClassGeometry::Region,
            [0xD5, 0x5E, 0x00],
            "49755003",
            "108369006",
        ),
        (
            "necrosis",
            "Necrosis",
            AnnotationClassGeometry::Region,
            [0xE6, 0x9F, 0x00],
            "49755003",
            "6574001",
        ),
        (
            "inflammation",
            "Inflammation",
            AnnotationClassGeometry::Region,
            [0xF0, 0xE4, 0x42],
            "49755003",
            "409774005",
        ),
        (
            "stroma",
            "Stroma / connective tissue",
            AnnotationClassGeometry::Region,
            [0x00, 0x9E, 0x73],
            "85756007",
            "21793004",
        ),
        (
            "unusable-tissue",
            "Unusable tissue",
            AnnotationClassGeometry::Region,
            [0xCC, 0x79, 0xA7],
            "263496004",
            "131502",
        ),
        (
            "cell",
            "Cell",
            AnnotationClassGeometry::Point,
            [0x56, 0xB4, 0xE9],
            "4421005",
            "4421005",
        ),
        (
            "nucleus",
            "Nucleus",
            AnnotationClassGeometry::Point,
            [0x00, 0x72, 0xB2],
            "4421005",
            "84640000",
        ),
    ];
    for (index, (class, expected)) in scheme.classes().iter().zip(expected).enumerate() {
        assert_eq!(class.id(), expected.0);
        assert_eq!(class.label(), expected.1);
        assert_eq!(class.geometry(), expected.2);
        assert_eq!(class.display_color(), expected.3);
        assert_eq!(class.category().value(), expected.4);
        assert_eq!(class.property_type().value(), expected.5);
        if index < 6 {
            assert_eq!(class.category().mapping_resource(), Some("DCMR"));
            assert_eq!(class.category().context_identifier(), Some("7150"));
        } else {
            assert_eq!(class.category().mapping_resource(), None);
            assert_eq!(class.category().context_identifier(), None);
        }
        assert_eq!(
            class.property_type().mapping_resource_uid(),
            Some("1.2.840.10008.8.1.1")
        );
        assert!(class.property_type().context_identifier().is_some());
    }

    let json = scheme.to_json().expect("built-in scheme should serialize");
    let restored = AnnotationScheme::from_json(&json).expect("built-in scheme should parse");
    assert_eq!(restored, scheme);
    assert_eq!(restored.content_digest(), scheme.content_digest());
}

#[test]
fn scheme_parsing_is_strict_bounded_and_rejects_junk_classes() {
    let duplicate_ids = br##"{
      "schema_version":1,"scheme_id":"org.example.test","scheme_version":1,
      "display_name":"Test","classes":[
        {"id":"same","label":"Tissue","geometry":"REGION","display_color":"#999999",
         "category":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Tissue"},
         "property_type":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Tissue"}},
        {"id":"same","label":"Neoplasm","geometry":"REGION","display_color":"#D55E00",
         "category":{"code_value":"49755003","coding_scheme_designator":"SCT","code_meaning":"Morphologically Abnormal Structure"},
         "property_type":{"code_value":"108369006","coding_scheme_designator":"SCT","code_meaning":"Neoplasm"}}
      ]
    }"##;
    let error = AnnotationScheme::from_json(duplicate_ids).expect_err("duplicate class IDs");
    assert!(error.to_string().contains("duplicate class id"));

    let duplicate_concepts = br##"{
      "schema_version":1,"scheme_id":"org.example.test","scheme_version":1,
      "display_name":"Test","classes":[
        {"id":"first","label":"Tissue","geometry":"REGION","display_color":"#999999",
         "category":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Tissue"},
         "property_type":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Tissue"}},
        {"id":"second","label":"Renamed","geometry":"REGION","display_color":"#FFFFFF",
         "category":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Different display meaning"},
         "property_type":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Different display meaning"}}
      ]
    }"##;
    let error = AnnotationScheme::from_json(duplicate_concepts).expect_err("duplicate concepts");
    assert!(error.to_string().contains("duplicate class concept"));

    let unknown = br##"{
      "schema_version":1,"scheme_id":"org.example.test","scheme_version":1,
      "display_name":"Test","classes":[],"new_class_button":true
    }"##;
    let error = AnnotationScheme::from_json(unknown).expect_err("unknown fields");
    assert!(error.to_string().contains("not valid JSON"));

    let oversized = vec![b' '; 4 * 1024 * 1024 + 1];
    let error = AnnotationScheme::from_json(&oversized).expect_err("oversized scheme");
    assert!(error.to_string().contains("4 MiB"));
}

#[test]
fn concept_identity_ignores_presentation_but_content_digest_does_not() {
    let first = br##"{
      "schema_version":1,"scheme_id":"org.example.first","scheme_version":1,
      "display_name":"First","classes":[
        {"id":"tissue","label":"Tissue","geometry":"REGION","display_color":"#999999",
         "category":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Tissue"},
         "property_type":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Tissue"}}
      ]
    }"##;
    let second = br##"{
      "schema_version":1,"scheme_id":"org.example.second","scheme_version":4,
      "display_name":"Second","classes":[
        {"id":"gray","label":"Renamed tissue","geometry":"REGION","display_color":"#FFFFFF",
         "category":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Alternate wording"},
         "property_type":{"code_value":"85756007","coding_scheme_designator":"SCT","code_meaning":"Alternate wording"}}
      ]
    }"##;
    let first = AnnotationScheme::from_json(first).expect("first scheme");
    let second = AnnotationScheme::from_json(second).expect("second scheme");

    assert_eq!(
        first.classes()[0].concept_key(),
        second.classes()[0].concept_key()
    );
    assert_ne!(first.content_digest(), second.content_digest());
}

#[test]
fn dicom_cielab_roundtrips_builtin_colors_within_two_code_levels() {
    for rgb in [
        [0x99, 0x99, 0x99],
        [0xD5, 0x5E, 0x00],
        [0xE6, 0x9F, 0x00],
        [0xF0, 0xE4, 0x42],
        [0x00, 0x9E, 0x73],
        [0xCC, 0x79, 0xA7],
        [0x56, 0xB4, 0xE9],
        [0x00, 0x72, 0xB2],
    ] {
        let restored = dicom_cielab_to_srgb(srgb_to_dicom_cielab(rgb));
        for (actual, expected) in restored.into_iter().zip(rgb) {
            assert!(
                actual.abs_diff(expected) <= 2,
                "{rgb:?} became {restored:?}"
            );
        }
    }
}

#[test]
fn tracking_identity_is_assigned_once_and_is_dicom_valid() {
    let identity = TrackingIdentity::generated("F", 7).expect("generated identity");
    assert_eq!(identity.id(), "F-000007");
    assert!(identity.uid().starts_with("2.25."));
    assert!(identity.uid().len() <= 64);

    let restored = TrackingIdentity::new(identity.id(), identity.uid()).expect("valid identity");
    assert_eq!(restored, identity);
    let error = TrackingIdentity::new("F-1", "not-a-uid").expect_err("invalid UID");
    assert!(error.to_string().contains("tracking UID"));
}
