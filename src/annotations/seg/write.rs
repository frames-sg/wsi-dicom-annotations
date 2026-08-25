use dicom_core::value::PrimitiveValue;
use dicom_core::{DataElement, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use super::read::segments_overlap;
use super::{BinarySegmentationFrame, SegmentationDocument, SegmentationSegment};
use crate::annotations::coded_content::{code_item, write_algorithm};
use crate::annotations::context::DicomAnnotationContext;
use crate::annotations::derived_object::sop_reference_item;
use crate::annotations::dicom_dataset::{
    dicom_now, dimension_index_item, new_dicom_uid, put_text, sequence,
};
use crate::annotations::dicom_value::format_ds;
use crate::annotations::model::{AlgorithmIdentification, DicomCode, GenerationType};
use crate::{Error, Result};

pub(super) fn add_segmentation_attributes(
    object: &mut InMemDicomObject,
    document: &SegmentationDocument,
    frames: &[BinarySegmentationFrame],
) -> Result<()> {
    let source = document.source();
    let source_metadata = source.source_metadata()?;
    let (matrix_width, matrix_height) = source.total_pixel_matrix_dimensions();
    let (tile_width, tile_height) = source.tile_dimensions();
    let (date, time) = dicom_now();
    put_text(object, tags::IMAGE_TYPE, VR::CS, "DERIVED\\PRIMARY");
    put_text(object, tags::CONTENT_DATE, VR::DA, &date);
    put_text(object, tags::CONTENT_TIME, VR::TM, &time);
    put_text(
        object,
        tags::CONTENT_LABEL,
        VR::CS,
        document.content_label(),
    );
    put_text(
        object,
        tags::CONTENT_DESCRIPTION,
        VR::LO,
        document.content_description(),
    );
    if let Some(creator) = document.content_creator_name() {
        put_text(object, tags::CONTENT_CREATOR_NAME, VR::PN, creator);
    }
    put_text(object, tags::LOSSY_IMAGE_COMPRESSION, VR::CS, "00");
    put_text(
        object,
        tags::DIMENSION_ORGANIZATION_TYPE,
        VR::CS,
        "TILED_SPARSE",
    );
    if let Some(frame_of_reference_uid) = source.frame_of_reference_uid() {
        put_text(
            object,
            tags::FRAME_OF_REFERENCE_UID,
            VR::UI,
            frame_of_reference_uid,
        );
        if let Some(element) = source_metadata.get(tags::POSITION_REFERENCE_INDICATOR) {
            object.put(element.clone());
        } else {
            put_text(
                object,
                tags::POSITION_REFERENCE_INDICATOR,
                VR::LO,
                "SLIDE_CORNER",
            );
        }
    }
    for tag in [
        tags::TOTAL_PIXEL_MATRIX_ORIGIN_SEQUENCE,
        tags::IMAGE_ORIENTATION_SLIDE,
    ] {
        if let Some(element) = source_metadata.get(tag) {
            object.put(element.clone());
        }
    }
    object.put(DataElement::new(
        tags::TOTAL_PIXEL_MATRIX_COLUMNS,
        VR::UL,
        PrimitiveValue::from(matrix_width),
    ));
    object.put(DataElement::new(
        tags::TOTAL_PIXEL_MATRIX_ROWS,
        VR::UL,
        PrimitiveValue::from(matrix_height),
    ));
    object.put(DataElement::new(
        tags::SAMPLES_PER_PIXEL,
        VR::US,
        PrimitiveValue::from(1_u16),
    ));
    put_text(
        object,
        tags::PHOTOMETRIC_INTERPRETATION,
        VR::CS,
        "MONOCHROME2",
    );
    object.put(DataElement::new(
        tags::ROWS,
        VR::US,
        PrimitiveValue::from(tile_height),
    ));
    object.put(DataElement::new(
        tags::COLUMNS,
        VR::US,
        PrimitiveValue::from(tile_width),
    ));
    for (tag, value) in [
        (tags::BITS_ALLOCATED, 1_u16),
        (tags::BITS_STORED, 1_u16),
        (tags::HIGH_BIT, 0_u16),
        (tags::PIXEL_REPRESENTATION, 0_u16),
    ] {
        object.put(DataElement::new(tag, VR::US, PrimitiveValue::from(value)));
    }
    put_text(object, tags::SEGMENTATION_TYPE, VR::CS, "BINARY");
    put_text(
        object,
        tags::SEGMENTS_OVERLAP,
        VR::CS,
        if segments_overlap(frames) {
            "YES"
        } else {
            "NO"
        },
    );
    object.put(sequence(
        tags::SEGMENT_SEQUENCE,
        document
            .segments()
            .iter()
            .enumerate()
            .map(write_segment)
            .collect::<Result<Vec<_>>>()?,
    ));
    put_text(
        object,
        tags::NUMBER_OF_FRAMES,
        VR::IS,
        &frames.len().to_string(),
    );
    let dimension_uid = new_dicom_uid();
    let mut organization = InMemDicomObject::new_empty();
    put_text(
        &mut organization,
        tags::DIMENSION_ORGANIZATION_UID,
        VR::UI,
        &dimension_uid,
    );
    object.put(sequence(
        tags::DIMENSION_ORGANIZATION_SEQUENCE,
        vec![organization],
    ));
    object.put(sequence(
        tags::DIMENSION_INDEX_SEQUENCE,
        vec![
            dimension_index_item(
                &dimension_uid,
                tags::REFERENCED_SEGMENT_NUMBER,
                tags::SEGMENT_IDENTIFICATION_SEQUENCE,
                "Segment",
            ),
            dimension_index_item(
                &dimension_uid,
                tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
                tags::PLANE_POSITION_SLIDE_SEQUENCE,
                "Column position",
            ),
            dimension_index_item(
                &dimension_uid,
                tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
                tags::PLANE_POSITION_SLIDE_SEQUENCE,
                "Row position",
            ),
        ],
    ));
    object.put(sequence(
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![shared_functional_groups(source)?],
    ));
    object.put(sequence(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        frames
            .iter()
            .map(|frame| per_frame_functional_groups(source, frame))
            .collect::<Result<Vec<_>>>()?,
    ));
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        PrimitiveValue::from(pack_binary_frames(frames)),
    ));
    Ok(())
}

fn write_segment((index, segment): (usize, &SegmentationSegment)) -> Result<InMemDicomObject> {
    let number = segment.source_segment_number.unwrap_or(
        u16::try_from(index + 1)
            .map_err(|_| Error::InvalidInput("segment number exceeds US range".into()))?,
    );
    let mut item = InMemDicomObject::new_empty();
    item.put(DataElement::new(
        tags::SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(number),
    ));
    put_text(&mut item, tags::SEGMENT_LABEL, VR::LO, segment.label());
    if !segment.description().is_empty() {
        put_text(
            &mut item,
            tags::SEGMENT_DESCRIPTION,
            VR::ST,
            segment.description(),
        );
    }
    let generation_type = if segment.algorithms().is_empty() {
        GenerationType::Manual
    } else {
        segment.generation_type()
    };
    put_text(
        &mut item,
        tags::SEGMENT_ALGORITHM_TYPE,
        VR::CS,
        generation_type.dicom_value(),
    );
    if !segment.algorithms().is_empty() {
        put_text(
            &mut item,
            tags::SEGMENT_ALGORITHM_NAME,
            VR::LO,
            &segment
                .algorithms()
                .iter()
                .map(AlgorithmIdentification::name)
                .collect::<Vec<_>>()
                .join("\\"),
        );
        item.put(sequence(
            tags::SEGMENTATION_ALGORITHM_IDENTIFICATION_SEQUENCE,
            segment.algorithms().iter().map(write_algorithm).collect(),
        ));
    }
    item.put(sequence(
        tags::SEGMENTED_PROPERTY_CATEGORY_CODE_SEQUENCE,
        vec![code_item(segment.category())],
    ));
    item.put(sequence(
        tags::SEGMENTED_PROPERTY_TYPE_CODE_SEQUENCE,
        vec![code_item(segment.property_type())],
    ));
    if !segment.property_type_modifiers().is_empty() {
        item.put(sequence(
            tags::SEGMENTED_PROPERTY_TYPE_MODIFIER_CODE_SEQUENCE,
            segment
                .property_type_modifiers()
                .iter()
                .map(code_item)
                .collect(),
        ));
    }
    if let (Some(tracking_id), Some(tracking_uid)) = (segment.tracking_id(), segment.tracking_uid())
    {
        put_text(&mut item, tags::TRACKING_ID, VR::UT, tracking_id);
        put_text(&mut item, tags::TRACKING_UID, VR::UI, tracking_uid);
    }
    if !segment.anatomic_regions().is_empty() {
        item.put(sequence(
            tags::ANATOMIC_REGION_SEQUENCE,
            segment.anatomic_regions().iter().map(code_item).collect(),
        ));
    }
    if !segment.primary_anatomic_structures().is_empty() {
        item.put(sequence(
            tags::PRIMARY_ANATOMIC_STRUCTURE_SEQUENCE,
            segment
                .primary_anatomic_structures()
                .iter()
                .map(code_item)
                .collect(),
        ));
    }
    item.put(DataElement::new(
        tags::RECOMMENDED_DISPLAY_CIE_LAB_VALUE,
        VR::US,
        PrimitiveValue::from(segment.recommended_display_cielab()),
    ));
    Ok(item)
}

fn shared_functional_groups(source: &DicomAnnotationContext) -> Result<InMemDicomObject> {
    let mut item = InMemDicomObject::new_empty();
    let spacing = source.pixel_spacing().ok_or_else(|| {
        Error::InvalidInput("WSI source has no Pixel Spacing required for SEG".into())
    })?;
    let slice_thickness = source.slice_thickness().ok_or_else(|| {
        Error::InvalidInput(
            "WSI source has no Slice Thickness/depth of field required for SEG".into(),
        )
    })?;
    let mut measures = InMemDicomObject::new_empty();
    put_text(
        &mut measures,
        tags::PIXEL_SPACING,
        VR::DS,
        &format!("{}\\{}", spacing[0], spacing[1]),
    );
    put_text(
        &mut measures,
        tags::SLICE_THICKNESS,
        VR::DS,
        &format_ds(slice_thickness),
    );
    item.put(sequence(tags::PIXEL_MEASURES_SEQUENCE, vec![measures]));

    let mut source_reference =
        sop_reference_item(source.sop_class_uid(), source.sop_instance_uid());
    source_reference.put(sequence(
        tags::PURPOSE_OF_REFERENCE_CODE_SEQUENCE,
        vec![code_item(&DicomCode::new(
            "121322",
            "DCM",
            "Source image for image processing operation",
        )?)],
    ));
    put_text(
        &mut source_reference,
        tags::SPATIAL_LOCATIONS_PRESERVED,
        VR::CS,
        "YES",
    );
    let mut derivation = InMemDicomObject::new_empty();
    derivation.put(sequence(
        tags::DERIVATION_CODE_SEQUENCE,
        vec![code_item(&DicomCode::new("113076", "DCM", "Segmentation")?)],
    ));
    derivation.put(sequence(
        tags::SOURCE_IMAGE_SEQUENCE,
        vec![source_reference],
    ));
    item.put(sequence(tags::DERIVATION_IMAGE_SEQUENCE, vec![derivation]));
    Ok(item)
}

fn per_frame_functional_groups(
    source: &DicomAnnotationContext,
    frame: &BinarySegmentationFrame,
) -> Result<InMemDicomObject> {
    let mut item = InMemDicomObject::new_empty();
    let mut segment = InMemDicomObject::new_empty();
    segment.put(DataElement::new(
        tags::REFERENCED_SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(frame.segment_number),
    ));
    item.put(sequence(
        tags::SEGMENT_IDENTIFICATION_SEQUENCE,
        vec![segment],
    ));
    let col_position = frame.tile_col * u32::from(frame.width) + 1;
    let row_position = frame.tile_row * u32::from(frame.height) + 1;
    let (x, y, z) = slide_offsets(source, col_position, row_position)?;
    let mut plane = InMemDicomObject::new_empty();
    plane.put(DataElement::new(
        tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(i32::try_from(col_position).map_err(|_| {
            Error::InvalidInput("SEG column position exceeds DICOM SL range".into())
        })?),
    ));
    plane.put(DataElement::new(
        tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(
            i32::try_from(row_position).map_err(|_| {
                Error::InvalidInput("SEG row position exceeds DICOM SL range".into())
            })?,
        ),
    ));
    put_text(
        &mut plane,
        tags::X_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        &format_ds(x),
    );
    put_text(
        &mut plane,
        tags::Y_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        &format_ds(y),
    );
    put_text(
        &mut plane,
        tags::Z_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        &format_ds(z),
    );
    item.put(sequence(tags::PLANE_POSITION_SLIDE_SEQUENCE, vec![plane]));
    let mut content = InMemDicomObject::new_empty();
    content.put(DataElement::new(
        tags::DIMENSION_INDEX_VALUES,
        VR::UL,
        PrimitiveValue::U32(
            vec![
                u32::from(frame.segment_number),
                frame.tile_col + 1,
                frame.tile_row + 1,
            ]
            .into(),
        ),
    ));
    item.put(sequence(tags::FRAME_CONTENT_SEQUENCE, vec![content]));
    Ok(item)
}

fn slide_offsets(
    source: &DicomAnnotationContext,
    col_position: u32,
    row_position: u32,
) -> Result<(f64, f64, f64)> {
    let origin = source.total_pixel_matrix_origin().ok_or_else(|| {
        Error::InvalidInput("WSI source has no Total Pixel Matrix Origin for SEG".into())
    })?;
    let orientation = source.image_orientation_slide().ok_or_else(|| {
        Error::InvalidInput("WSI source has no Image Orientation (Slide) for SEG".into())
    })?;
    let spacing = source
        .pixel_spacing()
        .ok_or_else(|| Error::InvalidInput("WSI source has no Pixel Spacing for SEG".into()))?;
    let col_distance = f64::from(col_position - 1) * spacing[1];
    let row_distance = f64::from(row_position - 1) * spacing[0];
    Ok((
        origin[0] + col_distance * orientation[0] + row_distance * orientation[3],
        origin[1] + col_distance * orientation[1] + row_distance * orientation[4],
        origin[2],
    ))
}

fn pack_binary_frames(frames: &[BinarySegmentationFrame]) -> Vec<u8> {
    let bit_count = frames.iter().map(|frame| frame.mask.len()).sum::<usize>();
    let mut bytes = vec![0_u8; bit_count.div_ceil(8)];
    let mut bit_offset = 0_usize;
    for frame in frames {
        for value in &frame.mask {
            if *value {
                bytes[bit_offset / 8] |= 1 << (bit_offset % 8);
            }
            bit_offset += 1;
        }
    }
    if bytes.len() % 2 == 1 {
        bytes.push(0);
    }
    bytes
}
