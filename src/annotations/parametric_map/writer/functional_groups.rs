use dicom_core::value::PrimitiveValue;
use dicom_core::{DataElement, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use crate::annotations::coded_content::code_item;
use crate::annotations::derived_object::sop_reference_item;
use crate::annotations::dicom_dataset::{put_text, sequence};
use crate::annotations::dicom_value::format_ds;
use crate::{DicomCode, Error, Result};

use super::super::document::{ParametricMapDocument, ValueRange};
use super::super::frame::FrameKey;
use super::quantity::real_world_value_item;

pub(super) fn shared_functional_groups(
    document: &ParametricMapDocument,
) -> Result<InMemDicomObject> {
    let mut item = InMemDicomObject::new_empty();
    let mut measures = InMemDicomObject::new_empty();
    put_text(
        &mut measures,
        tags::PIXEL_SPACING,
        VR::DS,
        &format!(
            "{}\\{}",
            format_ds(document.geometry.pixel_spacing[0]),
            format_ds(document.geometry.pixel_spacing[1])
        ),
    );
    put_text(
        &mut measures,
        tags::SLICE_THICKNESS,
        VR::DS,
        &format_ds(document.geometry.slice_thickness),
    );
    item.put(sequence(tags::PIXEL_MEASURES_SEQUENCE, vec![measures]));
    item.put(sequence(
        tags::PIXEL_VALUE_TRANSFORMATION_SEQUENCE,
        vec![identity_pixel_transform()],
    ));
    item.put(sequence(
        tags::PARAMETRIC_MAP_FRAME_TYPE_SEQUENCE,
        vec![parametric_map_frame_type()],
    ));
    item.put(sequence(
        tags::DERIVATION_IMAGE_SEQUENCE,
        vec![derivation_image(document)?],
    ));
    if document.selected_channels.len() == 1 {
        let channel = document.selected_channels[0];
        item.put(sequence(
            tags::FRAME_VOILUT_SEQUENCE,
            vec![voi_item(
                document.channel_ranges[0],
                &document.profile.channels[channel].name,
            )],
        ));
        item.put(sequence(
            tags::REAL_WORLD_VALUE_MAPPING_SEQUENCE,
            vec![real_world_value_item(document, channel, 1)?],
        ));
    }
    Ok(item)
}

pub(super) fn per_frame_functional_groups(
    document: &ParametricMapDocument,
    frame: FrameKey,
) -> Result<InMemDicomObject> {
    let mut item = InMemDicomObject::new_empty();
    let mut content = InMemDicomObject::new_empty();
    if !document.dense {
        let mut indices = Vec::with_capacity(if document.selected_channels.len() > 1 {
            3
        } else {
            2
        });
        if document.selected_channels.len() > 1 {
            indices.push(frame.channel_ordinal);
        }
        indices.extend([frame.tile_column + 1, frame.tile_row + 1]);
        content.put(DataElement::new(
            tags::DIMENSION_INDEX_VALUES,
            VR::UL,
            PrimitiveValue::U32(indices.into()),
        ));
        item.put(sequence(
            tags::PLANE_POSITION_SLIDE_SEQUENCE,
            vec![plane_position(document, frame)?],
        ));
    }
    item.put(sequence(tags::FRAME_CONTENT_SEQUENCE, vec![content]));
    if document.selected_channels.len() > 1 {
        let ordinal = usize::try_from(frame.channel_ordinal)
            .map_err(|_| Error::InvalidInput("PM channel ordinal does not fit usize".into()))?;
        let range = document.channel_ranges[ordinal - 1];
        let channel = &document.profile.channels[frame.channel];
        item.put(sequence(
            tags::FRAME_VOILUT_SEQUENCE,
            vec![voi_item(range, &channel.name)],
        ));
        item.put(sequence(
            tags::REAL_WORLD_VALUE_MAPPING_SEQUENCE,
            vec![real_world_value_item(
                document,
                frame.channel,
                frame.channel_ordinal,
            )?],
        ));
    }
    Ok(item)
}

fn identity_pixel_transform() -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    put_text(&mut item, tags::RESCALE_INTERCEPT, VR::DS, "0");
    put_text(&mut item, tags::RESCALE_SLOPE, VR::DS, "1");
    put_text(&mut item, tags::RESCALE_TYPE, VR::LO, "US");
    item
}

fn parametric_map_frame_type() -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    put_text(
        &mut item,
        tags::FRAME_TYPE,
        VR::CS,
        "DERIVED\\PRIMARY\\VOLUME\\QUANTITY",
    );
    item
}

fn voi_item(range: ValueRange, name: &str) -> InMemDicomObject {
    let width = range.maximum - range.minimum;
    let (center, width) = if width > 0.0 {
        (range.minimum + width / 2.0, width)
    } else {
        (range.minimum, 1.0)
    };
    let mut item = InMemDicomObject::new_empty();
    put_text(&mut item, tags::WINDOW_CENTER, VR::DS, &format_ds(center));
    put_text(&mut item, tags::WINDOW_WIDTH, VR::DS, &format_ds(width));
    put_text(
        &mut item,
        tags::WINDOW_CENTER_WIDTH_EXPLANATION,
        VR::LO,
        name,
    );
    put_text(&mut item, tags::VOILUT_FUNCTION, VR::CS, "LINEAR_EXACT");
    item
}

fn plane_position(document: &ParametricMapDocument, frame: FrameKey) -> Result<InMemDicomObject> {
    let column = frame.column_position(document.descriptor)?;
    let row = frame.row_position(document.descriptor)?;
    let position = document.geometry.frame_position(column, row);
    let mut item = InMemDicomObject::new_empty();
    item.put(DataElement::new(
        tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(
            i32::try_from(column)
                .map_err(|_| Error::InvalidInput("PM column position exceeds DICOM SL".into()))?,
        ),
    ));
    item.put(DataElement::new(
        tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX,
        VR::SL,
        PrimitiveValue::from(
            i32::try_from(row)
                .map_err(|_| Error::InvalidInput("PM row position exceeds DICOM SL".into()))?,
        ),
    ));
    for (tag, value) in [
        (tags::X_OFFSET_IN_SLIDE_COORDINATE_SYSTEM, position[0]),
        (tags::Y_OFFSET_IN_SLIDE_COORDINATE_SYSTEM, position[1]),
        (tags::Z_OFFSET_IN_SLIDE_COORDINATE_SYSTEM, position[2]),
    ] {
        put_text(&mut item, tag, VR::DS, &format_ds(value));
    }
    Ok(item)
}

fn derivation_image(document: &ParametricMapDocument) -> Result<InMemDicomObject> {
    let mut reference = sop_reference_item(
        document.source.sop_class_uid(),
        document.source.sop_instance_uid(),
    );
    reference.put(sequence(
        tags::PURPOSE_OF_REFERENCE_CODE_SEQUENCE,
        vec![code_item(&DicomCode::new(
            "121322",
            "DCM",
            "Source Image for Image Processing Operation",
        )?)],
    ));
    put_text(
        &mut reference,
        tags::SPATIAL_LOCATIONS_PRESERVED,
        VR::CS,
        "YES",
    );
    let mut derivation = InMemDicomObject::new_empty();
    derivation.put(sequence(
        tags::DERIVATION_CODE_SEQUENCE,
        vec![code_item(&DicomCode::new(
            "MODEL_INFERENCE",
            "99FRAMES",
            "Model inference",
        )?)],
    ));
    derivation.put(sequence(tags::SOURCE_IMAGE_SEQUENCE, vec![reference]));
    Ok(derivation)
}
