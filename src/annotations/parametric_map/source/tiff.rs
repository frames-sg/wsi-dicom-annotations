use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use tiff::decoder::{ChunkType, Decoder, DecodingResult, Limits};
use tiff::tags::Tag;

use crate::{Error, Result};

use super::{
    normalized_shape, validate_region, RasterDType, RasterDescriptor, RasterProfile, RasterSource,
    RawTile,
};
use crate::annotations::parametric_map::profile::RasterAxis;

const MAX_TIFF_CHUNK_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct TiffSource {
    path: PathBuf,
    descriptor: RasterDescriptor,
    chunky: bool,
    chunks_across: u32,
    chunks_down: u32,
}

impl TiffSource {
    pub(super) fn open(profile: &RasterProfile, path: &Path) -> Result<Self> {
        let metadata = std::fs::metadata(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(Error::InvalidInput(format!(
                "TIFF input {} is not a regular file",
                path.display()
            )));
        }
        let mut decoder = open_decoder(path)?;
        let (width, height) = decoder.dimensions().map_err(tiff_error)?;
        let color_type = decoder.colortype().map_err(tiff_error)?;
        let channels = usize::from(color_type.num_samples());
        validate_profile_axes(profile, channels)?;
        let (profile_height, profile_width, profile_channels) = normalized_shape(
            &[u64::from(height), u64::from(width), channels as u64][..profile.axes.len()],
            &profile.axes,
            profile.channels.len(),
        )?;
        if (height, width, channels) != (profile_height, profile_width, profile_channels) {
            return Err(Error::InvalidInput(
                "TIFF dimensions or channel count do not match the profile axes".into(),
            ));
        }
        validate_tags(&mut decoder)?;
        let chunky = decoder
            .find_tag_unsigned::<u16>(Tag::PlanarConfiguration)
            .map_err(tiff_error)?
            .unwrap_or(1)
            == 1;
        let (tile_width, tile_height) = chunk_dimensions(&mut decoder, width, height)?;
        let chunks_across = width.div_ceil(tile_width);
        let chunks_down = height.div_ceil(tile_height);
        let first = decoder.read_chunk(0).map_err(tiff_error)?;
        let dtype = decoded_dtype(&first)?;
        Ok(Self {
            path: path.to_path_buf(),
            descriptor: RasterDescriptor {
                height,
                width,
                channels,
                dtype,
                tile_height,
                tile_width,
            },
            chunky,
            chunks_across,
            chunks_down,
        })
    }

    fn chunk_storage_index(&self, channel: usize, row: u32, column: u32) -> Result<u32> {
        let spatial = row
            .checked_mul(self.chunks_across)
            .and_then(|value| value.checked_add(column))
            .ok_or_else(|| Error::InvalidInput("TIFF chunk index overflows".into()))?;
        if self.chunky {
            Ok(spatial)
        } else {
            u32::try_from(channel)
                .ok()
                .and_then(|channel| {
                    channel
                        .checked_mul(self.chunks_across.checked_mul(self.chunks_down)?)
                        .and_then(|value| value.checked_add(spatial))
                })
                .ok_or_else(|| Error::InvalidInput("TIFF chunk index overflows".into()))
        }
    }
}

impl RasterSource for TiffSource {
    fn descriptor(&self) -> RasterDescriptor {
        self.descriptor
    }

    fn read_region(
        &self,
        channel: usize,
        origin_y: u32,
        origin_x: u32,
        height: u32,
        width: u32,
    ) -> Result<RawTile> {
        validate_region(self.descriptor, channel, origin_y, origin_x, height, width)?;
        let count = usize::try_from(u64::from(height) * u64::from(width)).map_err(|_| {
            Error::InvalidInput("requested TIFF region is too large for this platform".into())
        })?;
        let mut output = empty_raw_tile(self.descriptor.dtype, count);
        let mut decoder = open_decoder(&self.path)?;
        let first_row = origin_y / self.descriptor.tile_height;
        let last_row = (origin_y + height - 1) / self.descriptor.tile_height;
        let first_column = origin_x / self.descriptor.tile_width;
        let last_column = (origin_x + width - 1) / self.descriptor.tile_width;
        for row in first_row..=last_row {
            for column in first_column..=last_column {
                let index = self.chunk_storage_index(channel, row, column)?;
                let decoded = decoder.read_chunk(index).map_err(tiff_error)?;
                let chunk =
                    select_channel(decoded, channel, self.descriptor.channels, self.chunky)?;
                copy_chunk_intersection(
                    &mut output,
                    chunk,
                    self.descriptor,
                    ChunkPosition { row, column },
                    Region {
                        y: origin_y,
                        x: origin_x,
                        height,
                        width,
                    },
                )?;
            }
        }
        Ok(output)
    }
}

fn open_decoder(path: &Path) -> Result<Decoder<BufReader<File>>> {
    let file = File::open(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut limits = Limits::default();
    limits.decoding_buffer_size = MAX_TIFF_CHUNK_BYTES;
    limits.intermediate_buffer_size = MAX_TIFF_CHUNK_BYTES;
    Decoder::new(BufReader::new(file))
        .map(|decoder| decoder.with_limits(limits))
        .map_err(tiff_error)
}

fn validate_profile_axes(profile: &RasterProfile, channels: usize) -> Result<()> {
    let expected = if channels == 1 {
        &[RasterAxis::Y, RasterAxis::X][..]
    } else {
        &[RasterAxis::Y, RasterAxis::X, RasterAxis::Channel][..]
    };
    if profile.axes != expected {
        return Err(Error::InvalidInput(
            "TIFF axes must be [y,x] for one sample or [y,x,channel] for multiple samples".into(),
        ));
    }
    Ok(())
}

fn validate_tags(decoder: &mut Decoder<BufReader<File>>) -> Result<()> {
    let orientation = decoder
        .find_tag_unsigned::<u16>(Tag::Orientation)
        .map_err(tiff_error)?
        .unwrap_or(1);
    if orientation != 1 {
        return Err(Error::Unsupported(format!(
            "TIFF Orientation {orientation} is unsupported; only top-left orientation is accepted"
        )));
    }
    let compression = decoder
        .find_tag_unsigned::<u16>(Tag::Compression)
        .map_err(tiff_error)?
        .unwrap_or(1);
    if !matches!(compression, 1 | 5 | 8 | 32_773 | 32_946 | 50_000) {
        return Err(Error::Unsupported(format!(
            "TIFF compression {compression} is lossy or unsupported"
        )));
    }
    if decoder.more_images() {
        return Err(Error::Unsupported(
            "multi-page TIFF raster inputs are not supported".into(),
        ));
    }
    Ok(())
}

fn chunk_dimensions(
    decoder: &mut Decoder<BufReader<File>>,
    width: u32,
    height: u32,
) -> Result<(u32, u32)> {
    let dimensions = match decoder.get_chunk_type() {
        ChunkType::Strip => (
            width,
            decoder
                .find_tag_unsigned::<u32>(Tag::RowsPerStrip)
                .map_err(tiff_error)?
                .unwrap_or(height)
                .min(height),
        ),
        ChunkType::Tile => (
            decoder
                .get_tag_unsigned(Tag::TileWidth)
                .map_err(tiff_error)?,
            decoder
                .get_tag_unsigned(Tag::TileLength)
                .map_err(tiff_error)?,
        ),
    };
    if dimensions.0 == 0 || dimensions.1 == 0 {
        return Err(Error::InvalidInput(
            "TIFF chunk dimensions must be nonzero".into(),
        ));
    }
    Ok(dimensions)
}

fn decoded_dtype(values: &DecodingResult) -> Result<RasterDType> {
    match values {
        DecodingResult::F32(_) => Ok(RasterDType::Float32),
        DecodingResult::F64(_) => Ok(RasterDType::Float64),
        DecodingResult::I8(_) => Ok(RasterDType::Int8),
        DecodingResult::I16(_) => Ok(RasterDType::Int16),
        DecodingResult::I32(_) => Ok(RasterDType::Int32),
        DecodingResult::I64(_) => Ok(RasterDType::Int64),
        DecodingResult::U8(_) => Ok(RasterDType::Uint8),
        DecodingResult::U16(_) => Ok(RasterDType::Uint16),
        DecodingResult::U32(_) => Ok(RasterDType::Uint32),
        DecodingResult::U64(_) => Ok(RasterDType::Uint64),
        DecodingResult::F16(_) => Err(Error::Unsupported(
            "float16 TIFF raster inputs are not supported".into(),
        )),
    }
}

fn select_channel(
    values: DecodingResult,
    channel: usize,
    channels: usize,
    chunky: bool,
) -> Result<RawTile> {
    macro_rules! select {
        ($values:expr, $variant:ident) => {{
            let values = if chunky {
                $values
                    .into_iter()
                    .skip(channel)
                    .step_by(channels)
                    .collect()
            } else {
                $values
            };
            Ok(RawTile::$variant(values))
        }};
    }
    match values {
        DecodingResult::F32(values) => select!(values, F32),
        DecodingResult::F64(values) => select!(values, F64),
        DecodingResult::I8(values) => select!(values, I8),
        DecodingResult::I16(values) => select!(values, I16),
        DecodingResult::I32(values) => select!(values, I32),
        DecodingResult::I64(values) => select!(values, I64),
        DecodingResult::U8(values) => select!(values, U8),
        DecodingResult::U16(values) => select!(values, U16),
        DecodingResult::U32(values) => select!(values, U32),
        DecodingResult::U64(values) => select!(values, U64),
        DecodingResult::F16(_) => Err(Error::Unsupported(
            "float16 TIFF raster inputs are not supported".into(),
        )),
    }
}

fn empty_raw_tile(dtype: RasterDType, length: usize) -> RawTile {
    match dtype {
        RasterDType::Float32 => RawTile::F32(vec![0.0; length]),
        RasterDType::Float64 => RawTile::F64(vec![0.0; length]),
        RasterDType::Int8 => RawTile::I8(vec![0; length]),
        RasterDType::Int16 => RawTile::I16(vec![0; length]),
        RasterDType::Int32 => RawTile::I32(vec![0; length]),
        RasterDType::Int64 => RawTile::I64(vec![0; length]),
        RasterDType::Uint8 => RawTile::U8(vec![0; length]),
        RasterDType::Uint16 => RawTile::U16(vec![0; length]),
        RasterDType::Uint32 => RawTile::U32(vec![0; length]),
        RasterDType::Uint64 => RawTile::U64(vec![0; length]),
    }
}

#[derive(Debug, Clone, Copy)]
struct ChunkPosition {
    row: u32,
    column: u32,
}

#[derive(Debug, Clone, Copy)]
struct Region {
    y: u32,
    x: u32,
    height: u32,
    width: u32,
}

#[derive(Debug, Clone, Copy)]
struct CopyWindow {
    source_width: u32,
    destination_width: u32,
    source_y: u32,
    source_x: u32,
    destination_y: u32,
    destination_x: u32,
    height: u32,
    width: u32,
}

fn copy_chunk_intersection(
    output: &mut RawTile,
    chunk: RawTile,
    descriptor: RasterDescriptor,
    position: ChunkPosition,
    output_region: Region,
) -> Result<()> {
    let chunk_y = position
        .row
        .checked_mul(descriptor.tile_height)
        .ok_or_else(|| Error::InvalidInput("TIFF chunk y origin overflows".into()))?;
    let chunk_x = position
        .column
        .checked_mul(descriptor.tile_width)
        .ok_or_else(|| Error::InvalidInput("TIFF chunk x origin overflows".into()))?;
    let chunk_height = descriptor.tile_height.min(descriptor.height - chunk_y);
    let chunk_width = descriptor.tile_width.min(descriptor.width - chunk_x);
    let top = output_region.y.max(chunk_y);
    let left = output_region.x.max(chunk_x);
    let bottom = output_region
        .y
        .checked_add(output_region.height)
        .ok_or_else(|| Error::InvalidInput("TIFF output y extent overflows".into()))?
        .min(chunk_y + chunk_height);
    let right = output_region
        .x
        .checked_add(output_region.width)
        .ok_or_else(|| Error::InvalidInput("TIFF output x extent overflows".into()))?
        .min(chunk_x + chunk_width);
    let source_y = top - chunk_y;
    let source_x = left - chunk_x;
    let destination_y = top - output_region.y;
    let destination_x = left - output_region.x;
    let window = CopyWindow {
        source_width: chunk_width,
        destination_width: output_region.width,
        source_y,
        source_x,
        destination_y,
        destination_x,
        height: bottom - top,
        width: right - left,
    };
    macro_rules! copy {
        ($destination:expr, $source:expr) => {{
            copy_values($destination, &$source, window)
        }};
    }
    match (output, chunk) {
        (RawTile::F32(destination), RawTile::F32(source)) => copy!(destination, source),
        (RawTile::F64(destination), RawTile::F64(source)) => copy!(destination, source),
        (RawTile::I8(destination), RawTile::I8(source)) => copy!(destination, source),
        (RawTile::I16(destination), RawTile::I16(source)) => copy!(destination, source),
        (RawTile::I32(destination), RawTile::I32(source)) => copy!(destination, source),
        (RawTile::I64(destination), RawTile::I64(source)) => copy!(destination, source),
        (RawTile::U8(destination), RawTile::U8(source)) => copy!(destination, source),
        (RawTile::U16(destination), RawTile::U16(source)) => copy!(destination, source),
        (RawTile::U32(destination), RawTile::U32(source)) => copy!(destination, source),
        (RawTile::U64(destination), RawTile::U64(source)) => copy!(destination, source),
        _ => Err(Error::InvalidInput(
            "TIFF chunks disagree on decoded data type".into(),
        )),
    }
}

fn copy_values<T: Copy>(destination: &mut [T], source: &[T], window: CopyWindow) -> Result<()> {
    for row in 0..window.height {
        let source_start = linear_index(
            window.source_y,
            row,
            window.source_x,
            window.source_width,
            "source",
        )?;
        let destination_start = linear_index(
            window.destination_y,
            row,
            window.destination_x,
            window.destination_width,
            "destination",
        )?;
        let width = usize::try_from(window.width)
            .map_err(|_| Error::InvalidInput("TIFF copy width overflows".into()))?;
        let source_end = source_start
            .checked_add(width)
            .ok_or_else(|| Error::InvalidInput("TIFF source range overflows".into()))?;
        let destination_end = destination_start
            .checked_add(width)
            .ok_or_else(|| Error::InvalidInput("TIFF destination range overflows".into()))?;
        let source_row = source
            .get(source_start..source_end)
            .ok_or_else(|| Error::InvalidInput("TIFF chunk data is truncated".into()))?;
        let destination_row = destination
            .get_mut(destination_start..destination_end)
            .ok_or_else(|| Error::InvalidInput("TIFF output region overflows".into()))?;
        destination_row.copy_from_slice(source_row);
    }
    Ok(())
}

fn linear_index(y: u32, row: u32, x: u32, width: u32, side: &str) -> Result<usize> {
    u64::from(y)
        .checked_add(u64::from(row))
        .and_then(|y| y.checked_mul(u64::from(width)))
        .and_then(|offset| offset.checked_add(u64::from(x)))
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or_else(|| Error::InvalidInput(format!("TIFF {side} index overflows")))
}

fn tiff_error(error: tiff::TiffError) -> Error {
    Error::InvalidInput(format!("TIFF decode failed: {error}"))
}

#[cfg(test)]
#[path = "tiff_unit_tests.rs"]
mod tests;
