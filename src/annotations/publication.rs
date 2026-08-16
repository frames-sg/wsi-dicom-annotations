use std::fs;
use std::path::{Component, Path, PathBuf};

use tempfile::TempDir;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct DicomPublicationError {
    code: &'static str,
    message: String,
}

impl DicomPublicationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn io(code: &'static str, path: &Path, error: std::io::Error) -> Self {
        Self::new(code, format!("{}: {error}", path.display()))
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

pub struct DicomSinglePublication {
    destination: PathBuf,
    parent: PathBuf,
    staging: TempDir,
    staged_file: PathBuf,
}

impl DicomSinglePublication {
    pub fn new(
        destination: &Path,
        staged_name: &str,
        protected_inputs: &[&Path],
    ) -> std::result::Result<Self, DicomPublicationError> {
        let (destination, parent) = validate_destination(destination, false, protected_inputs)?;
        let staging = tempfile::Builder::new()
            .prefix(".dicom-output-")
            .tempdir_in(&parent)
            .map_err(|error| {
                DicomPublicationError::io("OUTPUT_STAGING_FAILED", &destination, error)
            })?;
        let staged_file = staging.path().join(staged_name);
        Ok(Self {
            destination,
            parent,
            staging,
            staged_file,
        })
    }

    #[must_use]
    pub fn staged_file(&self) -> &Path {
        &self.staged_file
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn publish(self) -> std::result::Result<PathBuf, DicomPublicationError> {
        sync_path(self.staging.path())?;
        fs::hard_link(&self.staged_file, &self.destination).map_err(|error| {
            DicomPublicationError::io("OUTPUT_PUBLICATION_FAILED", &self.destination, error)
        })?;
        if let Err(error) = sync_path(&self.parent) {
            let cleanup = fs::remove_file(&self.destination);
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup_error) => DicomPublicationError::new(
                    "OUTPUT_ROLLBACK_FAILED",
                    format!(
                        "{error}; additionally could not remove {}: {cleanup_error}",
                        self.destination.display()
                    ),
                ),
            });
        }
        Ok(self.destination)
    }
}

pub struct DicomBundlePublication {
    destination: PathBuf,
    parent: PathBuf,
    staging: TempDir,
}

impl DicomBundlePublication {
    pub fn new(
        destination: &Path,
        protected_inputs: &[&Path],
    ) -> std::result::Result<Self, DicomPublicationError> {
        let (destination, parent) = validate_destination(destination, true, protected_inputs)?;
        let staging = tempfile::Builder::new()
            .prefix(".dicom-output-")
            .tempdir_in(&parent)
            .map_err(|error| {
                DicomPublicationError::io("OUTPUT_STAGING_FAILED", &destination, error)
            })?;
        Ok(Self {
            destination,
            parent,
            staging,
        })
    }

    #[must_use]
    pub fn staging_path(&self) -> &Path {
        self.staging.path()
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn sync_staged_file(&self, path: &Path) -> std::result::Result<(), DicomPublicationError> {
        if !path.starts_with(self.staging.path()) || !path.is_file() {
            return Err(DicomPublicationError::new(
                "OUTPUT_SYNC_FAILED",
                format!(
                    "staged output {} is not a regular file in {}",
                    path.display(),
                    self.staging.path().display()
                ),
            ));
        }
        sync_path(path)
    }

    pub fn publish(self) -> std::result::Result<PathBuf, DicomPublicationError> {
        sync_path(self.staging.path())?;
        ensure_absent(&self.destination)?;
        fs::rename(self.staging.path(), &self.destination).map_err(|error| {
            DicomPublicationError::io("OUTPUT_PUBLICATION_FAILED", &self.destination, error)
        })?;
        if let Err(error) = sync_path(&self.parent) {
            let rollback = fs::rename(&self.destination, self.staging.path());
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => DicomPublicationError::new(
                    "OUTPUT_ROLLBACK_FAILED",
                    format!(
                        "{error}; additionally could not roll back {}: {rollback_error}",
                        self.destination.display()
                    ),
                ),
            });
        }
        Ok(self.destination)
    }
}

fn sync_path(path: &Path) -> std::result::Result<(), DicomPublicationError> {
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| DicomPublicationError::io("OUTPUT_SYNC_FAILED", path, error))
}

fn validate_destination(
    destination: &Path,
    directory: bool,
    protected_inputs: &[&Path],
) -> std::result::Result<(PathBuf, PathBuf), DicomPublicationError> {
    ensure_absent(destination)?;
    if destination.file_name().is_none()
        || destination
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(DicomPublicationError::new(
            "UNSAFE_OUTPUT_PATH",
            format!(
                "output path {} is not a concrete child path",
                destination.display()
            ),
        ));
    }
    let requested_parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = requested_parent.canonicalize().map_err(|error| {
        DicomPublicationError::io("OUTPUT_PARENT_INVALID", requested_parent, error)
    })?;
    if !parent.is_dir() {
        return Err(DicomPublicationError::new(
            "OUTPUT_PARENT_INVALID",
            format!("output parent {} is not a directory", parent.display()),
        ));
    }
    let file_name = destination.file_name().ok_or_else(|| {
        DicomPublicationError::new(
            "UNSAFE_OUTPUT_PATH",
            format!("output path {} has no file name", destination.display()),
        )
    })?;
    let resolved = parent.join(file_name);
    for input in protected_inputs {
        let canonical = input
            .canonicalize()
            .map_err(|error| DicomPublicationError::io("INPUT_PATH_INVALID", input, error))?;
        if resolved == canonical
            || (directory && canonical.starts_with(&resolved))
            || (canonical.is_dir() && resolved.starts_with(&canonical))
        {
            return Err(DicomPublicationError::new(
                "UNSAFE_OUTPUT_PATH",
                format!(
                    "output {} aliases or is contained by protected input {}",
                    resolved.display(),
                    canonical.display()
                ),
            ));
        }
    }
    Ok((resolved, parent))
}

fn ensure_absent(path: &Path) -> std::result::Result<(), DicomPublicationError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(DicomPublicationError::new(
            "OUTPUT_EXISTS",
            format!("output path already exists: {}", path.display()),
        )),
        Err(error) => Err(DicomPublicationError::io(
            "OUTPUT_PATH_INVALID",
            path,
            error,
        )),
    }
}

#[cfg(test)]
#[path = "publication_tests.rs"]
mod tests;
