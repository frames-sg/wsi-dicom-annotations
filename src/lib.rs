#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

use std::error::Error as StdError;
use std::path::PathBuf;

mod annotations;
pub mod metadata;

#[cfg(test)]
mod annotation_tests;
#[cfg(test)]
mod linear_measurement_tests;
#[cfg(all(test, feature = "parametric-map"))]
mod parametric_map_tests;
#[cfg(test)]
mod pathology_geojson_tests;
#[cfg(test)]
mod scheme_tests;
#[cfg(test)]
mod test_support;

pub use annotations::*;

/// Error returned by annotation import, conversion, and DICOM persistence.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Input is structurally invalid or violates a checked invariant.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// Input is valid but uses a representation this crate does not support.
    #[error("unsupported input: {0}")]
    Unsupported(String),
    /// Filesystem operation failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
    /// DICOM decoding failed.
    #[error("DICOM read error at {path}: {source}")]
    DicomRead {
        /// Path of the DICOM object being read.
        path: PathBuf,
        /// Underlying DICOM parser error.
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
    /// DICOM encoding failed.
    #[error("DICOM write error at {path}: {source}")]
    DicomWrite {
        /// Destination path of the DICOM object being written.
        path: PathBuf,
        /// Underlying DICOM writer error.
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
}

/// Result type used by annotation operations.
pub type Result<T> = std::result::Result<T, Error>;
