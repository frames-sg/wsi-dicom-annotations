use super::*;

#[test]
fn dtype_axes_subsets_and_reordering_cover_supported_scalar_contracts() {
    assert!(is_canonical_axis_order(&[RasterAxis::Y, RasterAxis::X]));
    assert!(is_canonical_axis_order(&[
        RasterAxis::Y,
        RasterAxis::X,
        RasterAxis::Channel,
    ]));
    assert!(!is_canonical_axis_order(&[
        RasterAxis::Channel,
        RasterAxis::Y,
        RasterAxis::X,
    ]));

    for (dtype, expected) in [
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
    ] {
        assert_eq!(dtype_from_zarr(&dtype).unwrap(), expected);
    }
    assert!(dtype_from_zarr(&data_type::bool()).is_err());

    let axes = [RasterAxis::Channel, RasterAxis::Y, RasterAxis::X];
    let (_, shape) = subset_for_region(&axes, 2, 3, 4, 5, 6).unwrap();
    assert_eq!(shape, vec![1, 5, 6]);
    assert_eq!(
        reorder_region(vec![0, 1, 2, 3, 4, 5], &[1, 2, 3], &axes, 2, 3).unwrap(),
        vec![0, 1, 2, 3, 4, 5]
    );
    assert!(reorder_region(vec![1], &[1], &[RasterAxis::X], 1, 1).is_err());
    assert!(reorder_region(vec![1], &[1], &[RasterAxis::Y], 1, 1).is_err());
    assert!(reorder_region(
        Vec::<u8>::new(),
        &[1, 1],
        &[RasterAxis::Y, RasterAxis::X],
        1,
        1
    )
    .is_err());
}

#[test]
fn strides_and_local_tree_validation_reject_overflow_missing_components_and_special_files() {
    assert_eq!(c_strides(&[2, 3, 4]).unwrap(), vec![12, 4, 1]);
    assert!(c_strides(&[1, u64::MAX, 2]).is_err());
    assert!(matches!(
        unsafe_zarr_entry(Path::new("entry"), "special file"),
        Error::InvalidInput(_)
    ));
    assert!(matches!(zarr_error("bad chunk"), Error::InvalidInput(_)));

    let directory = tempfile::tempdir().unwrap();
    assert!(validate_local_array_tree(directory.path(), "missing").is_err());
    std::fs::write(directory.path().join("file"), b"not a directory").unwrap();
    assert!(validate_local_array_tree(directory.path(), "file").is_err());

    let deep_root = directory.path().join("deep");
    std::fs::create_dir(&deep_root).unwrap();
    let mut nested = deep_root;
    for _ in 0..=MAX_LOCAL_ARRAY_DEPTH {
        nested.push("d");
        std::fs::create_dir(&nested).unwrap();
    }
    assert!(validate_local_array_tree(directory.path(), "deep").is_err());

    #[cfg(unix)]
    {
        let socket_root = directory.path().join("socket-array");
        std::fs::create_dir(&socket_root).unwrap();
        let _socket =
            std::os::unix::net::UnixListener::bind(socket_root.join("chunk.sock")).unwrap();
        assert!(validate_local_array_tree(directory.path(), "socket-array").is_err());
    }
}
