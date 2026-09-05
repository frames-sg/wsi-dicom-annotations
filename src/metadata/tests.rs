use super::*;

fn part10_file_meta(transfer_syntax: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 128];
    bytes.extend_from_slice(b"DICM");
    bytes.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, b'U', b'L', 0x04, 0x00]);
    let group_length = u32::try_from(8 + transfer_syntax.len()).unwrap();
    bytes.extend_from_slice(&group_length.to_le_bytes());
    bytes.extend_from_slice(&[0x02, 0x00, 0x10, 0x00, b'U', b'I']);
    bytes.extend_from_slice(&u16::try_from(transfer_syntax.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(transfer_syntax);
    bytes
}

#[test]
fn explicit_headers_accept_both_length_forms_and_reject_malformed_headers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("header.bin");

    std::fs::write(&path, [0x02, 0x00, 0x00, 0x00, b'U', b'L', 0x04, 0x00]).unwrap();
    let mut file = std::fs::File::open(&path).unwrap();
    let header = read_explicit_header(&mut file, &path).unwrap();
    assert_eq!(header.tag, tags::FILE_META_INFORMATION_GROUP_LENGTH);
    assert_eq!(header.vr, VR::UL);
    assert_eq!((header.value_len, header.header_len), (4, 8));

    std::fs::write(
        &path,
        [
            0x02, 0x00, 0x01, 0x00, b'O', b'B', 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
        ],
    )
    .unwrap();
    let mut file = std::fs::File::open(&path).unwrap();
    let header = read_explicit_header(&mut file, &path).unwrap();
    assert_eq!(
        (header.vr, header.value_len, header.header_len),
        (VR::OB, 4, 12)
    );

    for malformed in [
        vec![0x02, 0x00, 0x01, 0x00, b'Z', b'Z', 0x00, 0x00],
        vec![0x02, 0x00, 0x01, 0x00, b'O', b'B', 0x01, 0x00],
        vec![0x02, 0x00],
    ] {
        std::fs::write(&path, malformed).unwrap();
        let mut file = std::fs::File::open(&path).unwrap();
        assert!(read_explicit_header(&mut file, &path).is_err());
    }
}

#[test]
fn file_meta_preflight_rejects_missing_invalid_and_unsupported_transfer_syntaxes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("meta.dcm");

    let mut missing = vec![0; 128];
    missing.extend_from_slice(b"DICM");
    missing.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, b'U', b'L', 0x04, 0x00]);
    missing.extend_from_slice(&0_u32.to_le_bytes());
    std::fs::write(&path, missing).unwrap();
    let mut file = std::fs::File::open(&path).unwrap();
    assert!(preflight_file_meta(&mut file, &path).is_err());

    std::fs::write(&path, part10_file_meta(&[0xff, 0x00])).unwrap();
    let mut file = std::fs::File::open(&path).unwrap();
    assert!(preflight_file_meta(&mut file, &path).is_err());

    std::fs::write(&path, part10_file_meta(b"9.9\0")).unwrap();
    assert!(open_metadata_object(&path).is_err());

    let mut truncated_value = part10_file_meta(b"1.2.840.10008.1.2.1\0");
    truncated_value.extend_from_slice(&[0x10, 0x00, 0x10, 0x00, b'P', b'N', 0x04, 0x00]);
    std::fs::write(&path, truncated_value).unwrap();
    assert!(open_metadata_object(&path).is_err());

    let mut invalid_data_set_vr = part10_file_meta(b"1.2.840.10008.1.2.1\0");
    invalid_data_set_vr.extend_from_slice(&[0x10, 0x00, 0x10, 0x00, b'Z', b'Z', 0x00, 0x00]);
    std::fs::write(&path, invalid_data_set_vr).unwrap();
    assert!(open_metadata_object(&path).is_err());

    let mut oversized = vec![0; 128];
    oversized.extend_from_slice(b"DICM");
    oversized.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, b'U', b'L', 0x04, 0x00]);
    oversized.extend_from_slice(&(MAX_FILE_META_BYTES + 1).to_le_bytes());
    std::fs::write(&path, oversized).unwrap();
    let mut file = std::fs::File::open(&path).unwrap();
    assert!(preflight_file_meta(&mut file, &path).is_err());
}

#[test]
fn pixel_boundaries_and_diagnostic_context_are_stable() {
    assert!(is_pixel_value(tags::PIXEL_DATA));
    assert!(is_pixel_value(tags::FLOAT_PIXEL_DATA));
    assert!(is_pixel_value(tags::DOUBLE_FLOAT_PIXEL_DATA));
    assert!(!is_pixel_value(tags::ROWS));
    let source = Path::new("source.dcm");
    assert!(invalid_metadata(source, "reason")
        .to_string()
        .contains("reason"));
}

#[test]
fn metadata_admission_batches_small_reads_without_consuming_pixel_payload() {
    struct CountedReader {
        data: std::io::Cursor<Vec<u8>>,
        reads: usize,
    }
    impl Read for CountedReader {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            self.data.read(bytes)
        }
    }
    impl Seek for CountedReader {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.data.seek(position)
        }
    }

    let mut data = Vec::new();
    for element in 0..2_048_u16 {
        data.extend_from_slice(&0x7777_u16.to_le_bytes());
        data.extend_from_slice(&element.to_le_bytes());
        data.extend_from_slice(b"US\x02\0\x01\0");
    }
    let metadata_bytes = data.len();
    data.extend_from_slice(b"\xe0\x7f\x10\0OB\0\0\0\0\x10\0");
    data.resize(data.len() + 1024 * 1024, 0xff);
    let mut source = CountedReader {
        data: std::io::Cursor::new(data),
        reads: 0,
    };
    let syntax = TransferSyntaxRegistry.get("1.2.840.10008.1.2.1").unwrap();
    preflight_data_set(&mut source, Path::new("metadata.dcm"), syntax).unwrap();
    // Permit bounded read-ahead, but never read the large pixel payload.
    assert!(source.data.position() <= (metadata_bytes + 8192) as u64);
    assert!(
        source.reads <= 4,
        "metadata admission issued {} underlying reads",
        source.reads
    );
}
