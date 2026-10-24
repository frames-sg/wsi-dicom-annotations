mod layout;

use std::collections::BTreeSet;

use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use super::{BinarySegmentationFrame, FractionalSegmentationFrame, SegmentationSegment};
use crate::annotations::coded_content::{read_algorithms, read_code_at, read_codes_at};
use crate::annotations::context::DicomAnnotationContext;
use crate::annotations::dicom_dataset::{optional_string, required_string, required_u16};
use crate::annotations::model::{
    DiagnosticDisposition, DiagnosticSeverity, FindingSemantics, GenerationType,
    InteroperabilityDiagnostic,
};
use crate::{Error, Result};
use layout::{enforce_pixel_budget, read_frame_positions, FrameLayout};
pub(super) use layout::{segments_overlap, validate_source_reference};

pub(super) fn read_segments(
    object: &InMemDicomObject,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
    allow_background: bool,
) -> Result<Vec<SegmentationSegment>> {
    let items = object
        .get(tags::SEGMENT_SEQUENCE)
        .and_then(|element| element.items())
        .ok_or_else(|| Error::InvalidInput("SEG has no Segment Sequence".into()))?;
    if items.is_empty() || items.len() > usize::from(u16::MAX) {
        return Err(Error::InvalidInput(
            "SEG Segment Sequence has an invalid item count".into(),
        ));
    }
    let segments = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let path = format!("SegmentSequence[{index}]");
            let color = item
                .get(tags::RECOMMENDED_DISPLAY_CIE_LAB_VALUE)
                .and_then(|element| element.to_multi_int::<u16>().ok())
                .filter(|values| values.len() == 3)
                .map_or([0, 0, 0], |values| [values[0], values[1], values[2]]);
            let generation_type = match optional_string(item, tags::SEGMENT_ALGORITHM_TYPE) {
                Some(value) => GenerationType::from_dicom(&value)?,
                None => {
                    diagnostics.push(InteroperabilityDiagnostic::new(
                        "MISSING_SEGMENT_ALGORITHM_TYPE_DEFAULTED",
                        DiagnosticSeverity::Warning,
                        format!("{path}.SegmentAlgorithmType"),
                        DiagnosticDisposition::Normalized,
                        "missing Segment Algorithm Type was interpreted as MANUAL",
                    ));
                    GenerationType::Manual
                }
            };
            let algorithms = read_algorithms(
                item,
                tags::SEGMENTATION_ALGORITHM_IDENTIFICATION_SEQUENCE,
                &format!("{path}.SegmentationAlgorithmIdentificationSequence"),
                diagnostics,
            )?;
            let finding = FindingSemantics::new(
                generation_type,
                algorithms,
                read_code_at(
                    item,
                    tags::SEGMENTED_PROPERTY_CATEGORY_CODE_SEQUENCE,
                    &format!("{path}.SegmentedPropertyCategoryCodeSequence"),
                    diagnostics,
                )?,
                read_code_at(
                    item,
                    tags::SEGMENTED_PROPERTY_TYPE_CODE_SEQUENCE,
                    &format!("{path}.SegmentedPropertyTypeCodeSequence"),
                    diagnostics,
                )?,
                read_codes_at(
                    item,
                    tags::SEGMENTED_PROPERTY_TYPE_MODIFIER_CODE_SEQUENCE,
                    &format!("{path}.SegmentedPropertyTypeModifierCodeSequence"),
                    diagnostics,
                )?,
                read_codes_at(
                    item,
                    tags::ANATOMIC_REGION_SEQUENCE,
                    &format!("{path}.AnatomicRegionSequence"),
                    diagnostics,
                )?,
                read_codes_at(
                    item,
                    tags::PRIMARY_ANATOMIC_STRUCTURE_SEQUENCE,
                    &format!("{path}.PrimaryAnatomicStructureSequence"),
                    diagnostics,
                )?,
                color,
            )?;
            let tracking_id = optional_string(item, tags::TRACKING_ID);
            let tracking_uid = optional_string(item, tags::TRACKING_UID);
            if tracking_id.is_some() != tracking_uid.is_some() {
                diagnostics.push(InteroperabilityDiagnostic::new(
                    "INCOMPLETE_TRACKING_IDENTITY_WOULD_DROP",
                    DiagnosticSeverity::Warning,
                    format!("{path}.TrackingID"),
                    DiagnosticDisposition::WouldDrop,
                    "Tracking ID and Tracking UID must both be present to preserve tracking identity",
                ));
            }
            Ok(SegmentationSegment {
                source_segment_number: Some(required_u16(item, tags::SEGMENT_NUMBER)?),
                label: required_string(item, tags::SEGMENT_LABEL)?,
                description: optional_string(item, tags::SEGMENT_DESCRIPTION).unwrap_or_default(),
                finding,
                tracking_id,
                tracking_uid,
                outer_polygons: Vec::new(),
                exclusion_polygons: Vec::new(),
                component_holes: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut numbers = BTreeSet::new();
    if segments.iter().any(|segment| {
        let number = segment.source_segment_number.unwrap_or(0);
        (!allow_background && number == 0) || !numbers.insert(number)
    }) {
        return Err(Error::InvalidInput(
            "SEG Segment Numbers must be unique and non-zero outside LABELMAP background".into(),
        ));
    }
    Ok(segments)
}

pub(super) fn read_binary_frames(
    object: &InMemDicomObject,
    source: &DicomAnnotationContext,
    segments: &[SegmentationSegment],
) -> Result<Vec<BinarySegmentationFrame>> {
    let layout = FrameLayout::read(object)?;
    let FrameLayout {
        rows,
        columns,
        pixels_per_frame,
        pixel_count,
        ..
    } = layout;
    let segment_numbers = segments
        .iter()
        .filter_map(|segment| segment.source_segment_number)
        .filter(|number| *number != 0)
        .collect::<Vec<_>>();
    let frame_positions = read_frame_positions(
        object,
        source,
        &layout,
        Some(&segment_numbers),
        "sparse SEG has no per-frame positions",
    )?;
    let required_bits = pixel_count;
    let bytes = object
        .get(tags::PIXEL_DATA)
        .ok_or_else(|| Error::InvalidInput("SEG has no Pixel Data".into()))?
        .to_bytes()
        .map_err(|error| Error::InvalidInput(format!("invalid SEG Pixel Data: {error}")))?;
    if bytes.len().saturating_mul(8) < required_bits {
        return Err(Error::InvalidInput("SEG Pixel Data is truncated".into()));
    }
    frame_positions
        .into_iter()
        .enumerate()
        .map(|(frame_index, (segment_number, tile_col, tile_row))| {
            let bit_start = frame_index * pixels_per_frame;
            let mask = (0..pixels_per_frame)
                .map(|index| {
                    let bit = bit_start + index;
                    bytes[bit / 8] & (1 << (bit % 8)) != 0
                })
                .collect();
            Ok(BinarySegmentationFrame {
                segment_number,
                tile_col,
                tile_row,
                width: columns,
                height: rows,
                mask,
            })
        })
        .collect()
}

pub(super) fn read_fractional_frames(
    object: &InMemDicomObject,
    source: &DicomAnnotationContext,
    segments: &[SegmentationSegment],
) -> Result<Vec<FractionalSegmentationFrame>> {
    let layout = FrameLayout::read(object)?;
    let FrameLayout {
        rows,
        columns,
        pixels_per_frame,
        pixel_count,
        ..
    } = layout;
    let segment_numbers = segments
        .iter()
        .filter_map(|segment| segment.source_segment_number)
        .filter(|number| *number != 0)
        .collect::<Vec<_>>();
    let frame_positions = read_frame_positions(
        object,
        source,
        &layout,
        Some(&segment_numbers),
        "fractional SEG has no frame positions",
    )?;
    let bits = required_u16(object, tags::BITS_ALLOCATED)?;
    let sample_bytes = match bits {
        8 => 1_usize,
        16 => 2_usize,
        other => {
            return Err(Error::Unsupported(format!(
                "fractional SEG Bits Allocated {other} is unsupported"
            )))
        }
    };
    let maximum_fractional_value = required_u16(object, tags::MAXIMUM_FRACTIONAL_VALUE)?;
    if maximum_fractional_value == 0 || (bits == 8 && maximum_fractional_value > u16::from(u8::MAX))
    {
        return Err(Error::InvalidInput(
            "fractional SEG has an invalid Maximum Fractional Value".into(),
        ));
    }
    let bytes = object
        .get(tags::PIXEL_DATA)
        .ok_or_else(|| Error::InvalidInput("fractional SEG has no Pixel Data".into()))?
        .to_bytes()
        .map_err(|error| {
            Error::InvalidInput(format!("invalid fractional SEG Pixel Data: {error}"))
        })?;
    let required_bytes = pixel_count
        .checked_mul(sample_bytes)
        .ok_or_else(|| Error::InvalidInput("fractional SEG byte count overflows".into()))?;
    if bytes.len() < required_bytes {
        return Err(Error::InvalidInput(
            "fractional SEG Pixel Data is truncated".into(),
        ));
    }
    frame_positions
        .into_iter()
        .enumerate()
        .map(|(frame_index, (segment_number, tile_col, tile_row))| {
            let first_pixel = frame_index * pixels_per_frame;
            let values = (0..pixels_per_frame)
                .map(|pixel| {
                    let offset = (first_pixel + pixel) * sample_bytes;
                    if sample_bytes == 1 {
                        u16::from(bytes[offset])
                    } else {
                        u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
                    }
                })
                .collect::<Vec<_>>();
            if values.iter().any(|value| *value > maximum_fractional_value) {
                return Err(Error::InvalidInput(
                    "fractional SEG contains a sample above Maximum Fractional Value".into(),
                ));
            }
            Ok(FractionalSegmentationFrame {
                segment_number,
                tile_col,
                tile_row,
                width: columns,
                height: rows,
                maximum_fractional_value,
                values,
            })
        })
        .collect()
}

pub(super) fn read_labelmap_frames(
    object: &InMemDicomObject,
    source: &DicomAnnotationContext,
    segments: &[SegmentationSegment],
) -> Result<Vec<BinarySegmentationFrame>> {
    let layout = FrameLayout::read(object)?;
    let FrameLayout {
        rows,
        columns,
        pixels_per_frame,
        pixel_count,
        ..
    } = layout;
    let frame_positions = read_frame_positions(
        object,
        source,
        &layout,
        None,
        "label-map SEG has no frame positions",
    )?;
    let bits = required_u16(object, tags::BITS_ALLOCATED)?;
    let bytes = object
        .get(tags::PIXEL_DATA)
        .ok_or_else(|| Error::InvalidInput("label-map SEG has no Pixel Data".into()))?
        .to_bytes()
        .map_err(|error| Error::InvalidInput(format!("invalid SEG Pixel Data: {error}")))?;
    let sample_bytes = match bits {
        8 => 1,
        16 => 2,
        other => {
            return Err(Error::Unsupported(format!(
                "label-map SEG Bits Allocated {other} is unsupported"
            )))
        }
    };
    if bytes.len() < pixel_count.saturating_mul(sample_bytes) {
        return Err(Error::InvalidInput(
            "label-map SEG frame metadata or Pixel Data is truncated".into(),
        ));
    }
    let mut frames = Vec::new();
    let mut decoded_pixels = 0_usize;
    let decode = |sample: &[u8]| {
        if sample_bytes == 1 {
            u16::from(sample[0])
        } else {
            u16::from_le_bytes([sample[0], sample[1]])
        }
    };
    for (frame_index, (_, tile_col, tile_row)) in frame_positions.into_iter().enumerate() {
        let start = frame_index * pixels_per_frame * sample_bytes;
        let frame_bytes = &bytes[start..start + pixels_per_frame * sample_bytes];
        let present = frame_bytes
            .chunks_exact(sample_bytes)
            .map(decode)
            .collect::<BTreeSet<_>>();
        for (segment_index, segment) in segments.iter().enumerate() {
            let segment_number =
                segment
                    .source_segment_number
                    .unwrap_or(u16::try_from(segment_index + 1).map_err(|_| {
                        Error::InvalidInput("label-map SEG segment number exceeds US".into())
                    })?);
            if segment_number == 0 || !present.contains(&segment_number) {
                continue;
            }
            decoded_pixels = decoded_pixels
                .checked_add(pixels_per_frame)
                .ok_or_else(|| Error::InvalidInput("decoded SEG pixel count overflows".into()))?;
            enforce_pixel_budget(decoded_pixels)?;
            let mask = frame_bytes
                .chunks_exact(sample_bytes)
                .map(|sample| decode(sample) == segment_number)
                .collect::<Vec<_>>();
            if mask.iter().any(|value| *value) {
                frames.push(BinarySegmentationFrame {
                    segment_number,
                    tile_col,
                    tile_row,
                    width: columns,
                    height: rows,
                    mask,
                });
            }
        }
    }
    Ok(frames)
}
