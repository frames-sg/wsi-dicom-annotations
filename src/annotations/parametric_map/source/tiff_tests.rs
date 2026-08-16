use std::fs::File;
use std::path::Path;

use tiff::encoder::colortype::Gray32Float;
use tiff::encoder::TiffEncoder;
use tiff::tags::Tag;

use super::{NormalizedRaster, PixelTile, RasterProfile};

#[test]
fn reads_stripped_standard_and_big_tiff_without_whole_image_allocation() {
    for big in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("values.tiff");
        let file = File::create(&path).unwrap();
        if big {
            let mut encoder = TiffEncoder::new_big(file).unwrap();
            let mut image = encoder.new_image::<Gray32Float>(3, 4).unwrap();
            image.rows_per_strip(2).unwrap();
            image.write_data(&values(3, 4)).unwrap();
        } else {
            let mut encoder = TiffEncoder::new(file).unwrap();
            let mut image = encoder.new_image::<Gray32Float>(3, 4).unwrap();
            image.rows_per_strip(2).unwrap();
            image.write_data(&values(3, 4)).unwrap();
        }
        let source = open(&path);
        assert_eq!(source.descriptor().tile_height, 2);
        assert_eq!(
            source.read_tile(0, 2, 0, 2, 3).unwrap(),
            PixelTile::F32(vec![6.0, 7.0, 8.0, 9.0, 10.0, 11.0])
        );
        assert_eq!(
            source.read_tile(0, 1, 1, 2, 2).unwrap(),
            PixelTile::F32(vec![4.0, 5.0, 7.0, 8.0])
        );
    }
}

#[test]
fn reads_uncompressed_tiled_tiff_across_tile_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tiled.tiff");
    write_tiled_tiff(&path, 1, 1);

    let source = open(&path);

    assert_eq!(source.descriptor().tile_height, 16);
    assert_eq!(source.descriptor().tile_width, 16);
    assert_eq!(
        source.read_tile(0, 14, 14, 4, 4).unwrap(),
        PixelTile::F32(vec![
            294.0, 295.0, 296.0, 297.0, 314.0, 315.0, 316.0, 317.0, 334.0, 335.0, 336.0, 337.0,
            354.0, 355.0, 356.0, 357.0,
        ])
    );
}

#[test]
fn rejects_unsupported_orientation_and_lossy_compression_before_pixels() {
    for (orientation, compression, expected) in [(3, 1, "Orientation"), (1, 7, "compression")] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid.tiff");
        write_tiled_tiff(&path, orientation, compression);

        let error = match NormalizedRaster::open(&profile(), &path) {
            Ok(_) => panic!("unsupported TIFF metadata should be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains(expected));
    }
}

fn open(path: &Path) -> NormalizedRaster {
    NormalizedRaster::open(&profile(), path).unwrap()
}

fn profile() -> RasterProfile {
    RasterProfile::from_json(PROFILE.as_bytes()).unwrap()
}

fn values(width: u32, height: u32) -> Vec<f32> {
    (0..width * height).map(|value| value as f32).collect()
}

fn write_tiled_tiff(path: &Path, orientation: u16, compression: u16) {
    const WIDTH: u32 = 20;
    const HEIGHT: u32 = 20;
    const TILE: u32 = 16;

    let file = File::create(path).unwrap();
    let mut encoder = TiffEncoder::new(file).unwrap();
    let mut directory = encoder.image_directory().unwrap();
    let mut offsets = Vec::new();
    let mut byte_counts = Vec::new();
    for tile_row in 0..2 {
        for tile_column in 0..2 {
            let mut tile = vec![f32::NAN; (TILE * TILE) as usize];
            for y in 0..TILE {
                for x in 0..TILE {
                    let image_y = tile_row * TILE + y;
                    let image_x = tile_column * TILE + x;
                    if image_y < HEIGHT && image_x < WIDTH {
                        tile[(y * TILE + x) as usize] = (image_y * WIDTH + image_x) as f32;
                    }
                }
            }
            offsets.push(u32::try_from(directory.write_data(tile.as_slice()).unwrap()).unwrap());
            byte_counts.push(u32::try_from(tile.len() * size_of::<f32>()).unwrap());
        }
    }
    directory.write_tag(Tag::ImageWidth, WIDTH).unwrap();
    directory.write_tag(Tag::ImageLength, HEIGHT).unwrap();
    directory.write_tag(Tag::BitsPerSample, 32_u16).unwrap();
    directory.write_tag(Tag::Compression, compression).unwrap();
    directory
        .write_tag(Tag::PhotometricInterpretation, 1_u16)
        .unwrap();
    directory.write_tag(Tag::Orientation, orientation).unwrap();
    directory.write_tag(Tag::SamplesPerPixel, 1_u16).unwrap();
    directory
        .write_tag(Tag::PlanarConfiguration, 1_u16)
        .unwrap();
    directory.write_tag(Tag::TileWidth, TILE).unwrap();
    directory.write_tag(Tag::TileLength, TILE).unwrap();
    directory
        .write_tag(Tag::TileOffsets, offsets.as_slice())
        .unwrap();
    directory
        .write_tag(Tag::TileByteCounts, byte_counts.as_slice())
        .unwrap();
    directory.write_tag(Tag::SampleFormat, 3_u16).unwrap();
    directory.finish().unwrap();
}

const PROFILE: &str = r#"{
  "schema_version":1,
  "input_format":"tiff",
  "dtype":"float32",
  "axes":["y","x"],
  "grid_origin":{"x":0.0,"y":0.0},
  "sample_spacing":{"x":1.0,"y":1.0},
  "coordinate_space":"level0-pixels",
  "channels":[{"name":"value","quantity":{"code_value":"V","coding_scheme_designator":"99T","code_meaning":"Value"},"unit":{"code_value":"1","coding_scheme_designator":"UCUM","code_meaning":"none"}}],
  "algorithm":{"family":{"code_value":"123110","coding_scheme_designator":"DCM","code_meaning":"Artificial Intelligence"},"name":"test","version":"1"}
}"#;
