use dicom_core::value::PrimitiveValue;
use dicom_core::{DataElement, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use crate::annotations::dicom_dataset::{dimension_index_item, put_text, sequence};
use crate::annotations::dicom_value::format_ds;
use crate::{Error, Result};

use super::super::document::ParametricMapDocument;
use super::super::frame::PixelPrecision;
use super::InstanceSpec;

pub(super) fn add_general_image_attributes(
    document: &ParametricMapDocument,
    spec: &InstanceSpec<'_>,
    object: &mut InMemDicomObject,
) -> Result<()> {
    for (tag, vr, value) in [
        (
            tags::IMAGE_TYPE,
            VR::CS,
            "DERIVED\\PRIMARY\\VOLUME\\QUANTITY",
        ),
        (tags::PIXEL_PRESENTATION, VR::CS, "MONOCHROME"),
        (tags::CONTENT_LABEL, VR::CS, "PATHOLOGY_PM"),
        (
            tags::CONTENT_DESCRIPTION,
            VR::LO,
            "Pathology model parametric map",
        ),
        (tags::BURNED_IN_ANNOTATION, VR::CS, "NO"),
        (tags::RECOGNIZABLE_VISUAL_FEATURES, VR::CS, "NO"),
        (tags::CONTENT_QUALIFICATION, VR::CS, "RESEARCH"),
        (tags::PRESENTATION_LUT_SHAPE, VR::CS, "IDENTITY"),
    ] {
        put_text(object, tag, vr, value);
    }
    put_text(object, tags::CONTENT_DATE, VR::DA, spec.content_date);
    put_text(object, tags::CONTENT_TIME, VR::TM, spec.content_time);
    put_text(
        object,
        tags::NUMBER_OF_FRAMES,
        VR::IS,
        &spec.frames.len().to_string(),
    );
    let source_metadata = document.source.source_metadata()?;
    let lossy = source_metadata
        .get(tags::LOSSY_IMAGE_COMPRESSION)
        .and_then(|element| element.to_str().ok())
        .is_some_and(|value| value.trim() == "01");
    put_text(
        object,
        tags::LOSSY_IMAGE_COMPRESSION,
        VR::CS,
        if lossy { "01" } else { "00" },
    );
    if lossy {
        for tag in [
            tags::LOSSY_IMAGE_COMPRESSION_RATIO,
            tags::LOSSY_IMAGE_COMPRESSION_METHOD,
        ] {
            if let Some(element) = source_metadata.get(tag) {
                object.put(element.clone());
            }
        }
    }
    if let Some(concatenation) = spec.concatenation {
        put_text(object, tags::CONCATENATION_UID, VR::UI, concatenation.uid);
        put_text(
            object,
            tags::SOP_INSTANCE_UID_OF_CONCATENATION_SOURCE,
            VR::UI,
            concatenation.source_sop_instance_uid,
        );
        object.put(DataElement::new(
            tags::IN_CONCATENATION_NUMBER,
            VR::US,
            PrimitiveValue::from(concatenation.number),
        ));
        object.put(DataElement::new(
            tags::IN_CONCATENATION_TOTAL_NUMBER,
            VR::US,
            PrimitiveValue::from(concatenation.total),
        ));
        object.put(DataElement::new(
            tags::CONCATENATION_FRAME_OFFSET_NUMBER,
            VR::UL,
            PrimitiveValue::from(spec.frame_offset),
        ));
    }
    Ok(())
}

pub(super) fn add_slide_geometry(
    document: &ParametricMapDocument,
    object: &mut InMemDicomObject,
) -> Result<()> {
    let frame_of_reference_uid = document
        .source
        .frame_of_reference_uid()
        .ok_or_else(|| Error::InvalidInput("PM source has no Frame of Reference UID".into()))?;
    put_text(
        object,
        tags::FRAME_OF_REFERENCE_UID,
        VR::UI,
        frame_of_reference_uid,
    );
    let source_metadata = document.source.source_metadata()?;
    let position_reference = source_metadata
        .get(tags::POSITION_REFERENCE_INDICATOR)
        .and_then(|element| element.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "SLIDE_CORNER".into());
    put_text(
        object,
        tags::POSITION_REFERENCE_INDICATOR,
        VR::LO,
        &position_reference,
    );
    put_text(
        object,
        tags::IMAGE_ORIENTATION_SLIDE,
        VR::DS,
        &document
            .geometry
            .orientation
            .iter()
            .map(|value| format_ds(*value))
            .collect::<Vec<_>>()
            .join("\\"),
    );
    let mut origin = InMemDicomObject::new_empty();
    put_text(
        &mut origin,
        tags::X_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        &format_ds(document.geometry.origin[0]),
    );
    put_text(
        &mut origin,
        tags::Y_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        &format_ds(document.geometry.origin[1]),
    );
    object.put(sequence(
        tags::TOTAL_PIXEL_MATRIX_ORIGIN_SEQUENCE,
        vec![origin],
    ));
    for (tag, value) in [
        (tags::TOTAL_PIXEL_MATRIX_ROWS, document.descriptor.height),
        (tags::TOTAL_PIXEL_MATRIX_COLUMNS, document.descriptor.width),
        (tags::TOTAL_PIXEL_MATRIX_FOCAL_PLANES, 1),
    ] {
        object.put(DataElement::new(tag, VR::UL, PrimitiveValue::from(value)));
    }
    Ok(())
}

pub(super) fn add_pixel_attributes(
    document: &ParametricMapDocument,
    object: &mut InMemDicomObject,
) -> Result<()> {
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
        PrimitiveValue::from(
            u16::try_from(document.descriptor.tile_height)
                .map_err(|_| Error::InvalidInput("PM tile rows exceed DICOM US".into()))?,
        ),
    ));
    object.put(DataElement::new(
        tags::COLUMNS,
        VR::US,
        PrimitiveValue::from(
            u16::try_from(document.descriptor.tile_width)
                .map_err(|_| Error::InvalidInput("PM tile columns exceed DICOM US".into()))?,
        ),
    ));
    let bits = match document.precision {
        PixelPrecision::Float32 => 32_u16,
        PixelPrecision::Float64 => 64_u16,
    };
    object.put(DataElement::new(
        tags::BITS_ALLOCATED,
        VR::US,
        PrimitiveValue::from(bits),
    ));
    match document.precision {
        PixelPrecision::Float32 => {
            let value = PrimitiveValue::F32(vec![super::super::source::CANONICAL_NAN_F32].into());
            object.put(DataElement::new(
                tags::FLOAT_PIXEL_PADDING_VALUE,
                VR::FL,
                value.clone(),
            ));
            object.put(DataElement::new(
                tags::FLOAT_PIXEL_PADDING_RANGE_LIMIT,
                VR::FL,
                value,
            ));
        }
        PixelPrecision::Float64 => {
            let value = PrimitiveValue::F64(vec![super::super::source::CANONICAL_NAN_F64].into());
            object.put(DataElement::new(
                tags::DOUBLE_FLOAT_PIXEL_PADDING_VALUE,
                VR::FD,
                value.clone(),
            ));
            object.put(DataElement::new(
                tags::DOUBLE_FLOAT_PIXEL_PADDING_RANGE_LIMIT,
                VR::FD,
                value,
            ));
        }
    }
    Ok(())
}

pub(super) fn add_dimensions(
    document: &ParametricMapDocument,
    spec: &InstanceSpec<'_>,
    object: &mut InMemDicomObject,
) {
    put_text(
        object,
        tags::DIMENSION_ORGANIZATION_TYPE,
        VR::CS,
        if document.dense {
            "TILED_FULL"
        } else {
            "TILED_SPARSE"
        },
    );
    let mut organization = InMemDicomObject::new_empty();
    put_text(
        &mut organization,
        tags::DIMENSION_ORGANIZATION_UID,
        VR::UI,
        spec.dimension_organization_uid,
    );
    object.put(sequence(
        tags::DIMENSION_ORGANIZATION_SEQUENCE,
        vec![organization],
    ));
    if document.dense {
        return;
    }
    let mut dimensions = Vec::new();
    if document.selected_channels.len() > 1 {
        dimensions.push(dimension_index_item(
            spec.dimension_organization_uid,
            tags::QUANTITY_DEFINITION_SEQUENCE,
            tags::REAL_WORLD_VALUE_MAPPING_SEQUENCE,
            "Quantity",
        ));
    }
    dimensions.push(dimension_index_item(
        spec.dimension_organization_uid,
        tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        tags::PLANE_POSITION_SLIDE_SEQUENCE,
        "Column position",
    ));
    dimensions.push(dimension_index_item(
        spec.dimension_organization_uid,
        tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        tags::PLANE_POSITION_SLIDE_SEQUENCE,
        "Row position",
    ));
    object.put(sequence(tags::DIMENSION_INDEX_SEQUENCE, dimensions));
}
