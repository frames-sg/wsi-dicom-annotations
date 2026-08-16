use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use npyz::{DType, NpyFile, TypeChar};

use crate::{Error, Result};

use super::{
    default_tile_length, normalized_shape, validate_region, RasterDType, RasterDescriptor,
    RasterProfile, RasterSource, RawTile, SourceNormalization,
};
use crate::annotations::parametric_map::profile::RasterAxis;

pub(super) struct NpySource {
    path: PathBuf,
    descriptor: RasterDescriptor,
    strides: Vec<u64>,
    y_axis: usize,
    x_axis: usize,
    channel_axis: Option<usize>,
    normalizations: Vec<SourceNormalization>,
}

impl NpySource {
    pub(super) fn open(profile: &RasterProfile, path: &Path) -> Result<Self> {
        let metadata = std::fs::metadata(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(Error::InvalidInput(format!(
                "NPY input {} is not a regular file",
                path.display()
            )));
        }
        let npy = open_file(path)?;
        if npy.uses_pickled_array() {
            return Err(Error::InvalidInput(
                "pickled/object NPY arrays are forbidden".into(),
            ));
        }
        let dtype = dtype_from_npy(&npy.dtype())?;
        let (height, width, channels) =
            normalized_shape(npy.shape(), &profile.axes, profile.channels.len())?;
        let y_axis = profile.axis_index(RasterAxis::Y).ok_or_else(axis_error)?;
        let x_axis = profile.axis_index(RasterAxis::X).ok_or_else(axis_error)?;
        let channel_axis = profile.axis_index(RasterAxis::Channel);
        let normalizations = if is_c_contiguous(npy.shape(), npy.strides())? {
            Vec::new()
        } else {
            vec![SourceNormalization::NpyFortranOrder]
        };
        Ok(Self {
            path: path.to_path_buf(),
            descriptor: RasterDescriptor {
                height,
                width,
                channels,
                dtype,
                tile_height: default_tile_length(height),
                tile_width: default_tile_length(width),
            },
            strides: npy.strides().to_vec(),
            y_axis,
            x_axis,
            channel_axis,
            normalizations,
        })
    }

    fn flat_indices(
        &self,
        channel: usize,
        origin_y: u32,
        origin_x: u32,
        height: u32,
        width: u32,
    ) -> Result<Vec<u64>> {
        validate_region(self.descriptor, channel, origin_y, origin_x, height, width)?;
        let count = usize::try_from(u64::from(height) * u64::from(width)).map_err(|_| {
            Error::InvalidInput("requested NPY tile is too large for this platform".into())
        })?;
        let mut indices = Vec::with_capacity(count);
        for y in origin_y..origin_y + height {
            for x in origin_x..origin_x + width {
                indices.push(flat_index(
                    &self.strides,
                    self.y_axis,
                    self.x_axis,
                    self.channel_axis,
                    channel,
                    y,
                    x,
                )?);
            }
        }
        Ok(indices)
    }
}

fn flat_index(
    strides: &[u64],
    y_axis: usize,
    x_axis: usize,
    channel_axis: Option<usize>,
    channel: usize,
    y: u32,
    x: u32,
) -> Result<u64> {
    let stride = |axis| {
        strides
            .get(axis)
            .copied()
            .ok_or_else(|| Error::InvalidInput("NPY stride dimensionality is invalid".into()))
    };
    let y_offset = u64::from(y)
        .checked_mul(stride(y_axis)?)
        .ok_or_else(index_overflow)?;
    let x_offset = u64::from(x)
        .checked_mul(stride(x_axis)?)
        .ok_or_else(index_overflow)?;
    let mut index = y_offset.checked_add(x_offset).ok_or_else(index_overflow)?;
    if let Some(axis) = channel_axis {
        let channel = u64::try_from(channel).map_err(|_| index_overflow())?;
        let channel_offset = channel
            .checked_mul(stride(axis)?)
            .ok_or_else(index_overflow)?;
        index = index
            .checked_add(channel_offset)
            .ok_or_else(index_overflow)?;
    }
    Ok(index)
}

fn index_overflow() -> Error {
    Error::InvalidInput("NPY element index overflows".into())
}

impl RasterSource for NpySource {
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
        let indices = self.flat_indices(channel, origin_y, origin_x, height, width)?;
        let npy = open_file(&self.path)?;
        match self.descriptor.dtype {
            RasterDType::Float32 => read_values::<f32>(npy, &indices).map(RawTile::F32),
            RasterDType::Float64 => read_values::<f64>(npy, &indices).map(RawTile::F64),
            RasterDType::Int8 => read_values::<i8>(npy, &indices).map(RawTile::I8),
            RasterDType::Int16 => read_values::<i16>(npy, &indices).map(RawTile::I16),
            RasterDType::Int32 => read_values::<i32>(npy, &indices).map(RawTile::I32),
            RasterDType::Int64 => read_values::<i64>(npy, &indices).map(RawTile::I64),
            RasterDType::Uint8 => read_values::<u8>(npy, &indices).map(RawTile::U8),
            RasterDType::Uint16 => read_values::<u16>(npy, &indices).map(RawTile::U16),
            RasterDType::Uint32 => read_values::<u32>(npy, &indices).map(RawTile::U32),
            RasterDType::Uint64 => read_values::<u64>(npy, &indices).map(RawTile::U64),
        }
    }

    fn normalizations(&self) -> &[SourceNormalization] {
        &self.normalizations
    }
}

fn is_c_contiguous(shape: &[u64], strides: &[u64]) -> Result<bool> {
    if shape.len() != strides.len() {
        return Err(Error::InvalidInput(
            "NPY shape and stride dimensionality disagree".into(),
        ));
    }
    let mut expected = 1_u64;
    for (&dimension, &stride) in shape.iter().zip(strides).rev() {
        if dimension > 1 && stride != expected {
            return Ok(false);
        }
        expected = expected
            .checked_mul(dimension)
            .ok_or_else(|| Error::InvalidInput("NPY shape overflows".into()))?;
    }
    Ok(true)
}

fn open_file(path: &Path) -> Result<NpyFile<BufReader<File>>> {
    let file = File::open(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    NpyFile::new(BufReader::new(file)).map_err(|error| {
        Error::InvalidInput(format!("could not parse NPY {}: {error}", path.display()))
    })
}

fn dtype_from_npy(dtype: &DType) -> Result<RasterDType> {
    let DType::Plain(dtype) = dtype else {
        return Err(Error::InvalidInput(
            "structured or nested NPY dtypes are forbidden".into(),
        ));
    };
    match (dtype.type_char(), dtype.size_field()) {
        (TypeChar::Float, 4) => Ok(RasterDType::Float32),
        (TypeChar::Float, 8) => Ok(RasterDType::Float64),
        (TypeChar::Int, 1) => Ok(RasterDType::Int8),
        (TypeChar::Int, 2) => Ok(RasterDType::Int16),
        (TypeChar::Int, 4) => Ok(RasterDType::Int32),
        (TypeChar::Int, 8) => Ok(RasterDType::Int64),
        (TypeChar::Uint, 1) => Ok(RasterDType::Uint8),
        (TypeChar::Uint, 2) => Ok(RasterDType::Uint16),
        (TypeChar::Uint, 4) => Ok(RasterDType::Uint32),
        (TypeChar::Uint, 8) => Ok(RasterDType::Uint64),
        (TypeChar::Complex, _) => Err(Error::InvalidInput(
            "complex NPY arrays are forbidden".into(),
        )),
        (TypeChar::Object, _) => Err(Error::InvalidInput(
            "pickled/object NPY arrays are forbidden".into(),
        )),
        _ => Err(Error::Unsupported(format!(
            "NPY dtype {} is not a supported numeric scalar",
            dtype
        ))),
    }
}

fn read_values<T>(npy: NpyFile<BufReader<File>>, indices: &[u64]) -> Result<Vec<T>>
where
    T: npyz::Deserialize + Copy,
{
    let mut reader = npy
        .data::<T>()
        .map_err(|error| Error::InvalidInput(format!("NPY dtype could not be decoded: {error}")))?;
    let mut values = Vec::with_capacity(indices.len());
    for &index in indices {
        reader.seek_to(index).map_err(|error| {
            Error::InvalidInput(format!("could not seek within NPY data: {error}"))
        })?;
        values.push(
            reader
                .next()
                .transpose()
                .map_err(|error| {
                    Error::InvalidInput(format!("NPY data is truncated or invalid: {error}"))
                })?
                .ok_or_else(|| {
                    Error::InvalidInput("NPY data ended before its declared shape".into())
                })?,
        );
    }
    Ok(values)
}

fn axis_error() -> Error {
    Error::InvalidInput("profile has no required raster axis".into())
}

#[cfg(test)]
pub(super) fn write_test_array<T>(path: &Path, values: &[T])
where
    T: npyz::Serialize + npyz::AutoSerialize + Copy,
{
    use std::io::BufWriter;

    use npyz::{WriteOptions, WriterBuilder};

    let file = BufWriter::new(File::create(path).unwrap());
    let mut writer = WriteOptions::<T>::new()
        .default_dtype()
        .shape(&[2, 2])
        .writer(file)
        .begin_nd()
        .unwrap();
    writer.extend(values.iter().copied()).unwrap();
    writer.finish().unwrap();
}

#[cfg(test)]
mod tests {
    use std::io::BufWriter;

    use npyz::{Order, WriteOptions, WriterBuilder};

    use super::*;
    use crate::annotations::parametric_map::RasterChannelSelection;

    #[test]
    fn reads_c_and_fortran_npy_in_profile_axis_order() {
        for order in [Order::C, Order::Fortran] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("values.npy");
            let file = BufWriter::new(File::create(&path).unwrap());
            let mut writer = WriteOptions::<f32>::new()
                .default_dtype()
                .shape(&[2, 3, 2])
                .order(order)
                .writer(file)
                .begin_nd()
                .unwrap();
            let values = match order {
                Order::C => vec![
                    0.0, 10.0, 1.0, 11.0, 2.0, 12.0, 3.0, 13.0, 4.0, 14.0, 5.0, 15.0,
                ],
                Order::Fortran => vec![
                    0.0, 3.0, 1.0, 4.0, 2.0, 5.0, 10.0, 13.0, 11.0, 14.0, 12.0, 15.0,
                ],
            };
            writer.extend(values).unwrap();
            writer.finish().unwrap();
            let profile =
                RasterProfile::from_json(profile_json("float32", "", true).as_bytes()).unwrap();
            let selected = profile
                .select_channels(RasterChannelSelection::Name("second".into()))
                .unwrap();
            let source = super::super::NormalizedRaster::open(&profile, &path).unwrap();

            let tile = source.read_tile(selected[0], 0, 1, 2, 2).unwrap();

            assert_eq!(
                tile,
                super::super::PixelTile::F32(vec![11.0, 12.0, 14.0, 15.0])
            );
            assert_eq!(
                source
                    .normalizations()
                    .iter()
                    .any(|diagnostic| diagnostic.code() == "NPY_FORTRAN_ORDER_NORMALIZED"),
                order == Order::Fortran
            );
        }
    }

    #[test]
    fn scales_integer_missing_values_and_rejects_infinity() {
        let directory = tempfile::tempdir().unwrap();
        let integer_path = directory.path().join("integer.npy");
        write_test_array(&integer_path, &[1_i16, -1, 3, 4]);
        let scaling = r#", "integer_scaling":{"slope":0.5,"intercept":1.0,"missing_sentinel":-1,"output_precision":"float32"}"#;
        let profile =
            RasterProfile::from_json(profile_json("int16", scaling, false).as_bytes()).unwrap();
        let source = super::super::NormalizedRaster::open(&profile, &integer_path).unwrap();
        let super::super::PixelTile::F32(values) = source.read_tile(0, 0, 0, 1, 2).unwrap() else {
            panic!("expected float32 output");
        };
        assert_eq!(values[0], 1.5);
        assert_eq!(values[1].to_bits(), 0x7fc0_0000);

        let float_path = directory.path().join("infinite.npy");
        write_test_array(&float_path, &[f32::INFINITY, 0.0, 0.0, 0.0]);
        let profile =
            RasterProfile::from_json(profile_json("float32", "", false).as_bytes()).unwrap();
        let source = super::super::NormalizedRaster::open(&profile, &float_path).unwrap();
        assert!(source.read_tile(0, 0, 0, 1, 1).is_err());
    }

    #[test]
    fn rejects_element_index_overflow() {
        let error = flat_index(&[u64::MAX, 1], 0, 1, None, 0, 2, 0).unwrap_err();

        assert!(error.to_string().contains("index overflows"));
    }

    #[test]
    fn index_stride_and_file_helpers_reject_each_malformed_boundary() {
        assert!(flat_index(&[], 0, 1, None, 0, 0, 0).is_err());
        assert!(flat_index(&[1, u64::MAX], 0, 1, None, 0, 0, 2).is_err());
        assert!(flat_index(&[u64::MAX, 1], 0, 1, None, 0, 1, 1).is_err());
        assert!(flat_index(&[1, 1, u64::MAX], 0, 1, Some(2), 2, 0, 0).is_err());
        assert!(matches!(index_overflow(), Error::InvalidInput(_)));
        assert!(matches!(axis_error(), Error::InvalidInput(_)));

        assert!(is_c_contiguous(&[2, 2], &[2]).is_err());
        assert!(!is_c_contiguous(&[2, 2], &[1, 2]).unwrap());
        assert!(is_c_contiguous(&[u64::MAX, 2], &[2, 1]).is_err());
        assert!(is_c_contiguous(&[2, 3], &[3, 1]).unwrap());

        let directory = tempfile::tempdir().unwrap();
        assert!(open_file(&directory.path().join("missing.npy")).is_err());
        let invalid = directory.path().join("invalid.npy");
        std::fs::write(&invalid, b"not an npy array").unwrap();
        assert!(open_file(&invalid).is_err());
    }

    fn profile_json(dtype: &str, scaling: &str, multichannel: bool) -> String {
        format!(
            r#"{{
              "schema_version":1,
              "input_format":"npy",
              "dtype":"{dtype}",
              "axes":["y","x"{}],
              "grid_origin":{{"x":0.0,"y":0.0}},
              "sample_spacing":{{"x":1.0,"y":1.0}},
              "coordinate_space":"level0-pixels",
              "channels":[{{"name":"first","quantity":{{"code_value":"A","coding_scheme_designator":"99T","code_meaning":"A"}},"unit":{{"code_value":"1","coding_scheme_designator":"UCUM","code_meaning":"none"}}}}{}],
              "algorithm":{{"family":{{"code_value":"123110","coding_scheme_designator":"DCM","code_meaning":"Artificial Intelligence"}},"name":"test","version":"1"}}
              {scaling}
            }}"#,
            if multichannel { ",\"channel\"" } else { "" },
            if multichannel {
                ",{\"name\":\"second\",\"quantity\":{\"code_value\":\"B\",\"coding_scheme_designator\":\"99T\",\"code_meaning\":\"B\"},\"unit\":{\"code_value\":\"1\",\"coding_scheme_designator\":\"UCUM\",\"code_meaning\":\"none\"}}"
            } else {
                ""
            },
        )
    }
}
