//! Bounded, pixel-free DICOM metadata loading shared by derived-object readers and viewers.
//!
//! The reader preflights file meta and data-set structure on the same file handle used
//! for parsing. Limits apply before materializing values; pixel data is never loaded.

use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use dicom_core::{Tag, VR};
use dicom_dictionary_std::tags;
use dicom_encoding::transfer_syntax::{TransferSyntax, TransferSyntaxIndex};
use dicom_object::{file::ReadPreamble, DefaultDicomObject, OpenFileOptions};
use dicom_parser::dataset::{lazy_read::LazyDataSetReader, LazyDataToken};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;

use crate::{Error, Result};

const MAX_FILE_META_BYTES: u32 = 1024 * 1024;
const MAX_FILE_META_ELEMENTS: usize = 128;
/// Largest encoded metadata value accepted before pixel data.
pub const MAX_METADATA_ELEMENT_BYTES: u32 = 16 * 1024 * 1024;
/// Maximum cumulative encoded metadata value bytes before pixel data.
pub const MAX_METADATA_VALUE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_METADATA_TOKENS: usize = 2_000_000;
/// Maximum sequence nesting accepted before pixel data.
pub const MAX_METADATA_SEQUENCE_DEPTH: usize = 64;
const MAX_TRANSFER_SYNTAX_UID_BYTES: u32 = 128;

/// Read a DICOM Part 10 object up to (but excluding) pixel data.
///
/// Requires a file preamble and a supported transfer syntax. File meta is limited
/// to 1 MiB and 128 elements, individual metadata values to 16 MiB, cumulative
/// values to 128 MiB, tokens to two million, and sequence nesting to 64 levels.
/// These are admission limits for encoded metadata, not a total heap bound.
///
/// # Errors
/// Returns [`Error::Io`] for filesystem failures, [`Error::InvalidInput`] for
/// rejected structure or limits, and [`Error::DicomRead`] for parser failures.
/// Errors identify the source path. Callers must not modify the file during reading.
pub fn open_metadata_object(path: &Path) -> Result<DefaultDicomObject> {
    let mut file = std::fs::File::open(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let transfer_syntax_uid = preflight_file_meta(&mut file, path)?;
    let transfer_syntax = TransferSyntaxRegistry
        .get(&transfer_syntax_uid)
        .ok_or_else(|| {
            invalid_metadata(
                path,
                format!("unsupported transfer syntax {transfer_syntax_uid}"),
            )
        })?;
    preflight_data_set(&mut file, path, transfer_syntax)?;
    file.seek(SeekFrom::Start(0)).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    OpenFileOptions::new()
        .read_until(tags::FLOAT_PIXEL_DATA)
        .read_preamble(ReadPreamble::Always)
        .from_reader(file)
        .map_err(|source| Error::DicomRead {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
}

fn preflight_file_meta(file: &mut std::fs::File, path: &Path) -> Result<String> {
    file.seek(SeekFrom::Start(128))
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut magic = [0_u8; 4];
    read_metadata_exact(file, path, &mut magic, "DICOM magic code")?;
    if &magic != b"DICM" {
        return Err(invalid_metadata(path, "missing DICOM file preamble"));
    }

    let group_length_header = read_explicit_header(file, path)?;
    if group_length_header.tag != tags::FILE_META_INFORMATION_GROUP_LENGTH
        || group_length_header.vr != VR::UL
        || group_length_header.value_len != 4
    {
        return Err(invalid_metadata(
            path,
            "invalid File Meta Information Group Length element",
        ));
    }
    let mut length_bytes = [0_u8; 4];
    read_metadata_exact(file, path, &mut length_bytes, "file meta group length")?;
    let group_length = u32::from_le_bytes(length_bytes);
    if group_length > MAX_FILE_META_BYTES {
        return Err(invalid_metadata(
            path,
            format!(
                "file meta group is {group_length} bytes, exceeding the {MAX_FILE_META_BYTES}-byte limit"
            ),
        ));
    }

    let mut remaining = u64::from(group_length);
    let mut element_count = 0_usize;
    let mut transfer_syntax_uid = None;
    while remaining > 0 {
        element_count = element_count.saturating_add(1);
        if element_count > MAX_FILE_META_ELEMENTS {
            return Err(invalid_metadata(
                path,
                format!("file meta group exceeds the {MAX_FILE_META_ELEMENTS}-element limit"),
            ));
        }
        let header = read_explicit_header(file, path)?;
        let encoded_len = header
            .header_len
            .checked_add(u64::from(header.value_len))
            .ok_or_else(|| invalid_metadata(path, "file meta element length overflows"))?;
        if header.tag.group() != 0x0002 || encoded_len > remaining {
            return Err(invalid_metadata(
                path,
                format!(
                    "file meta element {} exceeds the declared group boundary",
                    header.tag
                ),
            ));
        }
        if header.tag == tags::TRANSFER_SYNTAX_UID {
            if header.value_len == 0 || header.value_len > MAX_TRANSFER_SYNTAX_UID_BYTES {
                return Err(invalid_metadata(
                    path,
                    "transfer syntax UID has an invalid declared length",
                ));
            }
            let mut value = vec![0_u8; header.value_len as usize];
            read_metadata_exact(file, path, &mut value, "transfer syntax UID")?;
            let value = String::from_utf8(value).map_err(|_| {
                invalid_metadata(path, "transfer syntax UID is not valid ASCII/UTF-8")
            })?;
            transfer_syntax_uid = Some(
                value
                    .trim_end_matches(|character: char| {
                        character.is_whitespace() || character == '\0'
                    })
                    .to_string(),
            );
        } else {
            file.seek(SeekFrom::Current(i64::from(header.value_len)))
                .map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
        remaining -= encoded_len;
    }

    let position = file.stream_position().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file_len = file
        .metadata()
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if position > file_len {
        return Err(invalid_metadata(
            path,
            "file meta group extends beyond the end of the file",
        ));
    }
    transfer_syntax_uid
        .filter(|uid| !uid.is_empty())
        .ok_or_else(|| invalid_metadata(path, "file meta group has no transfer syntax UID"))
}

struct ExplicitHeader {
    tag: Tag,
    vr: VR,
    value_len: u32,
    header_len: u64,
}

fn read_explicit_header(file: &mut std::fs::File, path: &Path) -> Result<ExplicitHeader> {
    let mut base = [0_u8; 8];
    read_metadata_exact(file, path, &mut base, "file meta element header")?;
    let tag = Tag(
        u16::from_le_bytes([base[0], base[1]]),
        u16::from_le_bytes([base[2], base[3]]),
    );
    let vr = VR::from_binary([base[4], base[5]])
        .ok_or_else(|| invalid_metadata(path, format!("invalid VR in file meta element {tag}")))?;
    let uses_u32_length = matches!(
        vr,
        VR::OB
            | VR::OD
            | VR::OF
            | VR::OL
            | VR::OV
            | VR::OW
            | VR::SQ
            | VR::UC
            | VR::UN
            | VR::UR
            | VR::UT
    );
    let (value_len, header_len) = if uses_u32_length {
        if base[6..8] != [0, 0] {
            return Err(invalid_metadata(
                path,
                format!("invalid reserved bytes in file meta element {tag}"),
            ));
        }
        let mut length = [0_u8; 4];
        read_metadata_exact(file, path, &mut length, "file meta element length")?;
        (u32::from_le_bytes(length), 12)
    } else {
        (u32::from(u16::from_le_bytes([base[6], base[7]])), 8)
    };
    Ok(ExplicitHeader {
        tag,
        vr,
        value_len,
        header_len,
    })
}

fn preflight_data_set(
    file: &mut (impl Read + Seek),
    path: &Path,
    transfer_syntax: &TransferSyntax,
) -> Result<()> {
    // The lazy parser does not buffer its source. Batch its small header/value
    // reads; this buffer is dropped before the owning caller rewinds the file.
    let mut reader = LazyDataSetReader::new_with_ts(BufReader::new(file), transfer_syntax)
        .map_err(|error| {
            invalid_metadata(
                path,
                format!("could not initialize metadata parser: {error}"),
            )
        })?;
    let mut token_count = 0_usize;
    let mut sequence_depth = 0_usize;
    let mut declared_value_bytes = 0_u64;

    while let Some(token) = reader.advance() {
        token_count = token_count.saturating_add(1);
        if token_count > MAX_METADATA_TOKENS {
            return Err(invalid_metadata(
                path,
                format!("metadata exceeds the {MAX_METADATA_TOKENS}-token limit"),
            ));
        }
        let token = token.map_err(|error| {
            invalid_metadata(path, format!("could not preflight metadata: {error}"))
        })?;
        match token {
            LazyDataToken::ElementHeader(header) => {
                if is_pixel_value(header.tag) {
                    return Ok(());
                }
                if header.len.0 > MAX_METADATA_ELEMENT_BYTES {
                    return Err(invalid_metadata(
                        path,
                        format!(
                            "metadata element value limit is {MAX_METADATA_ELEMENT_BYTES} bytes, but {} declares {} bytes",
                            header.tag, header.len.0
                        ),
                    ));
                }
                declared_value_bytes = declared_value_bytes
                    .checked_add(u64::from(header.len.0))
                    .ok_or_else(|| invalid_metadata(path, "metadata byte count overflows"))?;
                if declared_value_bytes > MAX_METADATA_VALUE_BYTES {
                    return Err(invalid_metadata(
                        path,
                        format!(
                            "metadata declares more than the {MAX_METADATA_VALUE_BYTES}-byte cumulative value limit"
                        ),
                    ));
                }
            }
            LazyDataToken::SequenceStart { tag, .. } => {
                if is_pixel_value(tag) {
                    return Ok(());
                }
                sequence_depth = sequence_depth.saturating_add(1);
                if sequence_depth > MAX_METADATA_SEQUENCE_DEPTH {
                    return Err(invalid_metadata(
                        path,
                        format!(
                            "metadata sequence nesting exceeds the {MAX_METADATA_SEQUENCE_DEPTH}-level limit"
                        ),
                    ));
                }
            }
            LazyDataToken::PixelSequenceStart => return Ok(()),
            LazyDataToken::SequenceEnd => {
                sequence_depth = sequence_depth.saturating_sub(1);
            }
            lazy @ (LazyDataToken::LazyValue { .. } | LazyDataToken::LazyItemValue { .. }) => {
                lazy.skip().map_err(|error| {
                    invalid_metadata(path, format!("could not skip metadata value: {error}"))
                })?;
            }
            LazyDataToken::ItemStart { .. } | LazyDataToken::ItemEnd => {}
            _ => {
                return Err(invalid_metadata(
                    path,
                    "metadata parser returned an unsupported token",
                ));
            }
        }
    }
    if sequence_depth != 0 {
        return Err(invalid_metadata(
            path,
            "metadata ended inside an unterminated sequence",
        ));
    }
    Ok(())
}

fn is_pixel_value(tag: Tag) -> bool {
    matches!(
        tag,
        tags::FLOAT_PIXEL_DATA | tags::DOUBLE_FLOAT_PIXEL_DATA | tags::PIXEL_DATA
    )
}

fn read_metadata_exact(
    file: &mut std::fs::File,
    path: &Path,
    bytes: &mut [u8],
    context: &str,
) -> Result<()> {
    file.read_exact(bytes)
        .map_err(|error| invalid_metadata(path, format!("could not read {context}: {error}")))
}

fn invalid_metadata(path: &Path, reason: impl std::fmt::Display) -> Error {
    Error::InvalidInput(format!(
        "DICOM metadata preflight failed for {}: {reason}",
        path.display()
    ))
}

#[cfg(test)]
mod tests;
