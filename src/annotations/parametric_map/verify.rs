use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use dicom_core::Tag;
use dicom_dictionary_std::{tags, uids};
use dicom_object::DefaultDicomObject;
use sha2::{Digest, Sha256};

use crate::annotations::dicom_dataset::{
    optional_string, required_string, required_u16, required_u32,
};
use crate::metadata::open_metadata_object;
use crate::{Error, Result};

use super::document::ParametricMapDocument;
use super::frame::PixelPrecision;
use super::plan::ParametricMapPartPlan;
use super::writer::InstanceSpec;

const EXPLICIT_LONG_HEADER_BYTES: u64 = 12;

pub(super) struct ExpectedInstance<'a> {
    document: &'a ParametricMapDocument,
    spec: &'a InstanceSpec<'a>,
    part: &'a ParametricMapPartPlan,
}

impl<'a> ExpectedInstance<'a> {
    pub(super) const fn new(
        document: &'a ParametricMapDocument,
        spec: &'a InstanceSpec<'a>,
        part: &'a ParametricMapPartPlan,
    ) -> Self {
        Self {
            document,
            spec,
            part,
        }
    }
}

pub(super) fn verify_instance(path: &Path, expected: &ExpectedInstance<'_>) -> Result<()> {
    let actual_length = fs::metadata(path)
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if actual_length != expected.part.encoded_file_length() {
        return mismatch(format!(
            "encoded length is {actual_length}, expected {}",
            expected.part.encoded_file_length()
        ));
    }
    let object = open_metadata_object(path)?;
    verify_metadata(&object, expected)?;
    verify_pixel_value(path, expected)
}

fn verify_metadata(object: &DefaultDicomObject, expected: &ExpectedInstance<'_>) -> Result<()> {
    if object.meta().media_storage_sop_class_uid() != uids::PARAMETRIC_MAP_STORAGE {
        return mismatch("file meta SOP Class is not Parametric Map Storage");
    }
    for (tag, value) in [
        (tags::SOP_CLASS_UID, uids::PARAMETRIC_MAP_STORAGE),
        (tags::SOP_INSTANCE_UID, expected.spec.sop_instance_uid),
        (tags::SERIES_INSTANCE_UID, expected.spec.series_instance_uid),
        (
            tags::STUDY_INSTANCE_UID,
            expected.document.source.study_instance_uid(),
        ),
        (
            tags::FRAME_OF_REFERENCE_UID,
            expected
                .document
                .source
                .frame_of_reference_uid()
                .ok_or_else(|| {
                    Error::InvalidInput("PM source has no Frame of Reference UID".into())
                })?,
        ),
    ] {
        if required_string(object, tag)? != value {
            return mismatch(format!("attribute {tag} does not match the write plan"));
        }
    }
    for (tag, value) in [
        (tags::NUMBER_OF_FRAMES, expected.part.frame_count()),
        (
            tags::TOTAL_PIXEL_MATRIX_ROWS,
            expected.document.descriptor.height,
        ),
        (
            tags::TOTAL_PIXEL_MATRIX_COLUMNS,
            expected.document.descriptor.width,
        ),
    ] {
        if required_u32(object, tag)? != value {
            return mismatch(format!("attribute {tag} does not match the write plan"));
        }
    }
    for tag in [
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        tags::ACQUISITION_CONTEXT_SEQUENCE,
    ] {
        if object.get(tag).is_none() {
            return mismatch(format!("required PM sequence {tag} is absent"));
        }
    }
    let per_frame_count = object
        .get(tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE)
        .and_then(|element| element.items())
        .map_or(0, <[_]>::len);
    if per_frame_count != expected.spec.frames.len() {
        return mismatch("Per-frame Functional Groups item count is incorrect");
    }
    let expected_bits = match expected.document.precision {
        PixelPrecision::Float32 => 32_u16,
        PixelPrecision::Float64 => 64_u16,
    };
    if required_u16(object, tags::BITS_ALLOCATED)? != expected_bits
        || required_u16(object, tags::ROWS)?
            != u16::try_from(expected.document.descriptor.tile_height)
                .map_err(|_| Error::InvalidInput("planned PM Rows exceeds DICOM US".into()))?
        || required_u16(object, tags::COLUMNS)?
            != u16::try_from(expected.document.descriptor.tile_width)
                .map_err(|_| Error::InvalidInput("planned PM Columns exceeds DICOM US".into()))?
    {
        return mismatch("PM pixel module differs from the write plan");
    }
    verify_padding(object, expected.document.precision)?;
    verify_concatenation(object, expected)
}

fn verify_concatenation(
    object: &DefaultDicomObject,
    expected: &ExpectedInstance<'_>,
) -> Result<()> {
    let concatenation_uid = optional_string(object, tags::CONCATENATION_UID);
    match expected.spec.concatenation {
        Some(concatenation) => {
            if concatenation_uid.as_deref() != Some(concatenation.uid)
                || optional_string(object, tags::SOP_INSTANCE_UID_OF_CONCATENATION_SOURCE)
                    .as_deref()
                    != Some(concatenation.source_sop_instance_uid)
                || required_u16(object, tags::IN_CONCATENATION_NUMBER)? != concatenation.number
                || required_u16(object, tags::IN_CONCATENATION_TOTAL_NUMBER)? != concatenation.total
                || required_u32(object, tags::CONCATENATION_FRAME_OFFSET_NUMBER)?
                    != expected.part.frame_offset()
            {
                return mismatch("PM concatenation attributes differ from the write plan");
            }
        }
        None if concatenation_uid.is_some() => {
            return mismatch("single-instance PM unexpectedly has Concatenation UID")
        }
        None => {}
    }
    Ok(())
}

fn verify_padding(object: &DefaultDicomObject, precision: PixelPrecision) -> Result<()> {
    let valid = match precision {
        PixelPrecision::Float32 => object
            .get(tags::FLOAT_PIXEL_PADDING_VALUE)
            .and_then(|element| element.to_float32().ok())
            .is_some_and(|value| value.to_bits() == 0x7fc0_0000),
        PixelPrecision::Float64 => object
            .get(tags::DOUBLE_FLOAT_PIXEL_PADDING_VALUE)
            .and_then(|element| element.to_float64().ok())
            .is_some_and(|value| value.to_bits() == 0x7ff8_0000_0000_0000),
    };
    if !valid {
        return mismatch("PM pixel padding is not the canonical quiet NaN");
    }
    Ok(())
}

fn verify_pixel_value(path: &Path, expected: &ExpectedInstance<'_>) -> Result<()> {
    let value_length = u64::from(expected.part.pixel_value_length());
    let header_offset = expected
        .part
        .encoded_file_length()
        .checked_sub(value_length + EXPLICIT_LONG_HEADER_BYTES)
        .ok_or_else(|| Error::InvalidInput("planned PM tail offset underflows".into()))?;
    let mut file = File::open(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.seek(SeekFrom::Start(header_offset))
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut header = [0_u8; 12];
    file.read_exact(&mut header).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let (tag, vr) = pixel_tag_and_vr(expected.document.precision);
    if header[..2] != tag.group().to_le_bytes()
        || header[2..4] != tag.element().to_le_bytes()
        || header[4..6] != *vr
        || header[6..8] != [0, 0]
        || u32::from_le_bytes(
            header[8..12]
                .try_into()
                .map_err(|_| Error::InvalidInput("PM pixel element header is truncated".into()))?,
        ) != expected.part.pixel_value_length()
    {
        return mismatch("streamed PM pixel element header is invalid");
    }
    let mut digest = Sha256::new();
    let mut remaining = value_length;
    let mut buffer = [0_u8; 128 * 1024];
    while remaining > 0 {
        let wanted = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| {
            Error::InvalidInput("PM verification read length does not fit usize".into())
        })?;
        file.read_exact(&mut buffer[..wanted])
            .map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
        digest.update(&buffer[..wanted]);
        remaining -= u64::try_from(wanted).map_err(|_| {
            Error::InvalidInput("PM verification read length does not fit u64".into())
        })?;
    }
    if format!("{:x}", digest.finalize()) != expected.part.pixel_sha256() {
        return mismatch("streamed PM pixel digest differs from the write plan");
    }
    Ok(())
}

fn pixel_tag_and_vr(precision: PixelPrecision) -> (Tag, &'static [u8; 2]) {
    match precision {
        PixelPrecision::Float32 => (tags::FLOAT_PIXEL_DATA, b"OF"),
        PixelPrecision::Float64 => (tags::DOUBLE_FLOAT_PIXEL_DATA, b"OD"),
    }
}

fn mismatch(message: impl Into<String>) -> Result<()> {
    Err(Error::InvalidInput(format!(
        "staged Parametric Map verification failed: {}",
        message.into()
    )))
}
