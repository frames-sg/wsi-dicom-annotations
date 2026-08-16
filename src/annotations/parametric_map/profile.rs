use std::collections::BTreeSet;

use serde::Deserialize;

use crate::{Error, Result};

use super::super::json::validate_unique_object_keys;
use super::super::model::{AlgorithmIdentification, DicomCode};
use super::super::profile::{ProfileAlgorithm, ProfileCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RasterInputFormat {
    Tiff,
    Npy,
    Zarr,
    TiledManifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterChannelSelection {
    Auto,
    Index(usize),
    Name(String),
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RasterDType {
    Float32,
    Float64,
    Int8,
    Int16,
    Int32,
    Int64,
    Uint8,
    Uint16,
    Uint32,
    Uint64,
}

impl RasterDType {
    pub(crate) const fn is_integer(self) -> bool {
        !matches!(self, Self::Float32 | Self::Float64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RasterAxis {
    Y,
    X,
    Channel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RasterCoordinateSpace {
    Level0Pixels,
    SourcePixels,
    SlideMm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RasterOutputPrecision {
    Float32,
    Float64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(untagged)]
pub(crate) enum IntegerSentinel {
    Signed(i64),
    Unsigned(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct IntegerScaling {
    pub(crate) slope: f64,
    pub(crate) intercept: f64,
    pub(crate) missing_sentinel: IntegerSentinel,
    pub(crate) output_precision: RasterOutputPrecision,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RasterChannel {
    pub(crate) name: String,
    pub(crate) quantity: DicomCode,
    pub(crate) unit: DicomCode,
}

#[derive(Debug, Clone)]
pub struct RasterProfile {
    pub(crate) input_format: RasterInputFormat,
    pub(crate) dtype: RasterDType,
    pub(crate) axes: Vec<RasterAxis>,
    pub(crate) grid_origin: GridOrigin,
    pub(crate) sample_spacing: SampleSpacing,
    pub(crate) coordinate_space: RasterCoordinateSpace,
    pub(crate) channels: Vec<RasterChannel>,
    pub(crate) algorithm: AlgorithmIdentification,
    pub(crate) integer_scaling: Option<IntegerScaling>,
    pub(crate) zarr_array_path: Option<String>,
}

impl RasterProfile {
    pub fn from_json(json: &[u8]) -> Result<Self> {
        validate_unique_object_keys(json, "raster profile")?;
        let raw: RawRasterProfile = serde_json::from_slice(json).map_err(|error| {
            Error::InvalidInput(format!("raster profile is not valid JSON: {error}"))
        })?;
        raw.validate_and_build()
    }

    #[must_use]
    pub const fn input_format(&self) -> RasterInputFormat {
        self.input_format
    }

    #[must_use]
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    #[must_use]
    pub fn channel_name(&self, index: usize) -> Option<&str> {
        self.channels
            .get(index)
            .map(|channel| channel.name.as_str())
    }

    pub fn select_channels(&self, selection: RasterChannelSelection) -> Result<Vec<usize>> {
        if self.channels.len() == 1 {
            return match selection {
                RasterChannelSelection::Auto => Ok(vec![0]),
                _ => Err(Error::InvalidInput(
                    "single-channel raster inputs do not accept a channel option".into(),
                )),
            };
        }
        match selection {
            RasterChannelSelection::Auto => Err(Error::InvalidInput(
                "multichannel raster input requires exactly one channel or all channels".into(),
            )),
            RasterChannelSelection::Index(index) if index < self.channels.len() => Ok(vec![index]),
            RasterChannelSelection::Index(index) => Err(Error::InvalidInput(format!(
                "channel index {index} is outside 0..{}",
                self.channels.len()
            ))),
            RasterChannelSelection::Name(name) => self
                .channels
                .iter()
                .position(|channel| channel.name == name)
                .map(|index| vec![index])
                .ok_or_else(|| {
                    Error::InvalidInput(format!("channel name {name:?} is not declared"))
                }),
            RasterChannelSelection::All => Ok((0..self.channels.len()).collect()),
        }
    }

    pub(crate) fn axis_index(&self, axis: RasterAxis) -> Option<usize> {
        self.axes.iter().position(|candidate| *candidate == axis)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRasterProfile {
    schema_version: u32,
    input_format: RasterInputFormat,
    dtype: RasterDType,
    axes: Vec<RasterAxis>,
    grid_origin: GridOrigin,
    sample_spacing: SampleSpacing,
    coordinate_space: RasterCoordinateSpace,
    channels: Vec<RawRasterChannel>,
    algorithm: ProfileAlgorithm,
    #[serde(default)]
    integer_scaling: Option<RawIntegerScaling>,
    #[serde(default)]
    zarr_array_path: Option<String>,
}

impl RawRasterProfile {
    fn validate_and_build(self) -> Result<RasterProfile> {
        if self.schema_version != 1 {
            return Err(Error::Unsupported(format!(
                "raster profile schema version {} is not supported",
                self.schema_version
            )));
        }
        validate_axes(&self.axes)?;
        self.grid_origin.validate(self.coordinate_space)?;
        self.sample_spacing.validate()?;
        let channels = build_channels(self.channels)?;
        let integer_scaling = self
            .integer_scaling
            .map(RawIntegerScaling::build)
            .transpose()?;
        if self.dtype.is_integer() != integer_scaling.is_some() {
            return Err(Error::InvalidInput(
                "integer dtypes require integer_scaling, which is forbidden for float dtypes"
                    .into(),
            ));
        }
        validate_zarr_path(self.input_format, self.zarr_array_path.as_deref())?;
        Ok(RasterProfile {
            input_format: self.input_format,
            dtype: self.dtype,
            axes: self.axes,
            grid_origin: self.grid_origin,
            sample_spacing: self.sample_spacing,
            coordinate_space: self.coordinate_space,
            channels,
            algorithm: self.algorithm.into_algorithm()?,
            integer_scaling,
            zarr_array_path: self.zarr_array_path,
        })
    }
}

fn validate_axes(axes: &[RasterAxis]) -> Result<()> {
    if !matches!(axes.len(), 2 | 3)
        || axes.iter().filter(|axis| **axis == RasterAxis::Y).count() != 1
        || axes.iter().filter(|axis| **axis == RasterAxis::X).count() != 1
        || axes
            .iter()
            .filter(|axis| **axis == RasterAxis::Channel)
            .count()
            != axes.len() - 2
    {
        return Err(Error::InvalidInput(
            "axes must contain y and x exactly once, plus at most one channel axis".into(),
        ));
    }
    Ok(())
}

fn build_channels(raw: Vec<RawRasterChannel>) -> Result<Vec<RasterChannel>> {
    if raw.is_empty() {
        return Err(Error::InvalidInput(
            "raster profile must declare at least one channel".into(),
        ));
    }
    let mut names = BTreeSet::new();
    raw.into_iter()
        .map(|channel| {
            if channel.name.trim().is_empty()
                || channel.name.contains('\0')
                || !names.insert(channel.name.clone())
            {
                return Err(Error::InvalidInput(
                    "raster channel names must be nonempty, NUL-free, and unique".into(),
                ));
            }
            Ok(RasterChannel {
                name: channel.name,
                quantity: channel.quantity.into_code()?,
                unit: channel.unit.into_code()?,
            })
        })
        .collect()
}

fn validate_zarr_path(format: RasterInputFormat, path: Option<&str>) -> Result<()> {
    match (format, path) {
        (RasterInputFormat::Zarr, Some(path))
            if !path.trim_matches('/').is_empty()
                && !path.split('/').any(|part| matches!(part, "" | "." | "..")) =>
        {
            Ok(())
        }
        (RasterInputFormat::Zarr, _) => Err(Error::InvalidInput(
            "zarr input requires an explicit local zarr_array_path".into(),
        )),
        (_, None) => Ok(()),
        (_, Some(_)) => Err(Error::InvalidInput(
            "zarr_array_path is valid only for zarr input".into(),
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GridOrigin {
    pub(crate) x: f64,
    pub(crate) y: f64,
    #[serde(default)]
    pub(crate) z: Option<f64>,
}

impl GridOrigin {
    fn validate(self, coordinate_space: RasterCoordinateSpace) -> Result<()> {
        if !self.x.is_finite() || !self.y.is_finite() || self.z.is_some_and(|z| !z.is_finite()) {
            return Err(Error::InvalidInput(
                "grid origin coordinates must be finite".into(),
            ));
        }
        if (coordinate_space == RasterCoordinateSpace::SlideMm) != self.z.is_some() {
            return Err(Error::InvalidInput(
                "slide-mm grid origin requires z; pixel coordinate origins must omit z".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SampleSpacing {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl SampleSpacing {
    fn validate(self) -> Result<()> {
        if !self.x.is_finite() || !self.y.is_finite() || self.x <= 0.0 || self.y <= 0.0 {
            return Err(Error::InvalidInput(
                "sample spacing must contain positive finite x and y values".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRasterChannel {
    name: String,
    quantity: ProfileCode,
    unit: ProfileCode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIntegerScaling {
    slope: f64,
    intercept: f64,
    missing_sentinel: IntegerSentinel,
    output_precision: RasterOutputPrecision,
}

impl RawIntegerScaling {
    fn build(self) -> Result<IntegerScaling> {
        if !self.slope.is_finite() || !self.intercept.is_finite() || self.slope == 0.0 {
            return Err(Error::InvalidInput(
                "integer scaling slope must be nonzero and slope/intercept must be finite".into(),
            ));
        }
        Ok(IntegerScaling {
            slope: self.slope,
            intercept: self.intercept,
            missing_sentinel: self.missing_sentinel,
            output_precision: self.output_precision,
        })
    }
}
