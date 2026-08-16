use std::path::Path;

use super::{npy::write_test_array, NormalizedRaster, PixelTile, RasterProfile};

#[test]
fn applies_all_explicit_overlap_policies_without_full_grid_allocation() {
    let directory = tempfile::tempdir().unwrap();
    write_test_array(
        &directory.path().join("left.npy"),
        &[1.0_f32, 2.0, 3.0, 4.0],
    );
    write_test_array(
        &directory.path().join("right.npy"),
        &[10.0_f32, 20.0, 30.0, 40.0],
    );
    let manifest_path = directory.path().join("tiles.json");
    let profile = profile();
    for (policy, expected) in [
        ("mean", vec![1.0, 6.0, 20.0, 3.0, 17.0, 40.0]),
        ("max", vec![1.0, 10.0, 20.0, 3.0, 30.0, 40.0]),
        (
            "manifest-order-last-write",
            vec![1.0, 10.0, 20.0, 3.0, 30.0, 40.0],
        ),
    ] {
        std::fs::write(&manifest_path, manifest_json(policy)).unwrap();
        let source = NormalizedRaster::open(&profile, &manifest_path).unwrap();
        assert!(source.normalizations().iter().any(|diagnostic| diagnostic
            .code()
            .contains(&policy.replace('-', "_").to_uppercase())));
        assert_eq!(
            source.read_tile(0, 0, 0, 2, 3).unwrap(),
            PixelTile::F32(expected),
            "policy {policy}"
        );
        assert_eq!(
            (source.descriptor().height, source.descriptor().width),
            (2, 3)
        );
    }

    std::fs::write(&manifest_path, manifest_json("reject")).unwrap();
    let source = NormalizedRaster::open(&profile, &manifest_path).unwrap();
    assert!(source
        .read_tile(0, 0, 0, 2, 3)
        .unwrap_err()
        .to_string()
        .contains("overlap"));
}

#[test]
fn valid_region_crop_uses_only_declared_tile_regions() {
    let directory = tempfile::tempdir().unwrap();
    write_test_array(
        &directory.path().join("left.npy"),
        &[1.0_f32, 2.0, 3.0, 4.0],
    );
    write_test_array(
        &directory.path().join("right.npy"),
        &[10.0_f32, 20.0, 30.0, 40.0],
    );
    let manifest_path = directory.path().join("tiles.json");
    std::fs::write(
        &manifest_path,
        r#"{"schema_version":1,"overlap_policy":"valid-region-crop","tiles":[
          {"path":"left.npy","format":"npy","sample_grid_origin":{"y":0,"x":0},"valid_region":{"y":0,"x":0,"height":2,"width":1}},
          {"path":"right.npy","format":"npy","sample_grid_origin":{"y":0,"x":1},"valid_region":{"y":0,"x":0,"height":2,"width":2}}
        ]}"#,
    )
    .unwrap();

    let source = NormalizedRaster::open(&profile(), &manifest_path).unwrap();

    assert!(source
        .normalizations()
        .iter()
        .any(|diagnostic| diagnostic.code() == "TILED_MANIFEST_VALID_REGION_CROP"));
    assert_eq!(
        source.read_tile(0, 0, 0, 2, 3).unwrap(),
        PixelTile::F32(vec![1.0, 10.0, 20.0, 3.0, 30.0, 40.0])
    );
}

#[test]
fn rejects_escaping_paths_mixed_dtypes_and_truncated_tiles() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write_test_array(
        &outside.path().join("outside.npy"),
        &[1.0_f32, 2.0, 3.0, 4.0],
    );
    let manifest_path = directory.path().join("tiles.json");
    std::fs::write(
        &manifest_path,
        r#"{"schema_version":1,"tiles":[{"path":"../outside.npy","format":"npy","sample_grid_origin":{"y":0,"x":0}}]}"#,
    )
    .unwrap();
    let error = open_error(&manifest_path);
    assert!(error.contains("relative") || error.contains("escape"));

    write_test_array(&directory.path().join("integer.npy"), &[1_i16, 2, 3, 4]);
    std::fs::write(
        &manifest_path,
        r#"{"schema_version":1,"tiles":[{"path":"integer.npy","format":"npy","sample_grid_origin":{"y":0,"x":0}}]}"#,
    )
    .unwrap();
    assert!(open_error(&manifest_path).contains("dtype"));

    std::fs::write(directory.path().join("truncated.npy"), b"\x93NUMPY").unwrap();
    std::fs::write(
        &manifest_path,
        r#"{"schema_version":1,"tiles":[{"path":"truncated.npy","format":"npy","sample_grid_origin":{"y":0,"x":0}}]}"#,
    )
    .unwrap();
    assert!(open_error(&manifest_path).contains("parse NPY"));
}

fn open_error(path: &Path) -> String {
    match NormalizedRaster::open(&profile(), path) {
        Ok(_) => panic!("invalid tiled manifest should be rejected"),
        Err(error) => error.to_string(),
    }
}

fn manifest_json(policy: &str) -> String {
    format!(
        r#"{{"schema_version":1,"overlap_policy":"{policy}","tiles":[
          {{"path":"left.npy","format":"npy","sample_grid_origin":{{"y":0,"x":0}}}},
          {{"path":"right.npy","format":"npy","sample_grid_origin":{{"y":0,"x":1}}}}
        ]}}"#,
    )
}

fn profile() -> RasterProfile {
    RasterProfile::from_json(PROFILE.as_bytes()).unwrap()
}

const PROFILE: &str = r#"{
  "schema_version":1,
  "input_format":"tiled-manifest",
  "dtype":"float32",
  "axes":["y","x"],
  "grid_origin":{"x":0.0,"y":0.0},
  "sample_spacing":{"x":1.0,"y":1.0},
  "coordinate_space":"level0-pixels",
  "channels":[{"name":"probability","quantity":{"code_value":"A","coding_scheme_designator":"99T","code_meaning":"A"},"unit":{"code_value":"1","coding_scheme_designator":"UCUM","code_meaning":"none"}}],
  "algorithm":{"family":{"code_value":"123110","coding_scheme_designator":"DCM","code_meaning":"Artificial Intelligence"},"name":"test","version":"1"}
}"#;
