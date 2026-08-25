use std::path::Path;

use dicom_core::value::{DataSetSequence, PrimitiveValue, Value};
use dicom_core::{DataElement, Length, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

use crate::AnnotationGraphicType;

pub(crate) type NativeAnnGroup = (AnnotationGraphicType, Vec<f64>, Vec<u32>, Option<Vec<f64>>);

pub(crate) fn write_native_ann(
    path: &Path,
    coordinate_type: &str,
    pixel_origin: Option<&str>,
    referenced_frame: Option<u32>,
    groups: &[NativeAnnGroup],
) {
    const SOP_UID: &str = "1.2.826.0.1.3680043.10.777.301";
    const SERIES_UID: &str = "1.2.826.0.1.3680043.10.777.302";
    const STUDY_UID: &str = "1.2.826.0.1.3680043.10.777.103";
    const FOR_UID: &str = "1.2.826.0.1.3680043.10.777.104";
    const SOURCE_SOP_UID: &str = "1.2.826.0.1.3680043.10.777.101";
    let sequence = |tag, items| {
        DataElement::new(
            tag,
            VR::SQ,
            Value::from(DataSetSequence::new(items, Length::UNDEFINED)),
        )
    };
    let code_item = |value: &'static str, meaning: &'static str| {
        let mut item = InMemDicomObject::new_empty();
        item.put(DataElement::new(tags::CODE_VALUE, VR::SH, value));
        item.put(DataElement::new(
            tags::CODING_SCHEME_DESIGNATOR,
            VR::SH,
            "99FRAMES",
        ));
        item.put(DataElement::new(tags::CODE_MEANING, VR::LO, meaning));
        item
    };
    let group_items = groups
        .iter()
        .enumerate()
        .map(|(index, (graphic_type, coordinates, indices, common_z))| {
            let dimensions = if coordinate_type == "3D" && common_z.is_none() {
                3
            } else {
                2
            };
            let annotation_count = match graphic_type {
                AnnotationGraphicType::Point => coordinates.len() / dimensions,
                AnnotationGraphicType::Polygon | AnnotationGraphicType::Polyline => indices.len(),
                AnnotationGraphicType::Ellipse | AnnotationGraphicType::Rectangle => {
                    coordinates.len() / (dimensions * 4)
                }
            };
            let mut item = InMemDicomObject::new_empty();
            item.put(DataElement::new(
                tags::ANNOTATION_GROUP_NUMBER,
                VR::US,
                PrimitiveValue::from(u16::try_from(index + 1).unwrap()),
            ));
            item.put(DataElement::new(
                tags::ANNOTATION_GROUP_UID,
                VR::UI,
                format!("2.25.{}", 400 + index),
            ));
            item.put(DataElement::new(
                tags::ANNOTATION_GROUP_LABEL,
                VR::LO,
                format!("Group {index}"),
            ));
            item.put(DataElement::new(
                tags::ANNOTATION_GROUP_GENERATION_TYPE,
                VR::CS,
                "MANUAL",
            ));
            item.put(sequence(
                tags::ANNOTATION_PROPERTY_CATEGORY_CODE_SEQUENCE,
                vec![code_item("MORPH", "Morphology")],
            ));
            item.put(sequence(
                tags::ANNOTATION_PROPERTY_TYPE_CODE_SEQUENCE,
                vec![code_item("TUMOR", "Tumor")],
            ));
            item.put(DataElement::new(
                tags::NUMBER_OF_ANNOTATIONS,
                VR::UL,
                PrimitiveValue::from(u32::try_from(annotation_count).unwrap()),
            ));
            item.put(DataElement::new(
                tags::GRAPHIC_TYPE,
                VR::CS,
                graphic_type.dicom_value(),
            ));
            item.put(DataElement::new(
                tags::ANNOTATION_APPLIES_TO_ALL_OPTICAL_PATHS,
                VR::CS,
                "YES",
            ));
            if coordinate_type == "3D" {
                item.put(DataElement::new(
                    tags::ANNOTATION_APPLIES_TO_ALL_Z_PLANES,
                    VR::CS,
                    "NO",
                ));
            }
            if let Some(common_z) = common_z {
                item.put(DataElement::new(
                    tags::COMMON_Z_COORDINATE_VALUE,
                    VR::FD,
                    PrimitiveValue::F64(common_z.clone().into()),
                ));
            }
            item.put(DataElement::new(
                tags::DOUBLE_POINT_COORDINATES_DATA,
                VR::OD,
                PrimitiveValue::F64(coordinates.clone().into()),
            ));
            if !indices.is_empty() {
                item.put(DataElement::new(
                    tags::LONG_PRIMITIVE_POINT_INDEX_LIST,
                    VR::OL,
                    PrimitiveValue::U32(indices.clone().into()),
                ));
            }
            item
        })
        .collect();
    let mut object = InMemDicomObject::new_empty();
    for element in [
        DataElement::new(
            tags::SOP_CLASS_UID,
            VR::UI,
            uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE,
        ),
        DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, SOP_UID),
        DataElement::new(tags::STUDY_INSTANCE_UID, VR::UI, STUDY_UID),
        DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, SERIES_UID),
        DataElement::new(tags::SERIES_NUMBER, VR::IS, "1"),
        DataElement::new(tags::MANUFACTURER, VR::LO, "Synthetic Manufacturer"),
        DataElement::new(tags::MANUFACTURER_MODEL_NAME, VR::LO, "Synthetic Model"),
        DataElement::new(tags::DEVICE_SERIAL_NUMBER, VR::LO, "SYNTHETIC"),
        DataElement::new(tags::SOFTWARE_VERSIONS, VR::LO, "1"),
        DataElement::new(tags::CONTENT_LABEL, VR::CS, "TEST_ANNOTATION"),
        DataElement::new(tags::ANNOTATION_COORDINATE_TYPE, VR::CS, coordinate_type),
    ] {
        object.put(element);
    }
    if coordinate_type == "2D" {
        object.put(DataElement::new(
            tags::PIXEL_ORIGIN_INTERPRETATION,
            VR::CS,
            pixel_origin.unwrap(),
        ));
        let mut reference = InMemDicomObject::new_empty();
        reference.put(DataElement::new(
            tags::REFERENCED_SOP_CLASS_UID,
            VR::UI,
            uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE,
        ));
        reference.put(DataElement::new(
            tags::REFERENCED_SOP_INSTANCE_UID,
            VR::UI,
            SOURCE_SOP_UID,
        ));
        if let Some(frame) = referenced_frame {
            reference.put(DataElement::new(
                tags::REFERENCED_FRAME_NUMBER,
                VR::IS,
                frame.to_string(),
            ));
        }
        object.put(sequence(tags::REFERENCED_IMAGE_SEQUENCE, vec![reference]));
    } else {
        object.put(DataElement::new(
            tags::FRAME_OF_REFERENCE_UID,
            VR::UI,
            FOR_UID,
        ));
    }
    object.put(sequence(tags::ANNOTATION_GROUP_SEQUENCE, group_items));
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE)
                .media_storage_sop_instance_uid(SOP_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap()
        .write_to_file(path)
        .unwrap();
}

pub(crate) fn write_fractional_seg(path: &Path, pixels: [u8; 16]) {
    const SOP_UID: &str = "1.2.826.0.1.3680043.10.777.201";
    const SERIES_UID: &str = "1.2.826.0.1.3680043.10.777.202";
    const STUDY_UID: &str = "1.2.826.0.1.3680043.10.777.103";
    const FOR_UID: &str = "1.2.826.0.1.3680043.10.777.104";
    const SOURCE_SOP_UID: &str = "1.2.826.0.1.3680043.10.777.101";
    let code_item = |value: &'static str, meaning: &'static str| {
        let mut item = InMemDicomObject::new_empty();
        item.put(DataElement::new(tags::CODE_VALUE, VR::SH, value));
        item.put(DataElement::new(
            tags::CODING_SCHEME_DESIGNATOR,
            VR::SH,
            "SCT",
        ));
        item.put(DataElement::new(tags::CODE_MEANING, VR::LO, meaning));
        item
    };
    let sequence = |tag, items| {
        DataElement::new(
            tag,
            VR::SQ,
            Value::from(DataSetSequence::new(items, Length::UNDEFINED)),
        )
    };
    let mut segment = InMemDicomObject::new_empty();
    segment.put(DataElement::new(
        tags::SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(1_u16),
    ));
    segment.put(DataElement::new(tags::SEGMENT_LABEL, VR::LO, "Probability"));
    segment.put(sequence(
        tags::SEGMENTED_PROPERTY_CATEGORY_CODE_SEQUENCE,
        vec![code_item("49755003", "Morphologically abnormal structure")],
    ));
    segment.put(sequence(
        tags::SEGMENTED_PROPERTY_TYPE_CODE_SEQUENCE,
        vec![code_item("108369006", "Neoplasm")],
    ));
    segment.put(DataElement::new(
        tags::RECOMMENDED_DISPLAY_CIE_LAB_VALUE,
        VR::US,
        PrimitiveValue::from([48_000_u16, 32_768, 32_768]),
    ));
    let mut segment_identification = InMemDicomObject::new_empty();
    segment_identification.put(DataElement::new(
        tags::REFERENCED_SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(1_u16),
    ));
    let mut plane = InMemDicomObject::new_empty();
    plane.put(DataElement::new(
        tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(1_i32),
    ));
    plane.put(DataElement::new(
        tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(1_i32),
    ));
    let mut frame = InMemDicomObject::new_empty();
    frame.put(sequence(
        tags::SEGMENT_IDENTIFICATION_SEQUENCE,
        vec![segment_identification],
    ));
    frame.put(sequence(tags::PLANE_POSITION_SLIDE_SEQUENCE, vec![plane]));
    let mut source_reference = InMemDicomObject::new_empty();
    source_reference.put(DataElement::new(
        tags::REFERENCED_SOP_CLASS_UID,
        VR::UI,
        uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE,
    ));
    source_reference.put(DataElement::new(
        tags::REFERENCED_SOP_INSTANCE_UID,
        VR::UI,
        SOURCE_SOP_UID,
    ));
    let mut derivation = InMemDicomObject::new_empty();
    derivation.put(sequence(
        tags::SOURCE_IMAGE_SEQUENCE,
        vec![source_reference],
    ));
    let mut shared = InMemDicomObject::new_empty();
    shared.put(sequence(tags::DERIVATION_IMAGE_SEQUENCE, vec![derivation]));
    let mut object = InMemDicomObject::new_empty();
    for element in [
        DataElement::new(tags::SOP_CLASS_UID, VR::UI, uids::SEGMENTATION_STORAGE),
        DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, SOP_UID),
        DataElement::new(tags::STUDY_INSTANCE_UID, VR::UI, STUDY_UID),
        DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, SERIES_UID),
        DataElement::new(tags::FRAME_OF_REFERENCE_UID, VR::UI, FOR_UID),
        DataElement::new(tags::CONTENT_LABEL, VR::CS, "TEST_SEGMENTATION"),
        DataElement::new(tags::SEGMENTATION_TYPE, VR::CS, "FRACTIONAL"),
        DataElement::new(tags::SEGMENTATION_FRACTIONAL_TYPE, VR::CS, "PROBABILITY"),
        DataElement::new(
            tags::MAXIMUM_FRACTIONAL_VALUE,
            VR::US,
            PrimitiveValue::from(255_u16),
        ),
        DataElement::new(tags::ROWS, VR::US, PrimitiveValue::from(4_u16)),
        DataElement::new(tags::COLUMNS, VR::US, PrimitiveValue::from(4_u16)),
        DataElement::new(tags::NUMBER_OF_FRAMES, VR::IS, "1"),
        DataElement::new(tags::BITS_ALLOCATED, VR::US, PrimitiveValue::from(8_u16)),
        DataElement::new(tags::BITS_STORED, VR::US, PrimitiveValue::from(8_u16)),
        DataElement::new(tags::HIGH_BIT, VR::US, PrimitiveValue::from(7_u16)),
    ] {
        object.put(element);
    }
    object.put(sequence(tags::SEGMENT_SEQUENCE, vec![segment]));
    object.put(sequence(
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![shared],
    ));
    object.put(sequence(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![frame],
    ));
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        PrimitiveValue::from(pixels.to_vec()),
    ));
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::SEGMENTATION_STORAGE)
                .media_storage_sop_instance_uid(SOP_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap()
        .write_to_file(path)
        .unwrap();
}

pub(crate) fn write_source_wsi(
    path: &Path,
    width: u32,
    height: u32,
    tile_width: u16,
    tile_height: u16,
) {
    write_source_wsi_with_spacing(path, width, height, tile_width, tile_height, 0.00025);
}

pub(crate) fn write_source_wsi_with_spacing(
    path: &Path,
    width: u32,
    height: u32,
    tile_width: u16,
    tile_height: u16,
    spacing: f64,
) {
    const SOP_UID: &str = "1.2.826.0.1.3680043.10.777.101";
    const SERIES_UID: &str = "1.2.826.0.1.3680043.10.777.102";
    const STUDY_UID: &str = "1.2.826.0.1.3680043.10.777.103";
    const FOR_UID: &str = "1.2.826.0.1.3680043.10.777.104";
    let mut origin = InMemDicomObject::new_empty();
    origin.put(DataElement::new(
        tags::X_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        "0",
    ));
    origin.put(DataElement::new(
        tags::Y_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        "0",
    ));
    let frame_count = width
        .div_ceil(u32::from(tile_width))
        .saturating_mul(height.div_ceil(u32::from(tile_height)));
    let mut object = InMemDicomObject::new_empty();
    for element in [
        DataElement::new(
            tags::SOP_CLASS_UID,
            VR::UI,
            uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE,
        ),
        DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, SOP_UID),
        DataElement::new(tags::STUDY_INSTANCE_UID, VR::UI, STUDY_UID),
        DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, SERIES_UID),
        DataElement::new(tags::FRAME_OF_REFERENCE_UID, VR::UI, FOR_UID),
        DataElement::new(tags::PATIENT_NAME, VR::PN, "Research^Slide"),
        DataElement::new(tags::PATIENT_ID, VR::LO, "R-1"),
        DataElement::new(tags::STUDY_DATE, VR::DA, "20260804"),
        DataElement::new(tags::STUDY_TIME, VR::TM, "120000"),
        DataElement::new(tags::STUDY_ID, VR::SH, "STUDY-1"),
        DataElement::new(tags::ACCESSION_NUMBER, VR::SH, ""),
        DataElement::new(tags::MANUFACTURER, VR::LO, "Frames"),
        DataElement::new(tags::MANUFACTURER_MODEL_NAME, VR::LO, "Synthetic WSI"),
        DataElement::new(tags::DEVICE_SERIAL_NUMBER, VR::LO, "TEST"),
        DataElement::new(tags::SOFTWARE_VERSIONS, VR::LO, "0.1"),
        DataElement::new(tags::DIMENSION_ORGANIZATION_TYPE, VR::CS, "TILED_FULL"),
        DataElement::new(tags::NUMBER_OF_FRAMES, VR::IS, frame_count.to_string()),
        DataElement::new(
            tags::NUMBER_OF_OPTICAL_PATHS,
            VR::UL,
            PrimitiveValue::from(1_u32),
        ),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_FOCAL_PLANES,
            VR::UL,
            PrimitiveValue::from(1_u32),
        ),
        DataElement::new(tags::ROWS, VR::US, PrimitiveValue::from(tile_height)),
        DataElement::new(tags::COLUMNS, VR::US, PrimitiveValue::from(tile_width)),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_ROWS,
            VR::UL,
            PrimitiveValue::from(height),
        ),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_COLUMNS,
            VR::UL,
            PrimitiveValue::from(width),
        ),
        DataElement::new(tags::IMAGE_ORIENTATION_SLIDE, VR::DS, "1\\0\\0\\0\\1\\0"),
        DataElement::new(tags::PIXEL_SPACING, VR::DS, format!("{spacing}\\{spacing}")),
        DataElement::new(tags::SLICE_THICKNESS, VR::DS, "0.001"),
    ] {
        object.put(element);
    }
    object.put(DataElement::new(
        tags::TOTAL_PIXEL_MATRIX_ORIGIN_SEQUENCE,
        VR::SQ,
        Value::from(DataSetSequence::new(vec![origin], Length::UNDEFINED)),
    ));
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        PrimitiveValue::from(vec![
            0_u8;
            usize::from(tile_width) * usize::from(tile_height) * 3
        ]),
    ));
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE)
                .media_storage_sop_instance_uid(SOP_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap()
        .write_to_file(path)
        .unwrap();
}

pub(crate) fn write_sparse_source_wsi(path: &Path) {
    write_source_wsi(path, 8, 8, 4, 4);
    let mut object = dicom_object::open_file(path).unwrap();
    object.put(DataElement::new(
        tags::DIMENSION_ORGANIZATION_TYPE,
        VR::CS,
        "TILED_SPARSE",
    ));
    object.put(DataElement::new(tags::NUMBER_OF_FRAMES, VR::IS, "2"));
    let frames: Vec<_> = [(1_i32, 1_i32), (5, 5)]
        .into_iter()
        .map(|(column, row)| {
            let mut plane = InMemDicomObject::new_empty();
            plane.put(DataElement::new(
                tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
                VR::SL,
                PrimitiveValue::from(column),
            ));
            plane.put(DataElement::new(
                tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
                VR::SL,
                PrimitiveValue::from(row),
            ));
            let mut frame = InMemDicomObject::new_empty();
            frame.put(DataElement::new(
                tags::PLANE_POSITION_SLIDE_SEQUENCE,
                VR::SQ,
                Value::from(DataSetSequence::new(vec![plane], Length::UNDEFINED)),
            ));
            frame
        })
        .collect();
    object.put(DataElement::new(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        VR::SQ,
        Value::from(DataSetSequence::new(frames, Length::UNDEFINED)),
    ));
    object.write_to_file(path).unwrap();
}
