use std::io::Write;

use dicom_core::value::{PrimitiveValue, Value};
use dicom_dictionary_std::{tags, uids};

use crate::metadata::open_metadata_object;
use crate::test_support::write_source_wsi;
use crate::{
    DerivedObjectProducer, DicomAnnotationContext, ParametricMapDocument, RasterChannelSelection,
    RasterInputFormat, RasterProfile,
};

#[test]
fn raster_profile_requires_explicit_axes_channels_and_integer_scaling() {
    let profile = RasterProfile::from_json(PROFILE_F32.as_bytes()).unwrap();

    assert_eq!(profile.input_format(), RasterInputFormat::Npy);
    assert_eq!(profile.channel_count(), 2);
    assert_eq!(profile.channel_name(0), Some("tumor"));
    assert_eq!(
        profile
            .select_channels(RasterChannelSelection::Name("stroma".into()))
            .unwrap(),
        vec![1]
    );

    let integer_without_scaling = PROFILE_F32
        .replace("\"npy\"", "\"tiff\"")
        .replace("\"float32\"", "\"uint16\"");
    let error = RasterProfile::from_json(integer_without_scaling.as_bytes()).unwrap_err();
    assert!(error.to_string().contains("integer_scaling"));
}

#[test]
fn raster_profile_enforces_zarr_path_and_multichannel_selection() {
    let missing_path = PROFILE_F32.replace("\"npy\"", "\"zarr\"");
    let error = RasterProfile::from_json(missing_path.as_bytes()).unwrap_err();
    assert!(error.to_string().contains("zarr_array_path"));

    let profile = RasterProfile::from_json(PROFILE_F32.as_bytes()).unwrap();
    assert!(profile
        .select_channels(RasterChannelSelection::Auto)
        .unwrap_err()
        .to_string()
        .contains("multichannel"));
    assert_eq!(
        profile
            .select_channels(RasterChannelSelection::All)
            .unwrap(),
        vec![0, 1]
    );
}

#[test]
fn raster_profile_enforces_scheme_designator_conditions_for_urn_codes() {
    let mut profile: serde_json::Value =
        serde_json::from_str(&single_channel_profile("float32")).unwrap();
    profile["channels"][0]["quantity"] = serde_json::json!({
        "urn_code_value": "urn:example:pathology:probability",
        "code_meaning": "Tumor probability"
    });

    RasterProfile::from_json(serde_json::to_vec(&profile).unwrap().as_slice()).unwrap();

    profile["channels"][0]["quantity"]["coding_scheme_version"] = serde_json::json!("2026");
    let error =
        RasterProfile::from_json(serde_json::to_vec(&profile).unwrap().as_slice()).unwrap_err();
    assert!(error
        .to_string()
        .contains("coding scheme version requires a coding scheme designator"));

    profile["channels"][0]["quantity"] = serde_json::json!({
        "code_value": "PROB",
        "code_meaning": "Tumor probability"
    });
    let error =
        RasterProfile::from_json(serde_json::to_vec(&profile).unwrap().as_slice()).unwrap_err();
    assert!(error.to_string().contains("coding scheme designator"));
}

#[test]
fn raster_profile_rejects_incomplete_enhanced_code_qualifiers() {
    let mut profile: serde_json::Value =
        serde_json::from_str(&single_channel_profile("float32")).unwrap();
    profile["channels"][0]["quantity"]["context_identifier"] = serde_json::json!("1234");

    let parse = |profile: &serde_json::Value| {
        RasterProfile::from_json(serde_json::to_vec(profile).unwrap().as_slice())
    };
    let error = parse(&profile).unwrap_err();
    assert!(error.to_string().contains("mapping resource"));

    profile["channels"][0]["quantity"]["mapping_resource"] = serde_json::json!("DCMR");
    let error = parse(&profile).unwrap_err();
    assert!(error.to_string().contains("context group version"));

    profile["channels"][0]["quantity"]["context_group_version"] = serde_json::json!("20260814");
    parse(&profile).unwrap();

    profile["channels"][0]["quantity"]["context_group_extension"] = serde_json::json!(true);
    let error = parse(&profile).unwrap_err();
    assert!(error.to_string().contains("local version"));

    profile["channels"][0]["quantity"]["context_group_local_version"] =
        serde_json::json!("20260814");
    let error = parse(&profile).unwrap_err();
    assert!(error.to_string().contains("extension creator UID"));

    profile["channels"][0]["quantity"]["context_group_extension_creator_uid"] =
        serde_json::json!("2.25.1");
    parse(&profile).unwrap();

    profile["channels"][0]["quantity"]["context_group_extension_creator_uid"] =
        serde_json::json!("not-a-uid");
    let error = parse(&profile).unwrap_err();
    assert!(error.to_string().contains("valid DICOM UID"));
}

#[test]
fn float32_parametric_map_streams_required_metadata_and_exact_pixels() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let input_path = directory.path().join("probability.npy");
    let output_path = directory.path().join("map.dcm");
    write_source_wsi(&source_path, 4, 3, 2, 2);
    let values = [
        0.0_f32,
        0.1,
        f32::from_bits(0x7fa1_2345),
        0.3,
        0.4,
        0.5,
        0.6,
        0.7,
        0.8,
        0.9,
        1.0,
        0.25,
    ];
    write_npy_f32(&input_path, &[3, 4], &values, false);
    let profile = RasterProfile::from_json(single_channel_profile("float32").as_bytes()).unwrap();
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let document = ParametricMapDocument::open(
        source.clone(),
        source,
        profile,
        &input_path,
        RasterChannelSelection::Auto,
    )
    .unwrap()
    .with_producer(
        DerivedObjectProducer::new(44, "Acme Pathology", "Acme PM", "PM-42", "7.3.1").unwrap(),
    );

    let written = document.write_single(&output_path, 2_000_000_000).unwrap();

    assert_eq!(written.frame_count(), 1);
    assert_eq!(written.pixel_value_length(), 3 * 4 * 4);
    let object = open_metadata_object(&output_path).unwrap();
    assert_eq!(
        object.meta().media_storage_sop_class_uid(),
        uids::PARAMETRIC_MAP_STORAGE
    );
    assert_eq!(
        object
            .element(tags::MANUFACTURER_MODEL_NAME)
            .unwrap()
            .to_str()
            .unwrap(),
        "Acme PM"
    );
    assert_eq!(
        object
            .element(tags::SERIES_NUMBER)
            .unwrap()
            .to_str()
            .unwrap(),
        "44"
    );
    assert_eq!(
        object
            .element(tags::CONTENT_QUALIFICATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "SERVICE"
    );
    assert_eq!(
        object
            .element(tags::DIMENSION_ORGANIZATION_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "TILED_FULL"
    );
    assert_eq!(
        object
            .element(tags::NUMBER_OF_FRAMES)
            .unwrap()
            .to_int::<u32>()
            .unwrap(),
        1
    );
    assert!(object.get(tags::DIMENSION_ORGANIZATION_SEQUENCE).is_some());
    assert!(object.element(tags::LATERALITY).is_err());
    assert!(object.get(tags::VOLUMETRIC_PROPERTIES).is_none());
    assert!(object
        .get(tags::VOLUME_BASED_CALCULATION_TECHNIQUE)
        .is_none());
    assert!(object
        .get(tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE)
        .is_some());
    assert!(object
        .get(tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE)
        .is_some());
    assert_eq!(
        object
            .element(tags::FLOAT_PIXEL_PADDING_VALUE)
            .unwrap()
            .to_float32()
            .unwrap()
            .to_bits(),
        0x7fc0_0000
    );
    let payload = last_explicit_long_value(&output_path, tags::FLOAT_PIXEL_DATA, b"OF");
    let actual = payload
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()).to_bits())
        .collect::<Vec<_>>();
    let expected = values
        .iter()
        .map(|value| {
            if value.is_nan() {
                0x7fc0_0000
            } else {
                value.to_bits()
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    let shared = object
        .element(tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    let derivation = shared[0]
        .element(tags::DERIVATION_IMAGE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    assert!(derivation[0].get(tags::DERIVATION_CODE_SEQUENCE).is_some());
}

#[test]
fn parametric_map_preview_is_bounded_normalized_and_registered_to_the_source() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let input_path = directory.path().join("probability.npy");
    write_source_wsi(&source_path, 4, 3, 2, 2);
    write_npy_f32(
        &input_path,
        &[3, 4],
        &[
            0.0,
            0.1,
            f32::NAN,
            0.3,
            0.4,
            0.5,
            0.6,
            0.7,
            0.8,
            0.9,
            1.0,
            0.25,
        ],
        false,
    );
    let profile = RasterProfile::from_json(single_channel_profile("float32").as_bytes()).unwrap();
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let document = ParametricMapDocument::open(
        source.clone(),
        source,
        profile,
        &input_path,
        RasterChannelSelection::Auto,
    )
    .unwrap();

    let preview = document.preview(0, 4).unwrap();

    assert_eq!(preview.dimensions(), (2, 2));
    assert_eq!(preview.channel_name(), "tumor");
    assert_eq!(preview.quantity().value(), "TUMOR");
    assert_eq!(preview.unit().value(), "1");
    assert_eq!(preview.value_range(), (0.0, 1.0));
    for (actual, expected) in
        preview
            .normalized_values()
            .iter()
            .zip([0.25_f32, 0.533_333_36, 0.85, 0.625])
    {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
    assert!((preview.base_corners()[0].x + 0.5).abs() < 1e-9);
    assert!((preview.base_corners()[0].y + 0.5).abs() < 1e-9);
    assert!((preview.base_corners()[2].x - 3.5).abs() < 1e-9);
    assert!((preview.base_corners()[2].y - 2.5).abs() < 1e-9);
}

#[test]
fn float64_parametric_map_uses_double_float_pixel_data() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let input_path = directory.path().join("probability.npy");
    let output_path = directory.path().join("map.dcm");
    write_source_wsi(&source_path, 2, 2, 2, 2);
    let values = [0.125_f64, 0.25, f64::NAN, 0.75];
    write_npy_f64(&input_path, &[2, 2], &values, false);
    let profile = RasterProfile::from_json(single_channel_profile("float64").as_bytes()).unwrap();
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let document = ParametricMapDocument::open(
        source.clone(),
        source,
        profile,
        &input_path,
        RasterChannelSelection::Auto,
    )
    .unwrap();

    let written = document.write_single(&output_path, 2_000_000_000).unwrap();

    assert_eq!(written.pixel_value_length(), 2 * 2 * 8);
    let object = open_metadata_object(&output_path).unwrap();
    assert_eq!(
        object
            .element(tags::DOUBLE_FLOAT_PIXEL_PADDING_VALUE)
            .unwrap()
            .to_float64()
            .unwrap()
            .to_bits(),
        0x7ff8_0000_0000_0000
    );
    let payload = last_explicit_long_value(&output_path, tags::DOUBLE_FLOAT_PIXEL_DATA, b"OD");
    assert_eq!(payload.len(), values.len() * 8);
    assert_eq!(
        f64::from_le_bytes(payload[16..24].try_into().unwrap()).to_bits(),
        0x7ff8_0000_0000_0000
    );
}

#[test]
fn integer_parametric_map_applies_declared_scaling_and_output_precision() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let input_path = directory.path().join("integer.npy");
    let output_path = directory.path().join("map.dcm");
    write_source_wsi(&source_path, 2, 2, 2, 2);
    write_npy(
        &input_path,
        &[2, 2],
        "<i2",
        false,
        [1_i16, -1, 3, 4].into_iter().flat_map(i16::to_le_bytes),
    );
    let mut profile: serde_json::Value =
        serde_json::from_str(&single_channel_profile("int16")).unwrap();
    profile["integer_scaling"] = serde_json::json!({
        "slope": 0.5,
        "intercept": 1.0,
        "missing_sentinel": -1,
        "output_precision": "float64"
    });
    let profile = RasterProfile::from_json(&serde_json::to_vec(&profile).unwrap()).unwrap();
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let document = ParametricMapDocument::open(
        source.clone(),
        source,
        profile,
        &input_path,
        RasterChannelSelection::Auto,
    )
    .unwrap();

    document.write_single(&output_path, 2_000_000_000).unwrap();

    assert!(document
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code() == "RASTER_INTEGER_SCALED"));
    let payload = last_explicit_long_value(&output_path, tags::DOUBLE_FLOAT_PIXEL_DATA, b"OD");
    let values = payload
        .chunks_exact(8)
        .map(|bytes| f64::from_le_bytes(bytes.try_into().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(values[0], 1.5);
    assert_eq!(values[1].to_bits(), 0x7ff8_0000_0000_0000);
    assert_eq!(values[2..], [2.5, 3.0]);
}

#[test]
fn all_missing_tiles_are_omitted_as_tiled_sparse_frames() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let input_path = directory.path().join("sparse.npy");
    let output_path = directory.path().join("map.dcm");
    write_source_wsi(&source_path, 257, 1, 256, 1);
    let mut values = vec![f32::NAN; 257];
    values[256] = 0.75;
    write_npy_f32(&input_path, &[1, 257], &values, false);
    let profile = RasterProfile::from_json(single_channel_profile("float32").as_bytes()).unwrap();
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let document = ParametricMapDocument::open(
        source.clone(),
        source,
        profile,
        &input_path,
        RasterChannelSelection::Auto,
    )
    .unwrap();

    assert_eq!(document.frame_count(), 1);
    document.write_single(&output_path, 2_000_000_000).unwrap();

    let object = open_metadata_object(&output_path).unwrap();
    assert_eq!(
        object
            .element(tags::DIMENSION_ORGANIZATION_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "TILED_SPARSE"
    );
    let per_frame = object
        .element(tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    let position = per_frame[0]
        .element(tags::PLANE_POSITION_SLIDE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    assert_eq!(
        position[0]
            .element(tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX)
            .unwrap()
            .to_int::<u32>()
            .unwrap(),
        257
    );
    let payload = last_explicit_long_value(&output_path, tags::FLOAT_PIXEL_DATA, b"OF");
    assert_eq!(f32::from_le_bytes(payload[..4].try_into().unwrap()), 0.75);
    assert!(payload[4..].chunks_exact(4).all(|bytes| f32::from_le_bytes(
        bytes.try_into().unwrap()
    )
    .to_bits()
        == 0x7fc0_0000));
}

#[test]
fn all_channels_use_one_sparse_pm_with_quantity_dimension() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let input_path = directory.path().join("channels.npy");
    let output_path = directory.path().join("map.dcm");
    write_source_wsi(&source_path, 2, 1, 2, 1);
    write_npy_f32(&input_path, &[1, 2, 2], &[0.1, 0.9, 0.25, 0.75], false);
    let profile = RasterProfile::from_json(PROFILE_F32.as_bytes()).unwrap();
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let document = ParametricMapDocument::open(
        source.clone(),
        source,
        profile,
        &input_path,
        RasterChannelSelection::All,
    )
    .unwrap();

    assert_eq!(document.frame_count(), 2);
    document.write_single(&output_path, 2_000_000_000).unwrap();

    let object = open_metadata_object(&output_path).unwrap();
    assert_eq!(
        object
            .element(tags::DIMENSION_ORGANIZATION_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "TILED_SPARSE"
    );
    let dimensions = object
        .element(tags::DIMENSION_INDEX_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    assert_eq!(dimensions.len(), 3);
    let pointer = dimensions[0]
        .element(tags::DIMENSION_INDEX_POINTER)
        .unwrap();
    assert!(matches!(
        pointer.value(),
        Value::Primitive(PrimitiveValue::Tags(values))
            if values.as_slice() == [tags::QUANTITY_DEFINITION_SEQUENCE]
    ));
    let per_frame = object
        .element(tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE)
        .unwrap()
        .items()
        .unwrap();
    assert_eq!(per_frame.len(), 2);
    for (index, frame) in per_frame.iter().enumerate() {
        let content = frame
            .element(tags::FRAME_CONTENT_SEQUENCE)
            .unwrap()
            .items()
            .unwrap();
        let values = content[0]
            .element(tags::DIMENSION_INDEX_VALUES)
            .unwrap()
            .to_multi_int::<u32>()
            .unwrap();
        assert_eq!(values, vec![u32::try_from(index + 1).unwrap(), 1, 1]);
        assert!(frame.get(tags::REAL_WORLD_VALUE_MAPPING_SEQUENCE).is_some());
    }
}

#[test]
fn exact_size_planning_forces_and_verifies_concatenation_parts() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let input_path = directory.path().join("wide.npy");
    write_source_wsi(&source_path, 513, 1, 256, 1);
    let values = (0..513)
        .map(|value| value as f32 / 512.0)
        .collect::<Vec<_>>();
    write_npy_f32(&input_path, &[1, 513], &values, false);
    let profile = RasterProfile::from_json(single_channel_profile("float32").as_bytes()).unwrap();
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let document = ParametricMapDocument::open(
        source.clone(),
        source,
        profile,
        &input_path,
        RasterChannelSelection::Auto,
    )
    .unwrap();
    let single = document.plan(2_000_000_000).unwrap();
    assert_eq!(single.parts().len(), 1);
    assert!(!single.series_instance_uid().is_empty());
    assert!(single.concatenation_uid().is_none());
    assert!(single.concatenation_source_sop_instance_uid().is_none());
    assert_eq!(single.parts()[0].frame_offset(), 0);
    assert_eq!(single.parts()[0].frame_count(), document.frame_count());
    assert!(single.parts()[0].pixel_value_length() > 0);
    assert_eq!(single.parts()[0].pixel_sha256().len(), 64);
    assert!(!single.parts()[0].sop_instance_uid().is_empty());
    let limit = single.parts()[0].encoded_file_length() - 1_024;

    let plan = document.plan(limit).unwrap();

    assert!(plan.parts().len() > 1);
    assert!(plan.concatenation_uid().is_some());
    assert!(plan.concatenation_source_sop_instance_uid().is_some());
    assert!(plan
        .parts()
        .iter()
        .all(|part| part.encoded_file_length() <= limit));
    let paths = (0..plan.parts().len())
        .map(|index| directory.path().join(format!("pm-{index:04}.dcm")))
        .collect::<Vec<_>>();
    let instances = document.write_planned_parts(&plan, &paths).unwrap();
    assert_eq!(instances.len(), plan.parts().len());
    let mut expected_offset = 0_u32;
    for (index, (path, instance)) in paths.iter().zip(&instances).enumerate() {
        let part = &plan.parts()[index];
        let object = open_metadata_object(path).unwrap();
        assert_eq!(
            object
                .element(tags::IN_CONCATENATION_NUMBER)
                .unwrap()
                .to_int::<u16>()
                .unwrap(),
            u16::try_from(index + 1).unwrap()
        );
        assert_eq!(
            object
                .element(tags::IN_CONCATENATION_TOTAL_NUMBER)
                .unwrap()
                .to_int::<u16>()
                .unwrap(),
            u16::try_from(paths.len()).unwrap()
        );
        assert_eq!(instance.frame_offset(), expected_offset);
        assert_eq!(instance.sop_instance_uid(), part.sop_instance_uid());
        assert_eq!(instance.series_instance_uid(), plan.series_instance_uid());
        assert_eq!(instance.pixel_value_length(), part.pixel_value_length());
        assert_eq!(instance.encoded_file_length(), part.encoded_file_length());
        assert_eq!(instance.pixel_sha256(), part.pixel_sha256());
        expected_offset += instance.frame_count();
    }
    assert_eq!(expected_offset, document.frame_count());
}

#[test]
fn parametric_map_semantic_digest_includes_full_algorithm_and_code_identity() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let input_path = directory.path().join("probability.npy");
    write_source_wsi(&source_path, 2, 2, 2, 2);
    write_npy_f32(&input_path, &[2, 2], &[0.1, 0.2, 0.3, 0.4], false);
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let base: serde_json::Value = serde_json::from_str(&single_channel_profile("float32")).unwrap();
    let digest = |profile: &serde_json::Value| {
        let profile = RasterProfile::from_json(&serde_json::to_vec(profile).unwrap()).unwrap();
        ParametricMapDocument::open(
            source.clone(),
            source.clone(),
            profile,
            &input_path,
            RasterChannelSelection::Auto,
        )
        .unwrap()
        .semantic_digest()
        .to_owned()
    };
    let base_digest = digest(&base);

    let mut algorithm_parameters = base.clone();
    algorithm_parameters["algorithm"]["parameters"] = serde_json::json!("threshold=0.4");
    assert_ne!(base_digest, digest(&algorithm_parameters));

    let mut code_qualifier = base;
    code_qualifier["channels"][0]["quantity"]["coding_scheme_version"] = serde_json::json!("2026");
    assert_ne!(base_digest, digest(&code_qualifier));
}

fn single_channel_profile(dtype: &str) -> String {
    PROFILE_F32
        .replace("\"dtype\": \"float32\"", &format!("\"dtype\": \"{dtype}\""))
        .replace(
            r#",
    {
      "name": "stroma",
      "quantity": {"code_value":"STROMA","coding_scheme_designator":"99WSI","code_meaning":"Stroma probability"},
      "unit": {"code_value":"1","coding_scheme_designator":"UCUM","code_meaning":"no units"}
    }"#,
            "",
        )
        .replace(", \"channel\"", "")
}

fn write_npy_f32(path: &std::path::Path, shape: &[usize], values: &[f32], fortran: bool) {
    write_npy(
        path,
        shape,
        "<f4",
        fortran,
        values.iter().flat_map(|value| value.to_le_bytes()),
    );
}

fn write_npy_f64(path: &std::path::Path, shape: &[usize], values: &[f64], fortran: bool) {
    write_npy(
        path,
        shape,
        "<f8",
        fortran,
        values.iter().flat_map(|value| value.to_le_bytes()),
    );
}

fn write_npy(
    path: &std::path::Path,
    shape: &[usize],
    dtype: &str,
    fortran: bool,
    bytes: impl Iterator<Item = u8>,
) {
    let shape = shape
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let mut header = format!(
        "{{'descr': '{dtype}', 'fortran_order': {}, 'shape': ({shape},), }}",
        if fortran { "True" } else { "False" }
    );
    let prefix_length = 10;
    while (prefix_length + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(b"\x93NUMPY\x01\x00").unwrap();
    file.write_all(&(header.len() as u16).to_le_bytes())
        .unwrap();
    file.write_all(header.as_bytes()).unwrap();
    file.write_all(&bytes.collect::<Vec<_>>()).unwrap();
}

fn last_explicit_long_value(path: &std::path::Path, tag: dicom_core::Tag, vr: &[u8; 2]) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    let offset = (0..=bytes.len() - 12)
        .rev()
        .find(|offset| {
            bytes[*offset..*offset + 2] == tag.group().to_le_bytes()
                && bytes[*offset + 2..*offset + 4] == tag.element().to_le_bytes()
                && bytes[*offset + 4..*offset + 6] == *vr
                && u32::from_le_bytes(bytes[*offset + 8..*offset + 12].try_into().unwrap()) as usize
                    == bytes.len() - *offset - 12
        })
        .expect("streamed pixel element header");
    assert_eq!(&bytes[offset..offset + 2], &tag.group().to_le_bytes());
    assert_eq!(&bytes[offset + 2..offset + 4], &tag.element().to_le_bytes());
    assert_eq!(&bytes[offset + 4..offset + 6], vr);
    let length = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
    bytes[offset + 12..offset + 12 + length].to_vec()
}

const PROFILE_F32: &str = r#"
{
  "schema_version": 1,
  "input_format": "npy",
  "dtype": "float32",
  "axes": ["y", "x", "channel"],
  "grid_origin": {"x": 0.0, "y": 0.0},
  "sample_spacing": {"x": 1.0, "y": 1.0},
  "coordinate_space": "level0-pixels",
  "channels": [
    {
      "name": "tumor",
      "quantity": {"code_value":"TUMOR","coding_scheme_designator":"99WSI","code_meaning":"Tumor probability"},
      "unit": {"code_value":"1","coding_scheme_designator":"UCUM","code_meaning":"no units"}
    },
    {
      "name": "stroma",
      "quantity": {"code_value":"STROMA","coding_scheme_designator":"99WSI","code_meaning":"Stroma probability"},
      "unit": {"code_value":"1","coding_scheme_designator":"UCUM","code_meaning":"no units"}
    }
  ],
  "algorithm": {
    "family": {"code_value":"123110","coding_scheme_designator":"DCM","code_meaning":"Artificial Intelligence"},
    "name": "Example model",
    "version": "1.0"
  }
}
"#;
