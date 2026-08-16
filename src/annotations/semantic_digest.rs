use sha2::{Digest, Sha256};

use super::model::{AlgorithmIdentification, DicomCode};

pub(crate) fn update_text(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value.as_bytes());
}

pub(crate) fn update_code(digest: &mut Sha256, code: &DicomCode) {
    digest.update([code.value_kind() as u8]);
    for value in [
        code.value(),
        code.scheme(),
        code.coding_scheme_version().unwrap_or(""),
        code.meaning(),
        code.context_identifier().unwrap_or(""),
        code.context_uid().unwrap_or(""),
        code.mapping_resource().unwrap_or(""),
        code.mapping_resource_uid().unwrap_or(""),
        code.context_group_version().unwrap_or(""),
        code.context_group_local_version().unwrap_or(""),
        code.context_group_extension_creator_uid().unwrap_or(""),
    ] {
        update_text(digest, value);
    }
    digest.update([code.context_group_extension().map_or(2, u8::from)]);
}

pub(crate) fn update_algorithm(digest: &mut Sha256, algorithm: &AlgorithmIdentification) {
    update_code(digest, algorithm.family());
    digest.update([u8::from(algorithm.name_code().is_some())]);
    if let Some(name_code) = algorithm.name_code() {
        update_code(digest, name_code);
    }
    for value in [
        algorithm.name(),
        algorithm.version(),
        algorithm.parameters().unwrap_or(""),
        algorithm.source().unwrap_or(""),
    ] {
        update_text(digest, value);
    }
}

pub(crate) fn finish(digest: Sha256) -> String {
    format!("{:x}", digest.finalize())
}
