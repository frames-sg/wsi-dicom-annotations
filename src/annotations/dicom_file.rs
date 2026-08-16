use std::fs::{self, File, OpenOptions};
#[cfg(feature = "parametric-map")]
use std::io::{self, Write};
use std::path::Path;

#[cfg(feature = "parametric-map")]
use dicom_core::{Tag, VR};
use dicom_dictionary_std::uids;
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

use crate::{Error, Result};

pub(crate) fn atomic_write_dicom(
    path: &Path,
    object: InMemDicomObject,
    sop_class_uid: &str,
    sop_instance_uid: &str,
) -> Result<()> {
    atomic_write(
        path,
        object,
        sop_class_uid,
        sop_instance_uid,
        Persistence::Replace,
        |_| Ok(()),
        |_| Ok(()),
    )
}

#[derive(Debug, Clone, Copy)]
#[cfg(feature = "parametric-map")]
pub(crate) struct StreamedDicomValue {
    tag: Tag,
    vr: VR,
    value_length: u32,
}

#[cfg(feature = "parametric-map")]
impl StreamedDicomValue {
    pub(crate) fn new(tag: Tag, vr: VR, value_length: u32) -> Result<Self> {
        if !matches!(vr, VR::OF | VR::OD) || !value_length.is_multiple_of(2) {
            return Err(Error::InvalidInput(
                "streamed DICOM value must be an even-length OF or OD element".into(),
            ));
        }
        Ok(Self {
            tag,
            vr,
            value_length,
        })
    }

    fn vr_bytes(self) -> Result<&'static [u8; 2]> {
        match self.vr {
            VR::OF => Ok(b"OF"),
            VR::OD => Ok(b"OD"),
            _ => Err(Error::InvalidInput(
                "streamed DICOM value must use OF or OD VR".into(),
            )),
        }
    }
}

#[cfg(feature = "parametric-map")]
pub(crate) fn atomic_write_new_dicom_with_streamed_value(
    path: &Path,
    object: InMemDicomObject,
    sop_class_uid: &str,
    sop_instance_uid: &str,
    streamed: StreamedDicomValue,
    write_value: impl FnOnce(&mut File) -> Result<()>,
    verify: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(Error::InvalidInput(format!(
                "output path already exists: {}",
                path.display()
            )))
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    }
    atomic_write(
        path,
        object,
        sop_class_uid,
        sop_instance_uid,
        Persistence::NoClobber,
        move |file| {
            let vr_bytes = streamed.vr_bytes()?;
            file.write_all(&streamed.tag.group().to_le_bytes())
                .and_then(|()| file.write_all(&streamed.tag.element().to_le_bytes()))
                .and_then(|()| file.write_all(vr_bytes))
                .and_then(|()| file.write_all(&[0, 0]))
                .and_then(|()| file.write_all(&streamed.value_length.to_le_bytes()))
                .map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
            write_value(file)
        },
        verify,
    )
}

fn atomic_write(
    path: &Path,
    object: InMemDicomObject,
    sop_class_uid: &str,
    sop_instance_uid: &str,
    persistence: Persistence,
    append: impl FnOnce(&mut File) -> Result<()>,
    verify: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = tempfile::NamedTempFile::new_in(parent).map_err(|source| Error::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(sop_class_uid)
                .media_storage_sop_instance_uid(sop_instance_uid)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .map_err(|error| Error::DicomWrite {
            path: path.to_path_buf(),
            source: Box::new(error),
        })?
        .write_to_file(temporary.path())
        .map_err(|error| Error::DicomWrite {
            path: path.to_path_buf(),
            source: Box::new(error),
        })?;
    let mut file = OpenOptions::new()
        .append(true)
        .open(temporary.path())
        .map_err(|source| Error::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    append(&mut file)?;
    file.sync_all().map_err(|source| Error::Io {
        path: temporary.path().to_path_buf(),
        source,
    })?;
    drop(file);
    verify(temporary.path())?;
    match persistence {
        Persistence::Replace => temporary.persist(path),
        #[cfg(feature = "parametric-map")]
        Persistence::NoClobber => temporary.persist_noclobber(path),
    }
    .map_err(|error| Error::Io {
        path: path.to_path_buf(),
        source: error.error,
    })?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Persistence {
    Replace,
    #[cfg(feature = "parametric-map")]
    NoClobber,
}

#[cfg(feature = "parametric-map")]
pub(crate) fn encoded_dicom_prefix_length(
    object: InMemDicomObject,
    sop_class_uid: &str,
    sop_instance_uid: &str,
) -> Result<u64> {
    let mut counter = CountingWriter::default();
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(sop_class_uid)
                .media_storage_sop_instance_uid(sop_instance_uid)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .map_err(|error| Error::DicomWrite {
            path: "<PM size planning>".into(),
            source: Box::new(error),
        })?
        .write_all(&mut counter)
        .map_err(|error| Error::DicomWrite {
            path: "<PM size planning>".into(),
            source: Box::new(error),
        })?;
    Ok(counter.bytes)
}

#[derive(Default)]
#[cfg(feature = "parametric-map")]
struct CountingWriter {
    bytes: u64,
}

#[cfg(feature = "parametric-map")]
impl Write for CountingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let length = u64::try_from(buffer.len())
            .map_err(|_| io::Error::other("encoded buffer length does not fit u64"))?;
        self.bytes = self
            .bytes
            .checked_add(length)
            .ok_or_else(|| io::Error::other("encoded DICOM length overflows u64"))?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn enforce_file_limit(path: &Path, limit: u64, kind: &str) -> Result<()> {
    let length = fs::metadata(path)
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if length > limit {
        return Err(Error::InvalidInput(format!(
            "{kind} file is {length} bytes, exceeding the {limit}-byte import limit"
        )));
    }
    Ok(())
}

pub(crate) fn ensure_sidecar_destination(path: &Path, source_path: &Path) -> Result<()> {
    let aliases_source = path == source_path
        || path
            .canonicalize()
            .ok()
            .zip(source_path.canonicalize().ok())
            .is_some_and(|(path, source)| path == source);
    if aliases_source {
        return Err(Error::InvalidInput(
            "a DICOM annotation sidecar cannot replace the source WSI".into(),
        ));
    }
    Ok(())
}

#[cfg(all(test, feature = "parametric-map"))]
#[path = "dicom_file_tests.rs"]
mod tests;
