use super::*;
use crate::{AnnotationGroup, Point2};

fn code(value: &str, meaning: &str) -> DicomCode {
    DicomCode::new(value, "99FRAMES", meaning).unwrap()
}

#[test]
fn one_validated_finding_value_populates_ann_and_seg_equivalently() {
    let algorithm =
        AlgorithmIdentification::new(code("AI", "Artificial intelligence"), "classifier", "1.0")
            .unwrap();
    let semantics = FindingSemantics::new(
        GenerationType::Automatic,
        vec![algorithm.clone()],
        code("MORPH", "Morphology"),
        code("TUMOR", "Tumor"),
        vec![code("INV", "Invasive")],
        vec![code("BREAST", "Breast")],
        vec![code("LOBULE", "Lobule")],
        [10, 20, 30],
    )
    .unwrap();
    let ann = AnnotationGroup::polygons(
        "Tumor",
        code("MORPH", "Morphology"),
        code("TUMOR", "Tumor"),
        [0, 0, 0],
        vec![vec![
            Point2::new(0.0, 0.0),
            Point2::new(4.0, 0.0),
            Point2::new(4.0, 4.0),
            Point2::new(0.0, 4.0),
        ]],
    )
    .unwrap()
    .with_finding_semantics(semantics.clone())
    .unwrap();
    let seg = crate::annotations::seg::SegmentationSegment::new(
        "Tumor",
        code("MORPH", "Morphology"),
        code("TUMOR", "Tumor"),
        [0, 0, 0],
        ann.polygon_annotations().unwrap().to_vec(),
        Vec::new(),
    )
    .unwrap()
    .with_finding_semantics(semantics)
    .unwrap();

    assert_eq!(ann.generation_type(), seg.generation_type());
    assert_eq!(ann.algorithms(), seg.algorithms());
    assert_eq!(ann.category(), seg.category());
    assert_eq!(ann.property_type(), seg.property_type());
    assert_eq!(ann.property_type_modifiers(), seg.property_type_modifiers());
    assert_eq!(ann.anatomic_regions(), seg.anatomic_regions());
    assert_eq!(
        ann.primary_anatomic_structures(),
        seg.primary_anatomic_structures()
    );
    assert_eq!(
        ann.recommended_display_cielab(),
        seg.recommended_display_cielab()
    );
    assert_eq!(ann.algorithms(), &[algorithm]);
}

#[test]
fn shared_finding_validation_rejects_generation_mismatch_once() {
    assert!(FindingSemantics::new(
        GenerationType::Automatic,
        Vec::new(),
        code("MORPH", "Morphology"),
        code("TUMOR", "Tumor"),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        [0, 0, 0],
    )
    .is_err());
}
