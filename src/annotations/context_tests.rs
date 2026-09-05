use super::*;
use crate::annotations::dicom_dataset::sequence;
use dicom_core::value::PrimitiveValue;
use dicom_core::{dicom_value, DataElement, VR};
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

fn empty_file_object() -> DefaultDicomObject {
    InMemDicomObject::new_empty()
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE)
                .media_storage_sop_instance_uid("2.25.999")
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .expect("test file metadata should be valid")
}

#[test]
fn context_projects_level_zero_slide_and_tiled_full_frame_coordinates() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let source = directory.path().join("source.dcm");
    crate::test_support::write_source_wsi(&source, 16, 12, 4, 4);
    let context = DicomAnnotationContext::from_source(&source)
        .expect("synthetic VL WSI context should parse");

    assert_eq!(context.source_path(), source);
    assert_eq!(
        context.sop_class_uid(),
        uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE
    );
    assert_eq!(context.sop_instance_uid(), "1.2.826.0.1.3680043.10.777.101");
    assert_eq!(
        context.series_instance_uid(),
        "1.2.826.0.1.3680043.10.777.102"
    );
    assert_eq!(
        context.study_instance_uid(),
        "1.2.826.0.1.3680043.10.777.103"
    );
    assert_eq!(
        context.frame_of_reference_uid(),
        Some("1.2.826.0.1.3680043.10.777.104")
    );
    assert_eq!(context.total_pixel_matrix_dimensions(), (16, 12));
    assert_eq!(context.tile_dimensions(), (4, 4));
    assert_eq!(context.pixel_spacing(), Some([0.00025, 0.00025]));
    assert_eq!(context.slice_thickness(), Some(0.001));
    assert_eq!(
        context.image_orientation_slide(),
        Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
    );
    assert_eq!(context.total_pixel_matrix_origin(), Some([0.0, 0.0, 0.0]));

    let slide = context
        .pixel_to_slide_coordinate(4.0, 8.0)
        .expect("in-bounds pixel should project to slide coordinates");
    assert_eq!([slide.x, slide.y, slide.z], [0.001, 0.002, 0.0]);
    assert_eq!(
        context
            .slide_coordinate_to_pixel3(slide.x, slide.y, slide.z)
            .expect("slide coordinate should invert"),
        Point2::new(4.0, 8.0)
    );
    assert_eq!(
        context
            .slide_coordinate_to_pixel(slide.x, slide.y)
            .expect("planar slide coordinate should invert"),
        Point2::new(4.0, 8.0)
    );
    assert_eq!(
        context
            .frame_coordinate_to_total_pixel(2, 1.5, 2.5)
            .expect("second tiled-full frame should project"),
        Point2::new(5.5, 2.5)
    );
    assert!(context.source_metadata().is_ok());
    assert!(context.require_shared_slide_frame(&context).is_ok());

    assert!(context.pixel_to_slide_coordinate(f64::NAN, 0.0).is_err());
    assert!(context.pixel_to_slide_coordinate(17.0, 0.0).is_err());
    assert!(context
        .slide_coordinate_to_pixel3(f64::INFINITY, 0.0, 0.0)
        .is_err());
    assert!(context
        .frame_coordinate_to_total_pixel(0, 0.0, 0.0)
        .is_err());
    assert!(context
        .frame_coordinate_to_total_pixel(99, 0.0, 0.0)
        .is_err());

    let mut different_study = context.clone();
    different_study.study_instance_uid = "2.25.1".into();
    assert!(context
        .require_shared_slide_frame(&different_study)
        .is_err());
    let mut missing_frame = context.clone();
    missing_frame.frame_of_reference_uid = None;
    assert!(context.require_shared_slide_frame(&missing_frame).is_err());
}

#[test]
fn coordinate_projection_reports_each_missing_source_geometry_attribute() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    crate::test_support::write_source_wsi(&source, 16, 12, 4, 4);
    let context = DicomAnnotationContext::from_source(&source).unwrap();

    let mut missing_origin = context.clone();
    missing_origin.total_pixel_matrix_origin = None;
    assert!(missing_origin.slide_coordinate_to_pixel(0.0, 0.0).is_err());
    assert!(missing_origin
        .slide_coordinate_to_pixel3(0.0, 0.0, 0.0)
        .is_err());
    assert!(missing_origin.pixel_to_slide_coordinate(0.0, 0.0).is_err());

    let mut missing_orientation = context.clone();
    missing_orientation.image_orientation_slide = None;
    assert!(missing_orientation
        .slide_coordinate_to_pixel3(0.0, 0.0, 0.0)
        .is_err());
    assert!(missing_orientation
        .pixel_to_slide_coordinate(0.0, 0.0)
        .is_err());

    let mut missing_spacing = context.clone();
    missing_spacing.pixel_spacing = None;
    assert!(missing_spacing
        .slide_coordinate_to_pixel3(0.0, 0.0, 0.0)
        .is_err());
    assert!(missing_spacing.pixel_to_slide_coordinate(0.0, 0.0).is_err());

    let mut missing_frames = context.clone();
    missing_frames.number_of_frames = None;
    assert!(missing_frames
        .frame_coordinate_to_total_pixel(1, 0.0, 0.0)
        .is_err());

    let mut missing_sparse_origin = context;
    missing_sparse_origin.dimension_organization_type = Some("TILED_SPARSE".into());
    missing_sparse_origin.sparse_frame_origins = None;
    assert!(missing_sparse_origin
        .frame_coordinate_to_total_pixel(1, 0.0, 0.0)
        .is_err());
}

#[test]
fn context_uses_shared_pixel_measures_when_top_level_values_are_absent() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    crate::test_support::write_source_wsi(&source, 16, 12, 4, 4);
    let mut object = dicom_object::open_file(&source).unwrap();
    object.take(tags::PIXEL_SPACING).unwrap();
    object.take(tags::SLICE_THICKNESS).unwrap();

    let mut measures = InMemDicomObject::new_empty();
    measures.put(DataElement::new(
        tags::PIXEL_SPACING,
        VR::DS,
        dicom_value!(F64, [0.5, 0.25]),
    ));
    measures.put(DataElement::new(tags::SLICE_THICKNESS, VR::DS, "0.01"));
    let mut shared = InMemDicomObject::new_empty();
    shared.put(sequence(tags::PIXEL_MEASURES_SEQUENCE, vec![measures]));
    object.put(sequence(
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![shared],
    ));
    object.write_to_file(&source).unwrap();

    let context = DicomAnnotationContext::from_source(&source).unwrap();
    assert_eq!(context.pixel_spacing(), Some([0.5, 0.25]));
    assert_eq!(context.slice_thickness(), Some(0.01));
}

#[test]
fn context_retains_declared_optical_path_identifiers() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    crate::test_support::write_source_wsi(&source, 16, 12, 4, 4);
    let mut object = dicom_object::open_file(&source).unwrap();
    let paths = ["BRIGHTFIELD", "FLUORESCENCE"]
        .into_iter()
        .map(|identifier| {
            let mut item = InMemDicomObject::new_empty();
            item.put(DataElement::new(
                tags::OPTICAL_PATH_IDENTIFIER,
                VR::SH,
                identifier,
            ));
            item
        })
        .collect();
    object.put(sequence(tags::OPTICAL_PATH_SEQUENCE, paths));
    object.put(DataElement::new(
        tags::NUMBER_OF_OPTICAL_PATHS,
        VR::UL,
        PrimitiveValue::from(2_u32),
    ));
    object.write_to_file(&source).unwrap();

    let context = DicomAnnotationContext::from_source(&source).unwrap();
    assert_eq!(
        context.optical_path_identifiers(),
        &["BRIGHTFIELD", "FLUORESCENCE"]
    );
}

#[test]
fn context_attribute_helpers_accept_only_exact_finite_shapes() {
    let mut object = empty_file_object();
    assert!(required_string(&object, tags::SOP_INSTANCE_UID).is_err());
    assert!(required_u32(&object, tags::NUMBER_OF_FRAMES).is_err());
    assert!(required_u16(&object, tags::ROWS).is_err());
    assert!(optional_pair(&object, tags::PIXEL_SPACING).is_none());
    assert!(optional_positive_float(&object, tags::SLICE_THICKNESS).is_none());
    assert!(optional_six(&object, tags::IMAGE_ORIENTATION_SLIDE).is_none());
    assert!(optional_origin(&object).is_none());
    assert!(optional_u32(&object, tags::NUMBER_OF_FRAMES).is_none());

    object.put(DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, "2.25.10"));
    object.put(DataElement::new(
        tags::NUMBER_OF_FRAMES,
        VR::UL,
        PrimitiveValue::from(5_u32),
    ));
    object.put(DataElement::new(
        tags::ROWS,
        VR::US,
        PrimitiveValue::from(4_u16),
    ));
    object.put(DataElement::new(
        tags::PIXEL_SPACING,
        VR::DS,
        dicom_value!(F64, [0.5, 0.25]),
    ));
    object.put(DataElement::new(tags::SLICE_THICKNESS, VR::DS, "0.001"));
    object.put(DataElement::new(
        tags::IMAGE_ORIENTATION_SLIDE,
        VR::DS,
        dicom_value!(F64, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
    ));
    let mut origin = InMemDicomObject::new_empty();
    origin.put(DataElement::new(
        tags::X_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        "1.5",
    ));
    origin.put(DataElement::new(
        tags::Y_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        "2.5",
    ));
    origin.put(DataElement::new(
        tags::Z_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        "3.5",
    ));
    object.put(sequence(
        tags::TOTAL_PIXEL_MATRIX_ORIGIN_SEQUENCE,
        vec![origin],
    ));

    assert_eq!(
        required_string(&object, tags::SOP_INSTANCE_UID).unwrap(),
        "2.25.10"
    );
    assert_eq!(required_u32(&object, tags::NUMBER_OF_FRAMES).unwrap(), 5);
    assert_eq!(required_u16(&object, tags::ROWS).unwrap(), 4);
    assert_eq!(
        optional_pair(&object, tags::PIXEL_SPACING),
        Some([0.5, 0.25])
    );
    assert_eq!(
        optional_positive_float(&object, tags::SLICE_THICKNESS),
        Some(0.001)
    );
    assert_eq!(
        optional_six(&object, tags::IMAGE_ORIENTATION_SLIDE),
        Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
    );
    assert_eq!(optional_origin(&object), Some([1.5, 2.5, 3.5]));
    assert_eq!(optional_u32(&object, tags::NUMBER_OF_FRAMES), Some(5));

    object.put(DataElement::new(
        tags::PIXEL_SPACING,
        VR::DS,
        dicom_value!(F64, [0.0, 0.25]),
    ));
    object.put(DataElement::new(tags::SLICE_THICKNESS, VR::DS, "-1"));
    object.put(DataElement::new(
        tags::IMAGE_ORIENTATION_SLIDE,
        VR::DS,
        dicom_value!(F64, [1.0, 0.0]),
    ));
    assert!(optional_pair(&object, tags::PIXEL_SPACING).is_none());
    assert!(optional_positive_float(&object, tags::SLICE_THICKNESS).is_none());
    assert!(optional_six(&object, tags::IMAGE_ORIENTATION_SLIDE).is_none());
}

#[test]
fn shared_pixel_measures_and_sparse_frame_origins_validate_nested_sequences() {
    let mut measures = InMemDicomObject::new_empty();
    measures.put(DataElement::new(
        tags::PIXEL_SPACING,
        VR::DS,
        dicom_value!(F64, [0.5, 0.25]),
    ));
    measures.put(DataElement::new(tags::SLICE_THICKNESS, VR::DS, "0.01"));
    let mut shared = InMemDicomObject::new_empty();
    shared.put(sequence(tags::PIXEL_MEASURES_SEQUENCE, vec![measures]));
    let mut object = empty_file_object();
    object.put(sequence(
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![shared],
    ));
    assert_eq!(
        shared_pixel_measures_pair(&object, tags::PIXEL_SPACING),
        Some([0.5, 0.25])
    );
    assert_eq!(
        shared_pixel_measures_float(&object, tags::SLICE_THICKNESS),
        Some(0.01)
    );
    assert!(shared_pixel_measures(&object).is_some());
    assert!(read_sparse_frame_origins(&object, Some("TILED_FULL"))
        .unwrap()
        .is_none());
    assert!(read_sparse_frame_origins(&object, Some("TILED_SPARSE")).is_err());

    object.put(DataElement::new(tags::NUMBER_OF_FRAMES, VR::IS, "1"));
    let mut frame = InMemDicomObject::new_empty();
    object.put(sequence(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![frame.clone()],
    ));
    assert!(read_sparse_frame_origins(&object, Some("TILED_SPARSE")).is_err());

    let mut plane = InMemDicomObject::new_empty();
    plane.put(DataElement::new(
        tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(9_i32),
    ));
    frame.put(sequence(
        tags::PLANE_POSITION_SLIDE_SEQUENCE,
        vec![plane.clone()],
    ));
    object.put(sequence(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![frame.clone()],
    ));
    assert!(read_sparse_frame_origins(&object, Some("TILED_SPARSE")).is_err());

    plane.put(DataElement::new(
        tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(5_i32),
    ));
    plane
        .take(tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX)
        .unwrap();
    frame.put(sequence(
        tags::PLANE_POSITION_SLIDE_SEQUENCE,
        vec![plane.clone()],
    ));
    object.put(sequence(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![frame.clone()],
    ));
    assert!(read_sparse_frame_origins(&object, Some("TILED_SPARSE")).is_err());

    plane.put(DataElement::new(
        tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(9_i32),
    ));
    frame.put(sequence(tags::PLANE_POSITION_SLIDE_SEQUENCE, vec![plane]));
    object.put(sequence(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![frame],
    ));
    assert_eq!(
        read_sparse_frame_origins(&object, Some("TILED_SPARSE")).unwrap(),
        Some(vec![FrameOrigin { column: 5, row: 9 }])
    );
    assert_eq!(dot([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), 32.0);
}
