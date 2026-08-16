use super::*;

fn descriptor(dtype: RasterDType) -> RasterDescriptor {
    RasterDescriptor {
        height: 4,
        width: 4,
        channels: 2,
        dtype,
        tile_height: 2,
        tile_width: 2,
    }
}

#[test]
fn chunk_indices_and_decoded_types_cover_chunky_planar_and_overflow_cases() {
    let chunky = TiffSource {
        path: PathBuf::from("unused.tiff"),
        descriptor: descriptor(RasterDType::Float32),
        chunky: true,
        chunks_across: 2,
        chunks_down: 2,
    };
    assert_eq!(chunky.chunk_storage_index(1, 1, 1).unwrap(), 3);
    let planar = TiffSource {
        chunky: false,
        ..chunky
    };
    assert_eq!(planar.chunk_storage_index(1, 1, 1).unwrap(), 7);
    let overflowing = TiffSource {
        chunks_across: u32::MAX,
        chunks_down: u32::MAX,
        ..planar
    };
    assert!(overflowing.chunk_storage_index(usize::MAX, 2, 1).is_err());

    for (decoded, expected) in [
        (DecodingResult::F32(Vec::new()), RasterDType::Float32),
        (DecodingResult::F64(Vec::new()), RasterDType::Float64),
        (DecodingResult::I8(Vec::new()), RasterDType::Int8),
        (DecodingResult::I16(Vec::new()), RasterDType::Int16),
        (DecodingResult::I32(Vec::new()), RasterDType::Int32),
        (DecodingResult::I64(Vec::new()), RasterDType::Int64),
        (DecodingResult::U8(Vec::new()), RasterDType::Uint8),
        (DecodingResult::U16(Vec::new()), RasterDType::Uint16),
        (DecodingResult::U32(Vec::new()), RasterDType::Uint32),
        (DecodingResult::U64(Vec::new()), RasterDType::Uint64),
    ] {
        assert_eq!(decoded_dtype(&decoded).unwrap(), expected);
    }
    assert!(decoded_dtype(&DecodingResult::F16(Vec::new())).is_err());
}

#[test]
fn channel_selection_and_empty_tiles_preserve_every_numeric_variant() {
    let RawTile::F32(values) =
        select_channel(DecodingResult::F32(vec![1.0, 10.0, 2.0, 20.0]), 1, 2, true).unwrap()
    else {
        panic!("float32 decoding must remain float32");
    };
    assert_eq!(values, vec![10.0, 20.0]);
    let RawTile::U16(values) =
        select_channel(DecodingResult::U16(vec![1, 2]), 1, 2, false).unwrap()
    else {
        panic!("planar uint16 decoding must remain uint16");
    };
    assert_eq!(values, vec![1, 2]);
    assert!(select_channel(DecodingResult::F16(Vec::new()), 0, 1, true).is_err());

    for tile in [
        empty_raw_tile(RasterDType::Float32, 2),
        empty_raw_tile(RasterDType::Float64, 2),
        empty_raw_tile(RasterDType::Int8, 2),
        empty_raw_tile(RasterDType::Int16, 2),
        empty_raw_tile(RasterDType::Int32, 2),
        empty_raw_tile(RasterDType::Int64, 2),
        empty_raw_tile(RasterDType::Uint8, 2),
        empty_raw_tile(RasterDType::Uint16, 2),
        empty_raw_tile(RasterDType::Uint32, 2),
        empty_raw_tile(RasterDType::Uint64, 2),
    ] {
        let length = match tile {
            RawTile::F32(values) => values.len(),
            RawTile::F64(values) => values.len(),
            RawTile::I8(values) => values.len(),
            RawTile::I16(values) => values.len(),
            RawTile::I32(values) => values.len(),
            RawTile::I64(values) => values.len(),
            RawTile::U8(values) => values.len(),
            RawTile::U16(values) => values.len(),
            RawTile::U32(values) => values.len(),
            RawTile::U64(values) => values.len(),
        };
        assert_eq!(length, 2);
    }
}

#[test]
fn copy_windows_reject_type_disagreement_truncated_chunks_and_extent_overflow() {
    let mut output = RawTile::F32(vec![0.0; 4]);
    assert!(copy_chunk_intersection(
        &mut output,
        RawTile::U8(vec![1; 4]),
        descriptor(RasterDType::Float32),
        ChunkPosition { row: 0, column: 0 },
        Region {
            y: 0,
            x: 0,
            height: 2,
            width: 2,
        },
    )
    .is_err());
    assert!(copy_chunk_intersection(
        &mut output,
        RawTile::F32(vec![1.0; 4]),
        descriptor(RasterDType::Float32),
        ChunkPosition { row: 0, column: 0 },
        Region {
            y: u32::MAX,
            x: 0,
            height: 1,
            width: 1,
        },
    )
    .is_err());

    let window = CopyWindow {
        source_width: 2,
        destination_width: 2,
        source_y: 0,
        source_x: 0,
        destination_y: 0,
        destination_x: 0,
        height: 1,
        width: 2,
    };
    assert!(copy_values(&mut [0_u8; 2], &[1_u8], window).is_err());
    assert!(copy_values(&mut [0_u8; 1], &[1_u8, 2], window).is_err());
    assert!(open_decoder(Path::new("missing-raster.tiff")).is_err());
}
