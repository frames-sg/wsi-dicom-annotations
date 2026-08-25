mod builtins;
mod color;
mod concept_key;
mod json;
mod model;

const SCHEME_SCHEMA_VERSION: u32 = 1;
const MAX_SCHEME_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLASSES: usize = 512;
const MAX_FINDING_SITES: usize = 128;
const DCMR_UID: &str = "1.2.840.10008.8.1.1";

pub use color::{dicom_cielab_to_srgb, srgb_to_dicom_cielab};
pub use concept_key::{annotation_class_concept_key, AnnotationClassConceptKey};
pub use model::{AnnotationClass, AnnotationClassGeometry, AnnotationScheme};
