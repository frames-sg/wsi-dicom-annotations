use std::fs;
use std::path::Path;
use std::sync::Arc;

use zarrs::array::codec::api::BytesToBytesCodecTraits;
use zarrs::array::codec::{BloscCodec, GzipCodec, ZstdCodec};
use zarrs::array::{data_type, ArrayBuilder};
use zarrs::filesystem::FilesystemStore;
use zarrs::metadata_ext::codec::blosc::{BloscCompressionLevel, BloscCompressor, BloscShuffleMode};

use super::{NormalizedRaster, PixelTile, RasterProfile};

const VALUES: [f32; 12] = [
    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
];

#[test]
fn reads_v2_and_v3_channels_in_declared_axis_order() {
    let v2 = tempfile::tempdir().unwrap();
    write_v2(v2.path());
    assert_second_channel(v2.path());

    let v3 = tempfile::tempdir().unwrap();
    write_v3(v3.path(), Vec::new(), false, Some(["channel", "y", "x"]));
    assert_second_channel(v3.path());
}

#[test]
fn reads_enabled_common_codecs_and_sharding() {
    let codecs: Vec<(&str, Arc<dyn BytesToBytesCodecTraits>)> = vec![
        ("gzip", Arc::new(GzipCodec::new(5).unwrap())),
        ("zstd", Arc::new(ZstdCodec::new(3, true))),
        (
            "blosc",
            Arc::new(
                BloscCodec::new(
                    BloscCompressor::Zstd,
                    BloscCompressionLevel::try_from(5).unwrap(),
                    None,
                    BloscShuffleMode::BitShuffle,
                    Some(size_of::<f32>()),
                )
                .unwrap(),
            ),
        ),
    ];
    for (name, codec) in codecs {
        let directory = tempfile::tempdir().unwrap();
        write_v3(directory.path(), vec![codec], false, None);
        assert_eq!(
            read_second_channel(directory.path()),
            expected(),
            "codec {name}"
        );
    }

    let directory = tempfile::tempdir().unwrap();
    write_v3(directory.path(), Vec::new(), true, None);
    assert_eq!(
        read_second_channel(directory.path()),
        expected(),
        "sharding"
    );
}

#[test]
fn rejects_dimension_name_disagreement() {
    let directory = tempfile::tempdir().unwrap();
    write_v3(directory.path(), Vec::new(), false, Some(["z", "y", "x"]));

    let error = open_error(directory.path());

    assert!(error.contains("dimension names"));
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_within_the_declared_local_array() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write_v3(directory.path(), Vec::new(), false, None);
    fs::write(outside.path().join("chunk"), b"untrusted").unwrap();
    symlink(
        outside.path().join("chunk"),
        directory.path().join("probabilities/escape"),
    )
    .unwrap();

    assert!(open_error(directory.path()).contains("symbolic link"));
}

fn write_v2(root: &Path) {
    let array = root.join("probabilities");
    fs::create_dir(&array).unwrap();
    fs::write(
        array.join(".zarray"),
        r#"{"zarr_format":2,"shape":[2,2,3],"chunks":[2,2,3],"dtype":"<f4","compressor":null,"fill_value":"NaN","order":"C","filters":null}"#,
    )
    .unwrap();
    let bytes = VALUES
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    fs::write(array.join("0.0.0"), bytes).unwrap();
}

fn write_v3(
    root: &Path,
    codecs: Vec<Arc<dyn BytesToBytesCodecTraits>>,
    sharded: bool,
    dimensions: Option<[&str; 3]>,
) {
    let store = Arc::new(FilesystemStore::new(root).unwrap());
    let mut builder =
        ArrayBuilder::new(vec![2, 2, 3], vec![2, 2, 3], data_type::float32(), f32::NAN);
    builder.bytes_to_bytes_codecs(codecs);
    if sharded {
        builder.subchunk_shape(vec![1, 1, 3]);
    }
    if let Some(dimensions) = dimensions {
        builder.dimension_names(Some(dimensions));
    }
    let array = builder.build(store, "/probabilities").unwrap();
    array.store_metadata().unwrap();
    array
        .store_array_subset(&array.subset_all(), &VALUES)
        .unwrap();
}

fn assert_second_channel(path: &Path) {
    let source = NormalizedRaster::open(&profile(), path).unwrap();
    assert!(source
        .normalizations()
        .iter()
        .any(|diagnostic| diagnostic.code() == "ZARR_AXIS_ORDER_NORMALIZED"));
    assert_eq!(source.descriptor().tile_height, 2);
    assert_eq!(source.descriptor().tile_width, 3);
    assert_eq!(source.read_tile(1, 0, 1, 2, 2).unwrap(), expected());
}

fn read_second_channel(path: &Path) -> PixelTile {
    NormalizedRaster::open(&profile(), path)
        .unwrap()
        .read_tile(1, 0, 1, 2, 2)
        .unwrap()
}

fn expected() -> PixelTile {
    PixelTile::F32(vec![11.0, 12.0, 14.0, 15.0])
}

fn open_error(path: &Path) -> String {
    match NormalizedRaster::open(&profile(), path) {
        Ok(_) => panic!("invalid Zarr input should be rejected"),
        Err(error) => error.to_string(),
    }
}

fn profile() -> RasterProfile {
    RasterProfile::from_json(PROFILE.as_bytes()).unwrap()
}

const PROFILE: &str = r#"{
  "schema_version":1,
  "input_format":"zarr",
  "dtype":"float32",
  "axes":["channel","y","x"],
  "grid_origin":{"x":0.0,"y":0.0},
  "sample_spacing":{"x":1.0,"y":1.0},
  "coordinate_space":"level0-pixels",
  "channels":[
    {"name":"first","quantity":{"code_value":"A","coding_scheme_designator":"99T","code_meaning":"A"},"unit":{"code_value":"1","coding_scheme_designator":"UCUM","code_meaning":"none"}},
    {"name":"second","quantity":{"code_value":"B","coding_scheme_designator":"99T","code_meaning":"B"},"unit":{"code_value":"1","coding_scheme_designator":"UCUM","code_meaning":"none"}}
  ],
  "algorithm":{"family":{"code_value":"123110","coding_scheme_designator":"DCM","code_meaning":"Artificial Intelligence"},"name":"test","version":"1"},
  "zarr_array_path":"probabilities"
}"#;
