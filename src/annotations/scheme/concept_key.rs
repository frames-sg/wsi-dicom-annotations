use sha2::{Digest, Sha256};

use super::model::AnnotationClassGeometry;
use crate::annotations::model::{DicomCode, DicomCodeValueKind};
use crate::annotations::semantic_digest::update_text;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AnnotationClassConceptKey(pub(super) String);

impl AnnotationClassConceptKey {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AnnotationClassConceptKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Builds the stable semantic identity used to match imported concepts to a
/// controlled annotation class.
///
/// Display meanings and other presentation metadata are deliberately ignored.
#[must_use]
pub fn annotation_class_concept_key(
    geometry: AnnotationClassGeometry,
    category: &DicomCode,
    property_type: &DicomCode,
    modifiers: &[DicomCode],
) -> AnnotationClassConceptKey {
    let mut digest = Sha256::new();
    digest.update(b"frames-annotation-class-concept-v1\0");
    digest.update([match geometry {
        AnnotationClassGeometry::Region => 0,
        AnnotationClassGeometry::Point => 1,
    }]);
    update_concept_code(&mut digest, category);
    update_concept_code(&mut digest, property_type);
    let mut modifiers = modifiers.iter().map(code_identity).collect::<Vec<_>>();
    modifiers.sort();
    digest.update((modifiers.len() as u64).to_le_bytes());
    for modifier in modifiers {
        update_text(&mut digest, &modifier);
    }
    AnnotationClassConceptKey(format!("sha256:{:x}", digest.finalize()))
}

fn update_concept_code(digest: &mut Sha256, code: &DicomCode) {
    update_text(digest, &code_identity(code));
}

pub(super) fn code_identity(code: &DicomCode) -> String {
    let kind = match code.value_kind() {
        DicomCodeValueKind::Short => "short",
        DicomCodeValueKind::Long => "long",
        DicomCodeValueKind::Urn => "urn",
    };
    format!(
        "{kind}\u{1f}{}\u{1f}{}\u{1f}{}",
        code.scheme(),
        code.value(),
        code.coding_scheme_version().unwrap_or("")
    )
}
