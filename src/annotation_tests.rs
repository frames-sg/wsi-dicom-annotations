use dicom_core::value::{DataSetSequence, PrimitiveValue, Value};
use dicom_core::{DataElement, Length, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;

use crate::test_support::{
    write_fractional_seg, write_native_ann, write_source_wsi, write_source_wsi_with_spacing,
    write_sparse_source_wsi,
};
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

#[path = "annotation_tests/ann_identity.rs"]
mod ann_identity;
#[path = "annotation_tests/ann_roundtrip.rs"]
mod ann_roundtrip;
#[path = "annotation_tests/coordinate_projection.rs"]
mod coordinate_projection;
#[path = "annotation_tests/external_validator.rs"]
mod external_validator;
#[path = "annotation_tests/seg_roundtrip.rs"]
mod seg_roundtrip;
#[path = "annotation_tests/sidecar_boundary.rs"]
mod sidecar_boundary;
