use super::*;

#[test]
fn region_and_composite_dispatch_preserve_intersections_and_mean_semantics() {
    let first = Region {
        y: 1,
        x: 2,
        height: 3,
        width: 4,
    };
    let second = Region {
        y: 2,
        x: 4,
        height: 4,
        width: 4,
    };
    assert_eq!(first.bottom(), 4);
    assert_eq!(first.right(), 6);
    let overlap = first.intersection(second).unwrap();
    assert_eq!(
        (overlap.y, overlap.x, overlap.height, overlap.width),
        (2, 4, 2, 2)
    );

    assert_eq!(default_overlap_policy(), OverlapPolicy::Reject);
    assert!(OverlapPolicy::Reject.normalization().is_none());
    assert_eq!(
        OverlapPolicy::Mean.normalization(),
        Some(SourceNormalization::ManifestOverlapMean)
    );

    let requested = Region {
        y: 0,
        x: 0,
        height: 1,
        width: 2,
    };
    let mut composite = CompositeTile::new(RasterOutputPrecision::Float64, 2);
    composite
        .merge(
            PixelTile::F64(vec![2.0, 4.0]),
            requested,
            requested,
            OverlapPolicy::Mean,
        )
        .unwrap();
    composite
        .merge(
            PixelTile::F64(vec![4.0, 8.0]),
            requested,
            requested,
            OverlapPolicy::Mean,
        )
        .unwrap();
    let RawTile::F64(values) = composite.finish(OverlapPolicy::Mean).unwrap() else {
        panic!("float64 composite must preserve float64 output precision");
    };
    assert_eq!(values, vec![3.0, 6.0]);
}

#[test]
fn composite_and_manifest_helpers_reject_truncation_overflow_and_unsafe_paths() {
    let region = Region {
        y: 0,
        x: 0,
        height: 1,
        width: 1,
    };
    assert!(merge_values::<f64>(
        &mut [f64::NAN],
        &mut [0],
        &[],
        region,
        region,
        OverlapPolicy::Reject,
    )
    .is_err());
    assert!(merge_values(
        &mut [],
        &mut [0],
        &[1.0_f64],
        region,
        region,
        OverlapPolicy::Reject,
    )
    .is_err());
    assert!(merge_values(
        &mut [f64::NAN],
        &mut [],
        &[1.0_f64],
        region,
        region,
        OverlapPolicy::Reject,
    )
    .is_err());
    assert!(merge_values(
        &mut [1.0_f64],
        &mut [u32::MAX],
        &[2.0_f64],
        region,
        region,
        OverlapPolicy::ManifestOrderLastWrite,
    )
    .is_err());
    assert!(merge_values(
        &mut [1.0_f64],
        &mut [u32::MAX],
        &[2.0_f64],
        region,
        region,
        OverlapPolicy::Mean,
    )
    .is_err());
    assert!(finish_mean(&mut [f64::NAN], &[2]).is_err());
    assert!(matches!(overlap_count_error(), Error::InvalidInput(_)));

    let directory = tempfile::tempdir().unwrap();
    assert!(read_manifest(&directory.path().join("missing.json")).is_err());
    assert!(read_manifest(directory.path()).is_err());
    let invalid = directory.path().join("invalid.json");
    std::fs::write(&invalid, b"{").unwrap();
    assert!(read_manifest(&invalid).is_err());
    for declared in ["", "/absolute.npy", "../escape.npy", "a/../escape.npy"] {
        assert!(resolve_tile_path(directory.path(), declared).is_err());
    }
    assert!(resolve_tile_path(directory.path(), "missing.npy").is_err());
}

#[test]
fn effective_regions_check_crop_requirements_and_every_coordinate_overflow() {
    let descriptor = RasterDescriptor {
        height: 2,
        width: 2,
        channels: 1,
        dtype: RasterDType::Float32,
        tile_height: 2,
        tile_width: 2,
    };
    let origin = SampleGridOrigin { y: 0, x: 0 };
    assert!(effective_region(origin, None, descriptor, OverlapPolicy::ValidRegionCrop).is_err());
    assert!(effective_region(
        origin,
        Some(ValidRegion {
            y: 0,
            x: 0,
            height: 0,
            width: 1,
        }),
        descriptor,
        OverlapPolicy::Reject,
    )
    .is_err());
    let unit = Some(ValidRegion {
        y: 1,
        x: 1,
        height: 1,
        width: 1,
    });
    assert!(effective_region(
        SampleGridOrigin { y: u32::MAX, x: 0 },
        unit,
        descriptor,
        OverlapPolicy::Reject,
    )
    .is_err());
    assert!(effective_region(
        SampleGridOrigin { y: 0, x: u32::MAX },
        unit,
        descriptor,
        OverlapPolicy::Reject,
    )
    .is_err());
    let full = Some(ValidRegion {
        y: 0,
        x: 0,
        height: 2,
        width: 2,
    });
    assert!(effective_region(
        SampleGridOrigin {
            y: u32::MAX - 1,
            x: 0,
        },
        full,
        descriptor,
        OverlapPolicy::Reject,
    )
    .is_err());
    assert!(effective_region(
        SampleGridOrigin {
            y: 0,
            x: u32::MAX - 1,
        },
        full,
        descriptor,
        OverlapPolicy::Reject,
    )
    .is_err());
}
