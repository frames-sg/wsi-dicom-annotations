use sha2::{Digest, Sha256};

use super::model::{AlgorithmIdentification, AnnotationGroup, DicomCode};

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

pub(crate) fn deterministic_annotation_group_uid(
    group: &AnnotationGroup,
    namespace: &str,
    scope: &str,
    application_key: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"wsi-dicom-annotations:annotation-group-uid:v1\0");
    for value in [namespace, scope, application_key] {
        update_text(&mut digest, value);
    }
    update_text(&mut digest, group.geometry().graphic_type().dicom_value());
    update_text(&mut digest, group.label());
    update_text(&mut digest, group.description());
    update_code(&mut digest, group.category());
    update_code(&mut digest, group.property_type());
    update_codes(&mut digest, group.property_type_modifiers());
    update_codes(&mut digest, group.anatomic_regions());
    update_codes(&mut digest, group.primary_anatomic_structures());
    update_text(&mut digest, group.generation_type().dicom_value());
    digest.update((group.algorithms().len() as u64).to_le_bytes());
    for algorithm in group.algorithms() {
        update_algorithm(&mut digest, algorithm);
    }
    for component in group.recommended_display_cielab() {
        digest.update(component.to_le_bytes());
    }
    digest.update([u8::from(group.applies_to_all_optical_paths())]);
    digest.update((group.referenced_optical_paths().len() as u64).to_le_bytes());
    for path in group.referenced_optical_paths() {
        update_text(&mut digest, path);
    }
    digest.update([u8::from(group.applies_to_all_z_planes())]);
    digest.update((group.common_z_coordinates().len() as u64).to_le_bytes());
    for coordinate in group.common_z_coordinates() {
        digest.update(coordinate.to_bits().to_le_bytes());
    }
    let bytes = digest.finalize();
    let value = u128::from_be_bytes(bytes[..16].try_into().expect("SHA-256 prefix is 16 bytes"));
    format!("2.25.{value}")
}

fn update_codes(digest: &mut Sha256, codes: &[DicomCode]) {
    digest.update((codes.len() as u64).to_le_bytes());
    for code in codes {
        update_code(digest, code);
    }
}
