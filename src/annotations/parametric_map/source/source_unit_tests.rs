use super::*;

#[test]
fn normalization_diagnostics_and_shapes_report_each_declared_semantic() {
    let expected = [
        (
            SourceNormalization::NpyFortranOrder,
            "NPY_FORTRAN_ORDER_NORMALIZED",
        ),
        (
            SourceNormalization::ZarrAxisOrder,
            "ZARR_AXIS_ORDER_NORMALIZED",
        ),
        (
            SourceNormalization::ManifestValidRegionCrop,
            "TILED_MANIFEST_VALID_REGION_CROP",
        ),
        (
            SourceNormalization::ManifestOverlapMean,
            "TILED_MANIFEST_OVERLAP_MEAN",
        ),
        (
            SourceNormalization::ManifestOverlapMax,
            "TILED_MANIFEST_OVERLAP_MAX",
        ),
        (
            SourceNormalization::ManifestOrderLastWrite,
            "TILED_MANIFEST_ORDER_LAST_WRITE",
        ),
    ];
    for (normalization, code) in expected {
        assert_eq!(normalization.diagnostic().code(), code);
    }

    assert_eq!(
        normalized_shape(&[3, 4], &[RasterAxis::Y, RasterAxis::X], 1).unwrap(),
        (3, 4, 1)
    );
    assert!(normalized_shape(&[3], &[RasterAxis::Y, RasterAxis::X], 1).is_err());
    assert!(normalized_shape(
        &[u64::from(u32::MAX) + 1, 4],
        &[RasterAxis::Y, RasterAxis::X],
        1,
    )
    .is_err());
    assert!(normalized_shape(
        &[3, u64::from(u32::MAX) + 1],
        &[RasterAxis::Y, RasterAxis::X],
        1,
    )
    .is_err());
    assert!(normalized_shape(&[0, 4], &[RasterAxis::Y, RasterAxis::X], 1).is_err());
    assert!(normalized_shape(
        &[2, 3, 4],
        &[RasterAxis::Channel, RasterAxis::Y, RasterAxis::X],
        1,
    )
    .is_err());
}

#[test]
fn every_raw_dtype_normalizes_or_rejects_missing_and_nonfinite_values_explicitly() {
    let f32_tile = normalize_tile(RawTile::F32(vec![f32::NAN, 1.0]), None).unwrap();
    let PixelTile::F32(values) = f32_tile else {
        panic!("float32 input must remain float32");
    };
    assert_eq!(values[0].to_bits(), CANONICAL_NAN_F32.to_bits());
    assert!(normalize_tile(RawTile::F32(vec![f32::INFINITY]), None).is_err());

    let f64_tile = normalize_tile(RawTile::F64(vec![f64::NAN, 1.0]), None).unwrap();
    let PixelTile::F64(values) = f64_tile else {
        panic!("float64 input must remain float64");
    };
    assert_eq!(values[0].to_bits(), CANONICAL_NAN_F64.to_bits());
    assert!(normalize_tile(RawTile::F64(vec![f64::NEG_INFINITY]), None).is_err());

    let signed = IntegerScaling {
        slope: 2.0,
        intercept: 1.0,
        missing_sentinel: IntegerSentinel::Signed(-1),
        output_precision: RasterOutputPrecision::Float64,
    };
    for tile in [
        RawTile::I8(vec![-1, 2]),
        RawTile::I16(vec![-1, 2]),
        RawTile::I32(vec![-1, 2]),
        RawTile::I64(vec![-1, 2]),
    ] {
        let PixelTile::F64(values) = normalize_tile(tile, Some(&signed)).unwrap() else {
            panic!("declared float64 scaling must produce float64 values");
        };
        assert!(values[0].is_nan());
        assert_eq!(values[1], 5.0);
    }
    let unsigned = IntegerScaling {
        missing_sentinel: IntegerSentinel::Unsigned(255),
        output_precision: RasterOutputPrecision::Float32,
        ..signed.clone()
    };
    for tile in [
        RawTile::U8(vec![2]),
        RawTile::U16(vec![2]),
        RawTile::U32(vec![2]),
        RawTile::U64(vec![2]),
    ] {
        let PixelTile::F32(values) = normalize_tile(tile, Some(&unsigned)).unwrap() else {
            panic!("declared float32 scaling must produce float32 values");
        };
        assert_eq!(values, vec![5.0]);
    }
    assert!(normalize_tile(RawTile::I8(vec![1]), None).is_err());
    assert!(normalize_tile(RawTile::U8(vec![1]), None).is_err());
    assert!(normalize_tile(RawTile::I8(vec![1]), Some(&unsigned)).is_err());
    let negative_unsigned = IntegerScaling {
        missing_sentinel: IntegerSentinel::Signed(-1),
        ..unsigned
    };
    assert!(normalize_tile(RawTile::U8(vec![1]), Some(&negative_unsigned)).is_err());
}

#[test]
fn sentinel_validation_covers_each_integer_width_and_scaling_overflow() {
    let signed = IntegerScaling {
        slope: 1.0,
        intercept: 0.0,
        missing_sentinel: IntegerSentinel::Signed(-1),
        output_precision: RasterOutputPrecision::Float64,
    };
    for dtype in [
        RasterDType::Int8,
        RasterDType::Int16,
        RasterDType::Int32,
        RasterDType::Int64,
    ] {
        validate_integer_sentinel(dtype, Some(&signed)).unwrap();
    }
    let unsigned = IntegerScaling {
        missing_sentinel: IntegerSentinel::Unsigned(1),
        ..signed.clone()
    };
    for dtype in [
        RasterDType::Uint8,
        RasterDType::Uint16,
        RasterDType::Uint32,
        RasterDType::Uint64,
    ] {
        validate_integer_sentinel(dtype, Some(&unsigned)).unwrap();
    }
    let too_large = IntegerScaling {
        missing_sentinel: IntegerSentinel::Unsigned(u64::MAX),
        ..signed.clone()
    };
    assert!(validate_integer_sentinel(RasterDType::Uint8, Some(&too_large)).is_err());
    assert!(validate_integer_sentinel(RasterDType::Float32, Some(&signed)).is_err());
    assert_eq!(unsigned_sentinel(IntegerSentinel::Signed(-1)), None);
    assert_eq!(unsigned_sentinel(IntegerSentinel::Unsigned(9)), Some(9));
    assert!(matches!(missing_scaling(), Error::InvalidInput(_)));

    let overflow_f32 = IntegerScaling {
        slope: f64::MAX,
        intercept: 0.0,
        missing_sentinel: IntegerSentinel::Signed(-1),
        output_precision: RasterOutputPrecision::Float32,
    };
    assert!(normalize_tile(RawTile::I64(vec![i64::MAX]), Some(&overflow_f32)).is_err());
    let overflow_f64 = IntegerScaling {
        output_precision: RasterOutputPrecision::Float64,
        ..overflow_f32
    };
    assert!(normalize_tile(RawTile::I64(vec![i64::MAX]), Some(&overflow_f64)).is_err());
}
