use super::*;

fn read_frame_position(
    item: &InMemDicomObject,
    source: &DicomAnnotationContext,
    columns: u16,
    rows: u16,
) -> Result<(u16, u32, u32)> {
    let segment_number = item
        .get(tags::SEGMENT_IDENTIFICATION_SEQUENCE)
        .and_then(|element| element.items())
        .and_then(|items| items.first())
        .and_then(|item| item.get(tags::REFERENCED_SEGMENT_NUMBER))
        .and_then(|element| element.to_int::<u16>().ok())
        .unwrap_or(1);
    let plane = item
        .get(tags::PLANE_POSITION_SLIDE_SEQUENCE)
        .and_then(|element| element.items())
        .and_then(|items| items.first())
        .ok_or_else(|| Error::InvalidInput("SEG frame has no slide position".into()))?;
    let col_position = plane
        .get(tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX)
        .and_then(|element| element.to_int::<u32>().ok())
        .ok_or_else(|| Error::InvalidInput("SEG frame has invalid column position".into()))?;
    let row_position = plane
        .get(tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX)
        .and_then(|element| element.to_int::<u32>().ok())
        .ok_or_else(|| Error::InvalidInput("SEG frame has invalid row position".into()))?;
    if col_position == 0 || row_position == 0 {
        return Err(Error::InvalidInput(
            "SEG frame positions are one-based and cannot be zero".into(),
        ));
    }
    let tile_col = (col_position - 1) / u32::from(columns);
    let tile_row = (row_position - 1) / u32::from(rows);
    let (matrix_width, matrix_height) = source.total_pixel_matrix_dimensions();
    if tile_col * u32::from(columns) >= matrix_width || tile_row * u32::from(rows) >= matrix_height
    {
        return Err(Error::InvalidInput(
            "SEG frame position is outside the open WSI Total Pixel Matrix".into(),
        ));
    }
    Ok((segment_number, tile_col, tile_row))
}

pub(super) fn read_frame_positions(
    object: &InMemDicomObject,
    source: &DicomAnnotationContext,
    columns: u16,
    rows: u16,
    frame_count: usize,
    channel_segment_numbers: Option<&[u16]>,
    missing_message: &str,
) -> Result<Vec<(u16, u32, u32)>> {
    if let Some(frame_items) = object
        .get(tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE)
        .and_then(|element| element.items())
    {
        if frame_items.len() != frame_count {
            return Err(Error::InvalidInput(format!(
                "SEG declares {frame_count} frames but has {} per-frame items",
                frame_items.len()
            )));
        }
        return frame_items
            .iter()
            .map(|item| read_frame_position(item, source, columns, rows))
            .collect();
    }
    if optional_string(object, tags::DIMENSION_ORGANIZATION_TYPE).as_deref() != Some("TILED_FULL") {
        return Err(Error::InvalidInput(missing_message.into()));
    }

    let total_columns = required_u32(object, tags::TOTAL_PIXEL_MATRIX_COLUMNS)?;
    let total_rows = required_u32(object, tags::TOTAL_PIXEL_MATRIX_ROWS)?;
    if total_columns == 0 || total_rows == 0 {
        return Err(Error::InvalidInput(
            "TILED_FULL SEG Total Pixel Matrix dimensions must be positive".into(),
        ));
    }
    let tile_columns = total_columns.div_ceil(u32::from(columns));
    let tile_rows = total_rows.div_ceil(u32::from(rows));
    let tile_columns_usize = usize::try_from(tile_columns)
        .map_err(|_| Error::InvalidInput("TILED_FULL SEG tile count overflows".into()))?;
    let tiles_per_channel = usize::try_from(
        tile_columns
            .checked_mul(tile_rows)
            .ok_or_else(|| Error::InvalidInput("TILED_FULL SEG tile count overflows".into()))?,
    )
    .map_err(|_| Error::InvalidInput("TILED_FULL SEG tile count overflows".into()))?;
    if tiles_per_channel == 0 || !frame_count.is_multiple_of(tiles_per_channel) {
        return Err(Error::InvalidInput(
            "TILED_FULL SEG frame count does not match its dense tile grid".into(),
        ));
    }
    let channel_count = frame_count / tiles_per_channel;
    if let Some(segment_numbers) = channel_segment_numbers {
        if segment_numbers.len() != channel_count {
            return Err(Error::InvalidInput(format!(
                "TILED_FULL SEG has {channel_count} frame channels but {} non-background segments",
                segment_numbers.len()
            )));
        }
    } else if channel_count != 1 {
        return Err(Error::InvalidInput(
            "TILED_FULL LABELMAP must have one dense frame channel".into(),
        ));
    }

    (0..frame_count)
        .map(|frame_index| {
            let channel_index = frame_index / tiles_per_channel;
            let tile_index = frame_index % tiles_per_channel;
            let tile_col = u32::try_from(tile_index % tile_columns_usize)
                .map_err(|_| Error::InvalidInput("SEG tile column overflows".into()))?;
            let tile_row = u32::try_from(tile_index / tile_columns_usize)
                .map_err(|_| Error::InvalidInput("SEG tile row overflows".into()))?;
            let segment_number =
                channel_segment_numbers.map_or(1, |numbers| numbers[channel_index]);
            let (matrix_width, matrix_height) = source.total_pixel_matrix_dimensions();
            if tile_col * u32::from(columns) >= matrix_width
                || tile_row * u32::from(rows) >= matrix_height
            {
                return Err(Error::InvalidInput(
                    "TILED_FULL SEG frame position is outside the open WSI Total Pixel Matrix"
                        .into(),
                ));
            }
            Ok((segment_number, tile_col, tile_row))
        })
        .collect()
}

pub(in crate::annotations::seg) fn validate_source_reference(
    object: &InMemDicomObject,
    source: &DicomAnnotationContext,
) -> Result<()> {
    if optional_string(object, tags::STUDY_INSTANCE_UID).as_deref()
        != Some(source.study_instance_uid())
    {
        return Err(Error::InvalidInput(
            "SEG Study Instance UID does not match the open WSI".into(),
        ));
    }
    if let (Some(expected), Some(actual)) = (
        source.frame_of_reference_uid(),
        optional_string(object, tags::FRAME_OF_REFERENCE_UID),
    ) {
        if actual != expected {
            return Err(Error::InvalidInput(
                "SEG Frame of Reference UID does not match the open WSI".into(),
            ));
        }
    }
    let referenced = sequence_references_source(
        object,
        tags::REFERENCED_SERIES_SEQUENCE,
        tags::REFERENCED_INSTANCE_SEQUENCE,
        source,
    ) || functional_groups_reference_source(
        object,
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        source,
    ) || functional_groups_reference_source(
        object,
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        source,
    );
    if !referenced {
        return Err(Error::InvalidInput(format!(
            "SEG does not reference the open WSI SOP Instance {}",
            source.sop_instance_uid()
        )));
    }
    Ok(())
}

fn sequence_references_source(
    object: &InMemDicomObject,
    outer_tag: Tag,
    inner_tag: Tag,
    source: &DicomAnnotationContext,
) -> bool {
    object
        .get(outer_tag)
        .and_then(|element| element.items())
        .into_iter()
        .flatten()
        .flat_map(|item| {
            item.get(inner_tag)
                .and_then(|element| element.items())
                .unwrap_or_default()
        })
        .any(|reference| sop_reference_matches(reference, source))
}

fn functional_groups_reference_source(
    object: &InMemDicomObject,
    functional_groups_tag: Tag,
    source: &DicomAnnotationContext,
) -> bool {
    object
        .get(functional_groups_tag)
        .and_then(|element| element.items())
        .into_iter()
        .flatten()
        .flat_map(|functional_group| {
            functional_group
                .get(tags::DERIVATION_IMAGE_SEQUENCE)
                .and_then(|element| element.items())
                .unwrap_or_default()
        })
        .flat_map(|derivation| {
            derivation
                .get(tags::SOURCE_IMAGE_SEQUENCE)
                .and_then(|element| element.items())
                .unwrap_or_default()
        })
        .any(|reference| sop_reference_matches(reference, source))
}

fn sop_reference_matches(reference: &InMemDicomObject, source: &DicomAnnotationContext) -> bool {
    optional_string(reference, tags::REFERENCED_SOP_INSTANCE_UID).as_deref()
        == Some(source.sop_instance_uid())
        && optional_string(reference, tags::REFERENCED_SOP_CLASS_UID).as_deref()
            == Some(source.sop_class_uid())
}

pub(in crate::annotations::seg) fn segments_overlap(frames: &[BinarySegmentationFrame]) -> bool {
    for (index, frame) in frames.iter().enumerate() {
        if frames[index + 1..].iter().any(|other| {
            frame.segment_number != other.segment_number
                && frame.tile_col == other.tile_col
                && frame.tile_row == other.tile_row
                && frame.mask.iter().zip(&other.mask).any(|(a, b)| *a && *b)
        }) {
            return true;
        }
    }
    false
}
