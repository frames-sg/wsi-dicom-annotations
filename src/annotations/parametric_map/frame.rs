use crate::{Error, Result};

use super::source::{NormalizedRaster, PixelTile, RasterDescriptor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PixelPrecision {
    Float32,
    Float64,
}

impl PixelPrecision {
    pub(super) const fn bytes_per_sample(self) -> u64 {
        match self {
            Self::Float32 => 4,
            Self::Float64 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FrameKey {
    pub(super) channel: usize,
    pub(super) channel_ordinal: u32,
    pub(super) tile_row: u32,
    pub(super) tile_column: u32,
}

impl FrameKey {
    pub(super) fn column_position(self, descriptor: RasterDescriptor) -> Result<u32> {
        matrix_position(self.tile_column, descriptor.tile_width, "column")
    }

    pub(super) fn row_position(self, descriptor: RasterDescriptor) -> Result<u32> {
        matrix_position(self.tile_row, descriptor.tile_height, "row")
    }
}

fn matrix_position(tile_index: u32, tile_size: u32, axis: &str) -> Result<u32> {
    tile_index
        .checked_mul(tile_size)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| Error::InvalidInput(format!("raster {axis} position overflows UL")))
}

pub(super) struct EncodedFrame {
    pub(super) key: FrameKey,
    pub(super) bytes: Vec<u8>,
    pub(super) all_missing: bool,
    pub(super) finite_minimum: Option<f64>,
    pub(super) finite_maximum: Option<f64>,
    pub(super) missing_sample_count: u64,
    pub(super) padded_sample_count: u64,
}

pub(super) fn encode_frame(
    raster: &NormalizedRaster,
    descriptor: RasterDescriptor,
    key: FrameKey,
) -> Result<EncodedFrame> {
    let origin_y = key
        .tile_row
        .checked_mul(descriptor.tile_height)
        .ok_or_else(|| Error::InvalidInput("raster tile row origin overflows UL".into()))?;
    let origin_x = key
        .tile_column
        .checked_mul(descriptor.tile_width)
        .ok_or_else(|| Error::InvalidInput("raster tile column origin overflows UL".into()))?;
    let height = descriptor.tile_height.min(descriptor.height - origin_y);
    let width = descriptor.tile_width.min(descriptor.width - origin_x);
    let tile = raster.read_tile(key.channel, origin_y, origin_x, height, width)?;
    match tile {
        PixelTile::F32(values) => encode_f32(values, descriptor, key, height, width),
        PixelTile::F64(values) => encode_f64(values, descriptor, key, height, width),
    }
}

fn encode_f32(
    values: Vec<f32>,
    descriptor: RasterDescriptor,
    key: FrameKey,
    height: u32,
    width: u32,
) -> Result<EncodedFrame> {
    validate_sample_count(values.len(), height, width)?;
    let padded_sample_count = padded_sample_count(descriptor, values.len())?;
    let mut bytes = Vec::with_capacity(frame_byte_length(descriptor, PixelPrecision::Float32)?);
    let mut range = FiniteRange::default();
    let width = width as usize;
    for row in values.chunks_exact(width) {
        for value in row {
            range.observe(f64::from(*value));
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for _ in width as u32..descriptor.tile_width {
            bytes.extend_from_slice(&super::source::CANONICAL_NAN_F32.to_le_bytes());
        }
    }
    for _ in height..descriptor.tile_height {
        for _ in 0..descriptor.tile_width {
            bytes.extend_from_slice(&super::source::CANONICAL_NAN_F32.to_le_bytes());
        }
    }
    Ok(range.finish(key, bytes, padded_sample_count))
}

fn encode_f64(
    values: Vec<f64>,
    descriptor: RasterDescriptor,
    key: FrameKey,
    height: u32,
    width: u32,
) -> Result<EncodedFrame> {
    validate_sample_count(values.len(), height, width)?;
    let padded_sample_count = padded_sample_count(descriptor, values.len())?;
    let mut bytes = Vec::with_capacity(frame_byte_length(descriptor, PixelPrecision::Float64)?);
    let mut range = FiniteRange::default();
    let width = width as usize;
    for row in values.chunks_exact(width) {
        for value in row {
            range.observe(*value);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for _ in width as u32..descriptor.tile_width {
            bytes.extend_from_slice(&super::source::CANONICAL_NAN_F64.to_le_bytes());
        }
    }
    for _ in height..descriptor.tile_height {
        for _ in 0..descriptor.tile_width {
            bytes.extend_from_slice(&super::source::CANONICAL_NAN_F64.to_le_bytes());
        }
    }
    Ok(range.finish(key, bytes, padded_sample_count))
}

fn padded_sample_count(descriptor: RasterDescriptor, actual: usize) -> Result<u64> {
    let actual = u64::try_from(actual)
        .map_err(|_| Error::InvalidInput("PM frame sample count does not fit u64".into()))?;
    u64::from(descriptor.tile_height)
        .checked_mul(u64::from(descriptor.tile_width))
        .and_then(|total| total.checked_sub(actual))
        .ok_or_else(|| Error::InvalidInput("PM frame padding count is invalid".into()))
}

fn validate_sample_count(actual: usize, height: u32, width: u32) -> Result<()> {
    let expected = usize::try_from(u64::from(height) * u64::from(width)).map_err(|_| {
        Error::InvalidInput("raster tile sample count exceeds addressable memory".into())
    })?;
    if actual != expected {
        return Err(Error::InvalidInput(format!(
            "raster adapter returned {actual} samples for a {height}x{width} region"
        )));
    }
    Ok(())
}

pub(super) fn frame_byte_length(
    descriptor: RasterDescriptor,
    precision: PixelPrecision,
) -> Result<usize> {
    usize::try_from(
        u64::from(descriptor.tile_height)
            .checked_mul(u64::from(descriptor.tile_width))
            .and_then(|samples| samples.checked_mul(precision.bytes_per_sample()))
            .ok_or_else(|| Error::InvalidInput("PM frame byte length overflows".into()))?,
    )
    .map_err(|_| Error::InvalidInput("PM frame exceeds addressable memory".into()))
}

#[derive(Default)]
struct FiniteRange {
    minimum: Option<f64>,
    maximum: Option<f64>,
    missing: u64,
}

impl FiniteRange {
    fn observe(&mut self, value: f64) {
        if value.is_finite() {
            self.minimum = Some(self.minimum.map_or(value, |minimum| minimum.min(value)));
            self.maximum = Some(self.maximum.map_or(value, |maximum| maximum.max(value)));
        } else {
            self.missing += 1;
        }
    }

    fn finish(self, key: FrameKey, bytes: Vec<u8>, padded_sample_count: u64) -> EncodedFrame {
        EncodedFrame {
            key,
            bytes,
            all_missing: self.minimum.is_none(),
            finite_minimum: self.minimum,
            finite_maximum: self.maximum,
            missing_sample_count: self.missing,
            padded_sample_count,
        }
    }
}
