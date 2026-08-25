use super::*;
use crate::{DiagnosticDisposition, DiagnosticSeverity};

fn code(value: &str, meaning: &str) -> DicomCode {
    DicomCode::new(value, "99FRAMES", meaning).unwrap()
}

#[test]
fn semantic_vectorization_preserves_ann_fields_and_reports_identity_loss() {
    let (mut segmentation, _) = super::performance_tests::benchmark_documents();
    segmentation.segments.truncate(1);
    segmentation
        .imported_binary_frames
        .as_mut()
        .unwrap()
        .retain(|frame| frame.segment_number == 1);
    let modifier = code("INV", "Invasive");
    let region = code("BREAST", "Breast");
    let structure = code("LOBULE", "Lobule");
    let algorithm =
        AlgorithmIdentification::new(code("AI", "Artificial intelligence"), "classifier", "2.0")
            .unwrap();
    segmentation.segments[0] = segmentation.segments[0]
        .clone()
        .with_description("Full semantic segment")
        .unwrap()
        .with_generation(GenerationType::Automatic, vec![algorithm.clone()])
        .unwrap()
        .with_property_type_modifiers(vec![modifier.clone()])
        .with_anatomic_regions(vec![region.clone()])
        .with_primary_anatomic_structures(vec![structure.clone()])
        .with_tracking("tracking-id", "2.25.4242")
        .unwrap();

    let rejected = segmentation
        .vectorized_annotations(SegToAnnConversionPolicy::RejectLoss)
        .unwrap_err();
    assert!(rejected
        .to_string()
        .contains("SEGMENT_NUMBER_NOT_REPRESENTABLE"));
    assert!(rejected
        .to_string()
        .contains("SEG_TRACKING_ID_NOT_REPRESENTABLE"));

    let vectorized = segmentation
        .vectorized_annotations(SegToAnnConversionPolicy::AllowLoss)
        .unwrap();
    assert_eq!(vectorized.groups().len(), 1);
    let group = &vectorized.groups()[0];
    let segment = &segmentation.segments[0];
    assert_eq!(group.uid(), "2.25.4242");
    assert_eq!(group.label(), segment.label());
    assert_eq!(group.description(), "Full semantic segment");
    assert_eq!(group.generation_type(), GenerationType::Automatic);
    assert_eq!(group.algorithms(), &[algorithm]);
    assert_eq!(group.category(), segment.category());
    assert_eq!(group.property_type(), segment.property_type());
    assert_eq!(group.property_type_modifiers(), &[modifier]);
    assert_eq!(group.anatomic_regions(), &[region]);
    assert_eq!(group.primary_anatomic_structures(), &[structure]);
    assert_eq!(
        group.recommended_display_cielab(),
        segment.recommended_display_cielab()
    );
    let codes = vectorized
        .diagnostics()
        .iter()
        .map(InteroperabilityDiagnostic::code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"SEGMENT_NUMBER_NOT_REPRESENTABLE"));
    assert!(codes.contains(&"SEG_TRACKING_ID_NOT_REPRESENTABLE"));
    assert!(codes.contains(&"SEG_RASTER_VECTORIZED"));
}

#[test]
fn semantic_vectorization_rejects_an_empty_segment_instead_of_dropping_it() {
    let (mut segmentation, _) = super::performance_tests::benchmark_documents();
    segmentation.segments.truncate(2);
    segmentation.segments[0].source_segment_number = None;
    segmentation.segments[1].source_segment_number = None;
    segmentation
        .imported_binary_frames
        .as_mut()
        .unwrap()
        .retain(|frame| frame.segment_number == 1);
    for frame in segmentation.imported_binary_frames.as_mut().unwrap() {
        frame.segment_number = 1;
    }

    let error = segmentation
        .vectorized_annotations(SegToAnnConversionPolicy::RejectLoss)
        .unwrap_err();
    assert!(error.to_string().contains("EMPTY_SEGMENT_NOT_VECTORIZED"));
}

#[test]
fn semantic_vectorization_carries_import_loss_diagnostics() {
    let (mut segmentation, _) = super::performance_tests::benchmark_documents();
    segmentation.segments.truncate(1);
    segmentation
        .imported_binary_frames
        .as_mut()
        .unwrap()
        .retain(|frame| frame.segment_number == 1);
    segmentation.segments[0].source_segment_number = None;
    segmentation
        .diagnostics
        .push(InteroperabilityDiagnostic::new(
            "SEG_APPLICABILITY_NOT_REPRESENTABLE",
            DiagnosticSeverity::Warning,
            "$.SegmentSequence[0].Applicability",
            DiagnosticDisposition::WouldDrop,
            "synthetic imported applicability restriction cannot be represented in ANN",
        ));

    let error = segmentation
        .vectorized_annotations(SegToAnnConversionPolicy::RejectLoss)
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("SEG_APPLICABILITY_NOT_REPRESENTABLE"));

    let vectorized = segmentation
        .vectorized_annotations(SegToAnnConversionPolicy::AllowLoss)
        .unwrap();
    assert!(vectorized
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code() == "SEG_APPLICABILITY_NOT_REPRESENTABLE"));
}
