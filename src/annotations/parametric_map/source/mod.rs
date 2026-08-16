mod npy;
mod tiff;
#[cfg(test)]
mod tiff_tests;
mod tiled_manifest;
#[cfg(test)]
mod tiled_manifest_tests;
mod zarr;
#[cfg(test)]
mod zarr_tests;

use std::path::Path;

use crate::{Error, InteroperabilityDiagnostic, Result};

use super::profile::{
    IntegerScaling, IntegerSentinel, RasterAxis, RasterDType, RasterOutputPrecision, RasterProfile,
};
use npy::NpySource;
use tiff::TiffSource;
use tiled_manifest::TiledManifestSource;
use zarr::ZarrSource;

const DEFAULT_TILE_LENGTH: u32 = 256;
pub(super) const CANONICAL_NAN_F32: f32 = f32::from_bits(0x7fc0_0000);
pub(super) const CANONICAL_NAN_F64: f64 = f64::from_bits(0x7ff8_0000_0000_0000);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RasterDescriptor {
    pub(crate) height: u32,
    pub(crate) width: u32,
    pub(crate) channels: usize,
    pub(crate) dtype: RasterDType,
    pub(crate) tile_height: u32,
    pub(crate) tile_width: u32,
}

pub(crate) enum RawTile {
    F32(Vec<f32>),
    F64(Vec<f64>),
    I8(Vec<i8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
    U64(Vec<u64>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PixelTile {
    F32(Vec<f32>),
    F64(Vec<f64>),
}

pub(crate) trait RasterSource {
    fn descriptor(&self) -> RasterDescriptor;

    fn read_region(
        &self,
        channel: usize,
        origin_y: u32,
        origin_x: u32,
        height: u32,
        width: u32,
    ) -> Result<RawTile>;

    fn normalizations(&self) -> &[SourceNormalization] {
        &[]
    }
}

pub(crate) struct NormalizedRaster {
    source: Box<dyn RasterSource>,
    scaling: Option<IntegerScaling>,
    normalizations: Vec<InteroperabilityDiagnostic>,
}

impl NormalizedRaster {
    pub(crate) fn open(profile: &RasterProfile, path: &Path) -> Result<Self> {
        let source: Box<dyn RasterSource> = match profile.input_format {
            super::RasterInputFormat::Npy => Box::new(NpySource::open(profile, path)?),
            super::RasterInputFormat::Tiff => Box::new(TiffSource::open(profile, path)?),
            super::RasterInputFormat::Zarr => Box::new(ZarrSource::open(profile, path)?),
            super::RasterInputFormat::TiledManifest => {
                Box::new(TiledManifestSource::open(profile, path)?)
            }
        };
        let descriptor = source.descriptor();
        if descriptor.dtype != profile.dtype {
            return Err(Error::InvalidInput(format!(
                "raster dtype {:?} does not match profile dtype {:?}",
                descriptor.dtype, profile.dtype
            )));
        }
        validate_integer_sentinel(descriptor.dtype, profile.integer_scaling.as_ref())?;
        let normalizations = source
            .normalizations()
            .iter()
            .copied()
            .map(SourceNormalization::diagnostic)
            .collect();
        Ok(Self {
            source,
            scaling: profile.integer_scaling.clone(),
            normalizations,
        })
    }

    pub(crate) fn descriptor(&self) -> RasterDescriptor {
        self.source.descriptor()
    }

    pub(crate) fn normalizations(&self) -> &[InteroperabilityDiagnostic] {
        &self.normalizations
    }

    pub(crate) fn read_tile(
        &self,
        channel: usize,
        origin_y: u32,
        origin_x: u32,
        height: u32,
        width: u32,
    ) -> Result<PixelTile> {
        normalize_tile(
            self.source
                .read_region(channel, origin_y, origin_x, height, width)?,
            self.scaling.as_ref(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SourceNormalization {
    NpyFortranOrder,
    ZarrAxisOrder,
    ManifestValidRegionCrop,
    ManifestOverlapMean,
    ManifestOverlapMax,
    ManifestOrderLastWrite,
}

impl SourceNormalization {
    fn diagnostic(self) -> InteroperabilityDiagnostic {
        let (code, path, message) = match self {
            Self::NpyFortranOrder => (
                "NPY_FORTRAN_ORDER_NORMALIZED",
                "$.raster.order",
                "read Fortran-order NPY storage into profile-declared y/x sample order",
            ),
            Self::ZarrAxisOrder => (
                "ZARR_AXIS_ORDER_NORMALIZED",
                "$.raster.axes",
                "read Zarr dimensions into profile-declared y/x sample order",
            ),
            Self::ManifestValidRegionCrop => (
                "TILED_MANIFEST_VALID_REGION_CROP",
                "$.raster.overlap_policy",
                "cropped every manifest tile to its explicit valid region",
            ),
            Self::ManifestOverlapMean => (
                "TILED_MANIFEST_OVERLAP_MEAN",
                "$.raster.overlap_policy",
                "combined finite overlapping samples by arithmetic mean",
            ),
            Self::ManifestOverlapMax => (
                "TILED_MANIFEST_OVERLAP_MAX",
                "$.raster.overlap_policy",
                "combined finite overlapping samples by maximum",
            ),
            Self::ManifestOrderLastWrite => (
                "TILED_MANIFEST_ORDER_LAST_WRITE",
                "$.raster.overlap_policy",
                "resolved overlapping samples in manifest order with last-write precedence",
            ),
        };
        InteroperabilityDiagnostic::normalized(code, path, message)
    }
}

pub(super) fn normalized_shape(
    shape: &[u64],
    axes: &[RasterAxis],
    profile_channels: usize,
) -> Result<(u32, u32, usize)> {
    if shape.len() != axes.len() {
        return Err(Error::InvalidInput(format!(
            "raster has {} dimensions, but profile declares {} axes",
            shape.len(),
            axes.len()
        )));
    }
    let dimension = |axis| {
        axes.iter()
            .position(|candidate| *candidate == axis)
            .and_then(|index| shape.get(index).copied())
    };
    let height = u32::try_from(dimension(RasterAxis::Y).unwrap_or(0))
        .map_err(|_| Error::InvalidInput("raster y dimension exceeds the DICOM UL range".into()))?;
    let width = u32::try_from(dimension(RasterAxis::X).unwrap_or(0))
        .map_err(|_| Error::InvalidInput("raster x dimension exceeds the DICOM UL range".into()))?;
    let channels = dimension(RasterAxis::Channel)
        .map(usize::try_from)
        .transpose()
        .map_err(|_| Error::InvalidInput("raster channel count exceeds usize".into()))?
        .unwrap_or(1);
    if height == 0 || width == 0 || channels == 0 || channels != profile_channels {
        return Err(Error::InvalidInput(format!(
            "raster shape resolves to {height}x{width}x{channels}, but the profile declares {profile_channels} channels"
        )));
    }
    Ok((height, width, channels))
}

pub(super) fn default_tile_length(length: u32) -> u32 {
    length.min(DEFAULT_TILE_LENGTH)
}

pub(super) fn validate_region(
    descriptor: RasterDescriptor,
    channel: usize,
    origin_y: u32,
    origin_x: u32,
    height: u32,
    width: u32,
) -> Result<()> {
    if channel >= descriptor.channels
        || height == 0
        || width == 0
        || origin_y
            .checked_add(height)
            .is_none_or(|end| end > descriptor.height)
        || origin_x
            .checked_add(width)
            .is_none_or(|end| end > descriptor.width)
    {
        return Err(Error::InvalidInput(
            "requested raster region is empty or outside the input shape".into(),
        ));
    }
    Ok(())
}

pub(super) fn normalize_tile(tile: RawTile, scaling: Option<&IntegerScaling>) -> Result<PixelTile> {
    match tile {
        RawTile::F32(values) => Ok(PixelTile::F32(normalize_f32(values)?)),
        RawTile::F64(values) => Ok(PixelTile::F64(normalize_f64(values)?)),
        RawTile::I8(values) => scale_signed(values, scaling),
        RawTile::I16(values) => scale_signed(values, scaling),
        RawTile::I32(values) => scale_signed(values, scaling),
        RawTile::I64(values) => scale_signed(values, scaling),
        RawTile::U8(values) => scale_unsigned(values, scaling),
        RawTile::U16(values) => scale_unsigned(values, scaling),
        RawTile::U32(values) => scale_unsigned(values, scaling),
        RawTile::U64(values) => scale_unsigned(values, scaling),
    }
}

fn normalize_f32(mut values: Vec<f32>) -> Result<Vec<f32>> {
    for value in &mut values {
        if value.is_infinite() {
            return Err(Error::InvalidInput(
                "raster contains an infinite floating-point value".into(),
            ));
        }
        if value.is_nan() {
            *value = CANONICAL_NAN_F32;
        }
    }
    Ok(values)
}

fn normalize_f64(mut values: Vec<f64>) -> Result<Vec<f64>> {
    for value in &mut values {
        if value.is_infinite() {
            return Err(Error::InvalidInput(
                "raster contains an infinite floating-point value".into(),
            ));
        }
        if value.is_nan() {
            *value = CANONICAL_NAN_F64;
        }
    }
    Ok(values)
}

fn scale_signed<T>(values: Vec<T>, scaling: Option<&IntegerScaling>) -> Result<PixelTile>
where
    T: Copy + Into<i64>,
{
    let scaling = scaling.ok_or_else(missing_scaling)?;
    let IntegerSentinel::Signed(sentinel) = scaling.missing_sentinel else {
        return Err(Error::InvalidInput(
            "signed raster dtype requires a signed missing sentinel".into(),
        ));
    };
    scale_values(
        values.into_iter().map(Into::into),
        |value| value == sentinel,
        |value| value as f64,
        scaling,
    )
}

fn scale_unsigned<T>(values: Vec<T>, scaling: Option<&IntegerScaling>) -> Result<PixelTile>
where
    T: Copy + Into<u64>,
{
    let scaling = scaling.ok_or_else(missing_scaling)?;
    let sentinel = match scaling.missing_sentinel {
        IntegerSentinel::Signed(value) if value >= 0 => value as u64,
        IntegerSentinel::Unsigned(value) => value,
        IntegerSentinel::Signed(_) => {
            return Err(Error::InvalidInput(
                "unsigned raster dtype requires a nonnegative missing sentinel".into(),
            ));
        }
    };
    scale_values(
        values.into_iter().map(Into::into),
        |value| value == sentinel,
        |value| value as f64,
        scaling,
    )
}

fn scale_values<T>(
    values: impl IntoIterator<Item = T>,
    is_missing: impl Fn(T) -> bool,
    to_f64: impl Fn(T) -> f64,
    scaling: &IntegerScaling,
) -> Result<PixelTile>
where
    T: Copy,
{
    match scaling.output_precision {
        RasterOutputPrecision::Float32 => values
            .into_iter()
            .map(|value| {
                if is_missing(value) {
                    return Ok(CANONICAL_NAN_F32);
                }
                let scaled = to_f64(value).mul_add(scaling.slope, scaling.intercept) as f32;
                if !scaled.is_finite() {
                    return Err(Error::InvalidInput(
                        "integer scaling produces a nonfinite float32 value".into(),
                    ));
                }
                Ok(scaled)
            })
            .collect::<Result<Vec<_>>>()
            .map(PixelTile::F32),
        RasterOutputPrecision::Float64 => values
            .into_iter()
            .map(|value| {
                if is_missing(value) {
                    return Ok(CANONICAL_NAN_F64);
                }
                let scaled = to_f64(value).mul_add(scaling.slope, scaling.intercept);
                if !scaled.is_finite() {
                    return Err(Error::InvalidInput(
                        "integer scaling produces a nonfinite float64 value".into(),
                    ));
                }
                Ok(scaled)
            })
            .collect::<Result<Vec<_>>>()
            .map(PixelTile::F64),
    }
}

fn validate_integer_sentinel(dtype: RasterDType, scaling: Option<&IntegerScaling>) -> Result<()> {
    let Some(scaling) = scaling else {
        return Ok(());
    };
    let valid = match (dtype, scaling.missing_sentinel) {
        (RasterDType::Int8, IntegerSentinel::Signed(value)) => i8::try_from(value).is_ok(),
        (RasterDType::Int16, IntegerSentinel::Signed(value)) => i16::try_from(value).is_ok(),
        (RasterDType::Int32, IntegerSentinel::Signed(value)) => i32::try_from(value).is_ok(),
        (RasterDType::Int64, IntegerSentinel::Signed(_)) => true,
        (RasterDType::Uint8, sentinel) => {
            unsigned_sentinel(sentinel).is_some_and(|v| u8::try_from(v).is_ok())
        }
        (RasterDType::Uint16, sentinel) => {
            unsigned_sentinel(sentinel).is_some_and(|v| u16::try_from(v).is_ok())
        }
        (RasterDType::Uint32, sentinel) => {
            unsigned_sentinel(sentinel).is_some_and(|v| u32::try_from(v).is_ok())
        }
        (RasterDType::Uint64, sentinel) => unsigned_sentinel(sentinel).is_some(),
        (RasterDType::Float32 | RasterDType::Float64, _) => false,
        _ => false,
    };
    if !valid {
        return Err(Error::InvalidInput(
            "integer missing sentinel does not fit the declared dtype".into(),
        ));
    }
    Ok(())
}

fn unsigned_sentinel(sentinel: IntegerSentinel) -> Option<u64> {
    match sentinel {
        IntegerSentinel::Signed(value) => u64::try_from(value).ok(),
        IntegerSentinel::Unsigned(value) => Some(value),
    }
}

fn missing_scaling() -> Error {
    Error::InvalidInput("integer raster has no scaling declaration".into())
}

#[cfg(test)]
#[path = "source_unit_tests.rs"]
mod unit_tests;
