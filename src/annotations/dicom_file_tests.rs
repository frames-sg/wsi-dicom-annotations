use super::*;
use dicom_dictionary_std::tags;

const SOP_INSTANCE_UID: &str = "1.2.826.0.1.3680043.10.777.9401";

fn empty_object() -> InMemDicomObject {
    InMemDicomObject::new_empty()
}

#[test]
fn streamed_value_contract_accepts_only_even_float_elements() {
    let of = StreamedDicomValue::new(tags::FLOAT_PIXEL_DATA, VR::OF, 8).unwrap();
    let od = StreamedDicomValue::new(tags::DOUBLE_FLOAT_PIXEL_DATA, VR::OD, 16).unwrap();
    assert_eq!(of.vr_bytes().unwrap(), b"OF");
    assert_eq!(od.vr_bytes().unwrap(), b"OD");
    assert!(StreamedDicomValue::new(tags::PIXEL_DATA, VR::OB, 8).is_err());
    assert!(StreamedDicomValue::new(tags::FLOAT_PIXEL_DATA, VR::OF, 3).is_err());
}

#[test]
fn encoded_prefix_and_counting_writer_report_exact_bytes() {
    let length = encoded_dicom_prefix_length(
        empty_object(),
        uids::PARAMETRIC_MAP_STORAGE,
        SOP_INSTANCE_UID,
    )
    .unwrap();
    assert!(length >= 132);

    let mut counter = CountingWriter::default();
    counter.write_all(&[1, 2, 3]).unwrap();
    counter.write_all(&[4, 5]).unwrap();
    counter.flush().unwrap();
    assert_eq!(counter.bytes, 5);
}

#[test]
fn atomic_dicom_writes_replace_or_publish_only_after_verification() {
    let directory = tempfile::tempdir().unwrap();
    let replacement = directory.path().join("replacement.dcm");
    fs::write(&replacement, b"old").unwrap();
    atomic_write_dicom(
        &replacement,
        empty_object(),
        uids::PARAMETRIC_MAP_STORAGE,
        SOP_INSTANCE_UID,
    )
    .unwrap();
    let object = dicom_object::open_file(&replacement).unwrap();
    assert_eq!(
        object.meta().media_storage_sop_class_uid(),
        uids::PARAMETRIC_MAP_STORAGE
    );

    let streamed_path = directory.path().join("streamed.dcm");
    let streamed = StreamedDicomValue::new(tags::FLOAT_PIXEL_DATA, VR::OF, 8).unwrap();
    let values = [1.0_f32, 2.0_f32]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    atomic_write_new_dicom_with_streamed_value(
        &streamed_path,
        empty_object(),
        uids::PARAMETRIC_MAP_STORAGE,
        SOP_INSTANCE_UID,
        streamed,
        |file| {
            file.write_all(&values).map_err(|source| Error::Io {
                path: streamed_path.clone(),
                source,
            })
        },
        |temporary| {
            let encoded = fs::read(temporary).unwrap();
            assert!(encoded.ends_with(&values));
            Ok(())
        },
    )
    .unwrap();
    assert!(streamed_path.is_file());
    assert!(atomic_write_new_dicom_with_streamed_value(
        &streamed_path,
        empty_object(),
        uids::PARAMETRIC_MAP_STORAGE,
        SOP_INSTANCE_UID,
        streamed,
        |_| Ok(()),
        |_| Ok(()),
    )
    .is_err());

    let rejected = directory.path().join("rejected.dcm");
    let result = atomic_write_new_dicom_with_streamed_value(
        &rejected,
        empty_object(),
        uids::PARAMETRIC_MAP_STORAGE,
        SOP_INSTANCE_UID,
        streamed,
        |_| Err(Error::InvalidInput("intentional write failure".into())),
        |_| Ok(()),
    );
    assert!(result.is_err());
    assert!(!rejected.exists());
}

#[test]
fn file_limits_and_sidecar_alias_checks_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.dcm");
    fs::write(&source, b"1234").unwrap();

    enforce_file_limit(&source, 4, "test").unwrap();
    assert!(enforce_file_limit(&source, 3, "test").is_err());
    assert!(enforce_file_limit(&directory.path().join("missing"), 4, "test").is_err());
    assert!(ensure_sidecar_destination(&source, &source).is_err());
    ensure_sidecar_destination(&directory.path().join("sidecar.dcm"), &source).unwrap();

    #[cfg(unix)]
    {
        let alias = directory.path().join("source-alias.dcm");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        assert!(ensure_sidecar_destination(&alias, &source).is_err());
    }
}
