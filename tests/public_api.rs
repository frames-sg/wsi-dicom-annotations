#[cfg(feature = "parametric-map")]
use wsi_dicom_annotations::ParametricMapDocument;
use wsi_dicom_annotations::{
    AnnotationDocument, DicomAnnotationContext, PathologyAnnotationSet, SegmentationDocument,
    StructuredReportDocument,
};

#[test]
fn exposes_format_neutral_annotation_documents() {
    fn assert_public<T>() {
        assert!(!std::any::type_name::<T>().is_empty());
    }

    assert_public::<AnnotationDocument>();
    assert_public::<DicomAnnotationContext>();
    assert_public::<SegmentationDocument>();
    assert_public::<StructuredReportDocument>();
    #[cfg(feature = "parametric-map")]
    assert_public::<ParametricMapDocument>();
    assert_public::<PathologyAnnotationSet>();
}
