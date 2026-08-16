use std::path::{Path, PathBuf};
use std::sync::Arc;

use zarrs::array::{data_type, Array, ArraySubset};
use zarrs::filesystem::FilesystemStore;
use zarrs::plugin::{ExtensionName, ZarrVersion};

use crate::{Error, Result};

use super::{
    normalized_shape, validate_region, RasterDType, RasterDescriptor, RasterProfile, RasterSource,
    RawTile, SourceNormalization,
};
use crate::annotations::parametric_map::profile::RasterAxis;

const MAX_LOCAL_ARRAY_ENTRIES: usize = 1_000_000;
const MAX_LOCAL_ARRAY_DEPTH: usize = 128;

pub(super) struct ZarrSource {
    array: Array<FilesystemStore>,
    descriptor: RasterDescriptor,
    axes: Vec<RasterAxis>,
    normalizations: Vec<SourceNormalization>,
}

impl ZarrSource {
    pub(super) fn open(profile: &RasterProfile, path: &Path) -> Result<Self> {
        let metadata = std::fs::metadata(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(Error::InvalidInput(format!(
                "Zarr input {} is not a local directory store",
                path.display()
            )));
        }
        let canonical = std::fs::canonicalize(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let store = Arc::new(FilesystemStore::new(&canonical).map_err(|error| {
            Error::InvalidInput(format!(
                "could not open local Zarr store {}: {error}",
                canonical.display()
            ))
        })?);
        let declared_path = profile
            .zarr_array_path
            .as_deref()
            .ok_or_else(|| Error::InvalidInput("Zarr profile has no local array path".into()))?;
        validate_local_array_tree(&canonical, declared_path)?;
        let array_path = format!("/{declared_path}");
        let array = Array::open(store, &array_path).map_err(zarr_error)?;
        if array
            .chunk_grid()
            .name(ZarrVersion::V3)
            .is_none_or(|name| name != "regular")
        {
            return Err(Error::Unsupported(
                "only regular Zarr chunk grids are supported".into(),
            ));
        }
        validate_dimension_names(&array, &profile.axes)?;
        let dtype = dtype_from_zarr(array.data_type())?;
        let (height, width, channels) =
            normalized_shape(array.shape(), &profile.axes, profile.channels.len())?;
        let chunk_shape = array
            .chunk_shape(&vec![0; array.shape().len()])
            .map_err(zarr_error)?;
        let tile_height = chunk_axis_length(&chunk_shape, profile, RasterAxis::Y, height)?;
        let tile_width = chunk_axis_length(&chunk_shape, profile, RasterAxis::X, width)?;
        let normalizations = if is_canonical_axis_order(&profile.axes) {
            Vec::new()
        } else {
            vec![SourceNormalization::ZarrAxisOrder]
        };
        Ok(Self {
            array,
            descriptor: RasterDescriptor {
                height,
                width,
                channels,
                dtype,
                tile_height,
                tile_width,
            },
            axes: profile.axes.clone(),
            normalizations,
        })
    }
}

impl RasterSource for ZarrSource {
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
        let (subset, subset_shape) =
            subset_for_region(&self.axes, channel, origin_y, origin_x, height, width)?;
        macro_rules! retrieve {
            ($element:ty, $variant:ident) => {{
                let values = self
                    .array
                    .retrieve_array_subset::<Vec<$element>>(&subset)
                    .map_err(zarr_error)?;
                reorder_region(values, &subset_shape, &self.axes, height, width)
                    .map(RawTile::$variant)
            }};
        }
        match self.descriptor.dtype {
            RasterDType::Float32 => retrieve!(f32, F32),
            RasterDType::Float64 => retrieve!(f64, F64),
            RasterDType::Int8 => retrieve!(i8, I8),
            RasterDType::Int16 => retrieve!(i16, I16),
            RasterDType::Int32 => retrieve!(i32, I32),
            RasterDType::Int64 => retrieve!(i64, I64),
            RasterDType::Uint8 => retrieve!(u8, U8),
            RasterDType::Uint16 => retrieve!(u16, U16),
            RasterDType::Uint32 => retrieve!(u32, U32),
            RasterDType::Uint64 => retrieve!(u64, U64),
        }
    }

    fn normalizations(&self) -> &[SourceNormalization] {
        &self.normalizations
    }
}

fn is_canonical_axis_order(axes: &[RasterAxis]) -> bool {
    matches!(
        axes,
        [RasterAxis::Y, RasterAxis::X] | [RasterAxis::Y, RasterAxis::X, RasterAxis::Channel]
    )
}

fn validate_dimension_names(array: &Array<FilesystemStore>, axes: &[RasterAxis]) -> Result<()> {
    let Some(names) = array.dimension_names() else {
        return Ok(());
    };
    let agree = names.len() == axes.len()
        && names.iter().zip(axes).all(|(name, axis)| {
            name.as_deref()
                == Some(match axis {
                    RasterAxis::Y => "y",
                    RasterAxis::X => "x",
                    RasterAxis::Channel => "channel",
                })
        });
    if !agree {
        return Err(Error::InvalidInput(
            "Zarr dimension names do not agree with the profile axes".into(),
        ));
    }
    Ok(())
}

fn dtype_from_zarr(dtype: &zarrs::array::DataType) -> Result<RasterDType> {
    let supported = [
        (data_type::float32(), RasterDType::Float32),
        (data_type::float64(), RasterDType::Float64),
        (data_type::int8(), RasterDType::Int8),
        (data_type::int16(), RasterDType::Int16),
        (data_type::int32(), RasterDType::Int32),
        (data_type::int64(), RasterDType::Int64),
        (data_type::uint8(), RasterDType::Uint8),
        (data_type::uint16(), RasterDType::Uint16),
        (data_type::uint32(), RasterDType::Uint32),
        (data_type::uint64(), RasterDType::Uint64),
    ];
    supported
        .into_iter()
        .find_map(|(candidate, raster)| (dtype == &candidate).then_some(raster))
        .ok_or_else(|| {
            Error::Unsupported(format!(
                "Zarr data type {dtype} is not a supported numeric scalar"
            ))
        })
}

fn chunk_axis_length(
    shape: &[std::num::NonZeroU64],
    profile: &RasterProfile,
    axis: RasterAxis,
    extent: u32,
) -> Result<u32> {
    let index = profile
        .axis_index(axis)
        .ok_or_else(|| Error::InvalidInput("profile has no required raster axis".into()))?;
    let length = shape
        .get(index)
        .map(|value| value.get())
        .ok_or_else(|| Error::InvalidInput("Zarr chunk shape dimensionality is invalid".into()))?;
    u32::try_from(length.min(u64::from(extent)))
        .map_err(|_| Error::InvalidInput("Zarr chunk dimension exceeds the DICOM UL range".into()))
}

fn subset_for_region(
    axes: &[RasterAxis],
    channel: usize,
    origin_y: u32,
    origin_x: u32,
    height: u32,
    width: u32,
) -> Result<(ArraySubset, Vec<u64>)> {
    let mut start = Vec::with_capacity(axes.len());
    let mut shape = Vec::with_capacity(axes.len());
    for axis in axes {
        match axis {
            RasterAxis::Y => {
                start.push(u64::from(origin_y));
                shape.push(u64::from(height));
            }
            RasterAxis::X => {
                start.push(u64::from(origin_x));
                shape.push(u64::from(width));
            }
            RasterAxis::Channel => {
                start.push(u64::try_from(channel).map_err(|_| {
                    Error::InvalidInput("channel index exceeds the Zarr range".into())
                })?);
                shape.push(1);
            }
        }
    }
    let subset = ArraySubset::new_with_start_shape(start, shape.clone()).map_err(zarr_error)?;
    Ok((subset, shape))
}

fn reorder_region<T: Copy>(
    values: Vec<T>,
    shape: &[u64],
    axes: &[RasterAxis],
    height: u32,
    width: u32,
) -> Result<Vec<T>> {
    let strides = c_strides(shape)?;
    let y_axis = axes
        .iter()
        .position(|axis| *axis == RasterAxis::Y)
        .ok_or_else(|| Error::InvalidInput("profile has no y axis".into()))?;
    let x_axis = axes
        .iter()
        .position(|axis| *axis == RasterAxis::X)
        .ok_or_else(|| Error::InvalidInput("profile has no x axis".into()))?;
    let capacity = usize::try_from(u64::from(height) * u64::from(width)).map_err(|_| {
        Error::InvalidInput("requested Zarr tile is too large for this platform".into())
    })?;
    let mut reordered = Vec::with_capacity(capacity);
    for y in 0..height {
        for x in 0..width {
            let index = u64::from(y)
                .checked_mul(strides[y_axis])
                .and_then(|value| {
                    u64::from(x)
                        .checked_mul(strides[x_axis])
                        .and_then(|x| value.checked_add(x))
                })
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| Error::InvalidInput("Zarr tile index overflows".into()))?;
            reordered.push(
                *values
                    .get(index)
                    .ok_or_else(|| Error::InvalidInput("Zarr subset data is truncated".into()))?,
            );
        }
    }
    Ok(reordered)
}

fn validate_local_array_tree(store_root: &Path, declared_path: &str) -> Result<()> {
    let mut array_root = store_root.to_path_buf();
    for component in declared_path.split('/') {
        array_root.push(component);
        let metadata = std::fs::symlink_metadata(&array_root).map_err(|source| Error::Io {
            path: array_root.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(unsafe_zarr_entry(&array_root, "symbolic link"));
        }
        if !metadata.is_dir() {
            return Err(unsafe_zarr_entry(
                &array_root,
                "non-directory path component",
            ));
        }
    }

    let mut pending = vec![(array_root, 0_usize)];
    let mut entries = 0_usize;
    while let Some((directory, depth)) = pending.pop() {
        if depth > MAX_LOCAL_ARRAY_DEPTH {
            return Err(Error::InvalidInput(format!(
                "local Zarr array exceeds the maximum directory depth of {MAX_LOCAL_ARRAY_DEPTH}"
            )));
        }
        let children = std::fs::read_dir(&directory).map_err(|source| Error::Io {
            path: directory.clone(),
            source,
        })?;
        for child in children {
            let child = child.map_err(|source| Error::Io {
                path: directory.clone(),
                source,
            })?;
            entries = entries
                .checked_add(1)
                .ok_or_else(|| Error::InvalidInput("local Zarr entry count overflows".into()))?;
            if entries > MAX_LOCAL_ARRAY_ENTRIES {
                return Err(Error::InvalidInput(format!(
                    "local Zarr array exceeds the maximum of {MAX_LOCAL_ARRAY_ENTRIES} entries"
                )));
            }
            let path: PathBuf = child.path();
            let kind = child.file_type().map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            if kind.is_symlink() {
                return Err(unsafe_zarr_entry(&path, "symbolic link"));
            }
            if kind.is_dir() {
                pending.push((path, depth + 1));
            } else if !kind.is_file() {
                return Err(unsafe_zarr_entry(&path, "special file"));
            }
        }
    }
    Ok(())
}

fn unsafe_zarr_entry(path: &Path, kind: &str) -> Error {
    Error::InvalidInput(format!(
        "local Zarr array contains a forbidden {kind}: {}",
        path.display()
    ))
}

fn c_strides(shape: &[u64]) -> Result<Vec<u64>> {
    let mut strides = vec![1_u64; shape.len()];
    for index in (0..shape.len().saturating_sub(1)).rev() {
        strides[index] = strides[index + 1]
            .checked_mul(shape[index + 1])
            .ok_or_else(|| Error::InvalidInput("Zarr shape overflows".into()))?;
    }
    Ok(strides)
}

fn zarr_error(error: impl std::fmt::Display) -> Error {
    Error::InvalidInput(format!("could not decode local Zarr array: {error}"))
}

#[cfg(test)]
#[path = "zarr_unit_tests.rs"]
mod tests;
