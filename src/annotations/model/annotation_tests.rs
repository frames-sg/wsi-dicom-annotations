use super::*;

#[test]
fn indexed_measurements_require_one_based_strictly_ordered_matches() {
    let concept = DicomCode::new("AREA", "99FRAMES", "Area").unwrap();
    let units = DicomCode::new("mm2", "UCUM", "square millimeter").unwrap();
    let measurement = AnnotationMeasurement::for_annotations(
        concept.clone(),
        units.clone(),
        vec![2.0, 4.0],
        vec![1, 3],
    )
    .unwrap();
    assert_eq!(measurement.concept(), &concept);
    assert_eq!(measurement.units(), &units);
    assert_eq!(measurement.values(), &[2.0, 4.0]);
    assert_eq!(measurement.annotation_indices(), Some([1, 3].as_slice()));

    assert!(AnnotationMeasurement::for_annotations(
        concept.clone(),
        units.clone(),
        vec![1.0],
        vec![1, 2]
    )
    .is_err());
    assert!(AnnotationMeasurement::for_annotations(
        concept.clone(),
        units.clone(),
        vec![1.0, 2.0],
        vec![2, 2]
    )
    .is_err());
    assert!(AnnotationMeasurement::for_annotations(concept, units, vec![1.0], vec![0]).is_err());
}

#[test]
fn group_builders_preserve_declared_semantics_and_edit_only_matching_geometry() {
    let category = DicomCode::new("MORPH", "99FRAMES", "Morphology").unwrap();
    let property = DicomCode::new("TUMOR", "99FRAMES", "Tumor").unwrap();
    let modifier = DicomCode::new("INV", "99FRAMES", "Invasive").unwrap();
    let region = DicomCode::new("BREAST", "99FRAMES", "Breast").unwrap();
    let structure = DicomCode::new("LOBULE", "99FRAMES", "Lobule").unwrap();
    let algorithm = AlgorithmIdentification::new(
        DicomCode::new("AI", "99FRAMES", "Artificial intelligence").unwrap(),
        "classifier",
        "1.0",
    )
    .unwrap();
    let mut points = AnnotationGroup::points(
        "Cells",
        category.clone(),
        property.clone(),
        [1, 2, 3],
        vec![Point2::new(1.0, 2.0), Point2::new(3.0, 4.0)],
    )
    .unwrap()
    .with_description("model-selected cells")
    .unwrap()
    .with_generation(GenerationType::Automatic, vec![algorithm])
    .unwrap()
    .with_property_type_modifiers(vec![modifier.clone()])
    .with_anatomic_regions(vec![region.clone()])
    .with_primary_anatomic_structures(vec![structure.clone()])
    .with_referenced_optical_paths(vec!["A".into(), "B".into()])
    .unwrap()
    .with_common_z_coordinates(vec![0.0, 0.5])
    .unwrap();
    points
        .add_measurement(
            AnnotationMeasurement::for_annotations(
                DicomCode::new("AREA", "99FRAMES", "Area").unwrap(),
                DicomCode::new("mm2", "UCUM", "square millimeter").unwrap(),
                vec![2.5],
                vec![2],
            )
            .unwrap(),
        )
        .unwrap();

    assert_eq!(points.description(), "model-selected cells");
    assert_eq!(points.generation_type(), GenerationType::Automatic);
    assert_eq!(points.algorithms().len(), 1);
    assert_eq!(
        points.property_type_modifiers(),
        std::slice::from_ref(&modifier)
    );
    assert_eq!(points.anatomic_regions(), std::slice::from_ref(&region));
    assert_eq!(
        points.primary_anatomic_structures(),
        std::slice::from_ref(&structure)
    );
    assert!(!points.applies_to_all_optical_paths());
    assert_eq!(points.referenced_optical_paths(), &["A", "B"]);
    assert!(!points.applies_to_all_z_planes());
    assert_eq!(points.common_z_coordinates(), &[0.0, 0.5]);
    assert!(points.editable());
    assert!(points.polygon_annotations().is_none());
    points
        .replace_points(vec![Point2::new(2.0, 2.0), Point2::new(3.0, 4.0)])
        .unwrap();

    let mut polygons = AnnotationGroup::polygons(
        "Region",
        category,
        property,
        [4, 5, 6],
        vec![vec![
            Point2::new(0.0, 0.0),
            Point2::new(4.0, 0.0),
            Point2::new(4.0, 4.0),
            Point2::new(0.0, 4.0),
        ]],
    )
    .unwrap();
    assert!(polygons.point_annotations().is_none());
    polygons
        .replace_polygons(vec![vec![
            Point2::new(0.5, 0.5),
            Point2::new(4.0, 0.0),
            Point2::new(4.0, 4.0),
            Point2::new(0.0, 4.0),
        ]])
        .unwrap();
}

#[test]
fn checked_geometry_replacement_is_atomic_and_revalidates_topology_and_coordinates() {
    let category = DicomCode::new("MORPH", "99FRAMES", "Morphology").unwrap();
    let property = DicomCode::new("TUMOR", "99FRAMES", "Tumor").unwrap();
    let square = vec![
        Point2::new(0.0, 0.0),
        Point2::new(4.0, 0.0),
        Point2::new(4.0, 4.0),
        Point2::new(0.0, 4.0),
    ];
    let mut polygons = AnnotationGroup::polygons(
        "Region",
        category.clone(),
        property.clone(),
        [1, 2, 3],
        vec![square.clone()],
    )
    .unwrap();

    for invalid in [
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(4.0, 4.0),
            Point2::new(0.0, 4.0),
            Point2::new(4.0, 0.0),
        ],
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(2.0, 0.0),
        ],
        vec![
            Point2::new(f64::NAN, 0.0),
            Point2::new(4.0, 0.0),
            Point2::new(0.0, 4.0),
        ],
        vec![
            Point2::new(f64::INFINITY, 0.0),
            Point2::new(4.0, 0.0),
            Point2::new(0.0, 4.0),
        ],
    ] {
        assert!(polygons.replace_polygons(vec![invalid]).is_err());
        assert_eq!(
            polygons.polygon_annotations(),
            Some([square.clone()].as_slice())
        );
    }

    let mut points = AnnotationGroup::points(
        "Cells",
        category,
        property,
        [1, 2, 3],
        vec![Point2::new(1.0, 1.0), Point2::new(2.0, 2.0)],
    )
    .unwrap();
    points
        .add_measurement(AnnotationMeasurement::new(
            DicomCode::new("AREA", "99FRAMES", "Area").unwrap(),
            DicomCode::new("mm2", "UCUM", "square millimeter").unwrap(),
            vec![1.0, 2.0],
        ))
        .unwrap();
    let original = points.clone();
    assert!(points.replace_points(vec![Point2::new(1.0, 1.0)]).is_err());
    assert!(points
        .replace_points(vec![
            Point2::new(1.0, 1.0),
            Point2::new(2.0, 2.0),
            Point2::new(3.0, 3.0),
        ])
        .is_err());
    assert_eq!(points, original);
}

#[test]
fn checked_geometry_replacement_rejects_removed_indexed_measurement_target() {
    let mut points = AnnotationGroup::points(
        "Cells",
        DicomCode::new("MORPH", "99FRAMES", "Morphology").unwrap(),
        DicomCode::new("TUMOR", "99FRAMES", "Tumor").unwrap(),
        [1, 2, 3],
        vec![Point2::new(1.0, 1.0), Point2::new(2.0, 2.0)],
    )
    .unwrap();
    points
        .add_measurement(
            AnnotationMeasurement::for_annotations(
                DicomCode::new("AREA", "99FRAMES", "Area").unwrap(),
                DicomCode::new("mm2", "UCUM", "square millimeter").unwrap(),
                vec![2.0],
                vec![2],
            )
            .unwrap(),
        )
        .unwrap();
    let original = points.clone();

    assert!(points.replace_points(vec![Point2::new(1.0, 1.0)]).is_err());
    assert_eq!(points, original);
}

#[test]
fn group_builders_reject_conflicting_applicability_and_measurement_shapes() {
    let category = DicomCode::new("MORPH", "99FRAMES", "Morphology").unwrap();
    let property = DicomCode::new("TUMOR", "99FRAMES", "Tumor").unwrap();
    let base = || {
        AnnotationGroup::points(
            "Cells",
            category.clone(),
            property.clone(),
            [1, 2, 3],
            vec![Point2::new(1.0, 2.0)],
        )
        .unwrap()
    };

    assert!(base().with_description("contains\0nul").is_err());
    assert!(base().with_referenced_optical_paths(Vec::new()).is_err());
    assert!(base()
        .with_referenced_optical_paths(vec!["A".into(), "A".into()])
        .is_err());
    assert!(base().with_common_z_coordinates(Vec::new()).is_err());
    assert!(base().with_common_z_coordinates(vec![f64::NAN]).is_err());

    let mut group = base();
    let concept = DicomCode::new("AREA", "99FRAMES", "Area").unwrap();
    let units = DicomCode::new("mm2", "UCUM", "square millimeter").unwrap();
    assert!(group
        .add_measurement(AnnotationMeasurement::new(
            concept.clone(),
            units.clone(),
            Vec::new(),
        ))
        .is_err());
    assert!(group
        .add_measurement(AnnotationMeasurement::new(
            concept,
            units,
            vec![f64::INFINITY]
        ))
        .is_err());
    assert!(group.measurements().is_empty());
}
