use dicom_core::{Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use crate::{Error, Result};

use super::dicom_dataset::{
    optional_string, put_optional_text, put_text, required_string, sequence, sequence_items,
};
use super::model::{
    AlgorithmIdentification, DiagnosticDisposition, DiagnosticSeverity, DicomCode,
    DicomCodeQualifiers, DicomCodeValueKind, InteroperabilityDiagnostic,
};

pub(crate) fn code_item(code: &DicomCode) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    let (value_tag, value_vr) = match code.value_kind() {
        DicomCodeValueKind::Short => (tags::CODE_VALUE, VR::SH),
        DicomCodeValueKind::Long => (tags::LONG_CODE_VALUE, VR::UC),
        DicomCodeValueKind::Urn => (tags::URN_CODE_VALUE, VR::UR),
    };
    put_text(&mut item, value_tag, value_vr, code.value());
    if !code.scheme().is_empty() {
        put_text(
            &mut item,
            tags::CODING_SCHEME_DESIGNATOR,
            VR::SH,
            code.scheme(),
        );
    }
    put_optional_text(
        &mut item,
        tags::CODING_SCHEME_VERSION,
        VR::SH,
        code.coding_scheme_version(),
    );
    put_text(&mut item, tags::CODE_MEANING, VR::LO, code.meaning());
    put_optional_text(
        &mut item,
        tags::CONTEXT_IDENTIFIER,
        VR::CS,
        code.context_identifier(),
    );
    put_optional_text(&mut item, tags::CONTEXT_UID, VR::UI, code.context_uid());
    put_optional_text(
        &mut item,
        tags::MAPPING_RESOURCE,
        VR::CS,
        code.mapping_resource(),
    );
    put_optional_text(
        &mut item,
        tags::MAPPING_RESOURCE_UID,
        VR::UI,
        code.mapping_resource_uid(),
    );
    put_optional_text(
        &mut item,
        tags::CONTEXT_GROUP_VERSION,
        VR::DT,
        code.context_group_version(),
    );
    put_optional_text(
        &mut item,
        tags::CONTEXT_GROUP_LOCAL_VERSION,
        VR::DT,
        code.context_group_local_version(),
    );
    if let Some(extension) = code.context_group_extension() {
        put_text(
            &mut item,
            tags::CONTEXT_GROUP_EXTENSION_FLAG,
            VR::CS,
            if extension { "Y" } else { "N" },
        );
    }
    put_optional_text(
        &mut item,
        tags::CONTEXT_GROUP_EXTENSION_CREATOR_UID,
        VR::UI,
        code.context_group_extension_creator_uid(),
    );
    item
}

pub(crate) fn read_code_at(
    object: &InMemDicomObject,
    tag: Tag,
    path: &str,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<DicomCode> {
    let items = sequence_items(object, tag)?;
    if items.len() != 1 {
        return Err(Error::InvalidInput(format!(
            "code sequence {tag} must contain exactly one item"
        )));
    }
    read_code_item(&items[0], path, diagnostics)
}

pub(crate) fn read_codes_at(
    object: &InMemDicomObject,
    tag: Tag,
    path: &str,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<Vec<DicomCode>> {
    let Some(element) = object.get(tag) else {
        return Ok(Vec::new());
    };
    let items = element
        .items()
        .ok_or_else(|| Error::InvalidInput(format!("{path} is present but is not a sequence")))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| read_code_item(item, &format!("{path}[{index}]"), diagnostics))
        .collect()
}

fn read_code_item(
    item: &InMemDicomObject,
    path: &str,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<DicomCode> {
    let candidates = [
        (tags::CODE_VALUE, DicomCodeValueKind::Short, false),
        (tags::LONG_CODE_VALUE, DicomCodeValueKind::Long, false),
        (tags::URN_CODE_VALUE, DicomCodeValueKind::Urn, false),
        (tags::EXTENDED_CODE_VALUE, DicomCodeValueKind::Long, true),
    ]
    .into_iter()
    .filter_map(|(tag, kind, normalized)| {
        optional_string(item, tag).map(|value| (value, kind, normalized))
    })
    .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err(Error::InvalidInput(format!(
            "{path} must contain exactly one code value representation"
        )));
    }
    let (value, kind, normalized) = &candidates[0];
    if *normalized {
        diagnostics.push(InteroperabilityDiagnostic::new(
            "EXTENDED_CODE_VALUE_NORMALIZED",
            DiagnosticSeverity::Info,
            path,
            DiagnosticDisposition::Normalized,
            "retired Extended Code Value will be written as Long Code Value",
        ));
    }
    for (tag, keyword, code) in [
        (
            tags::EQUIVALENT_CODE_SEQUENCE,
            "EquivalentCodeSequence",
            "EQUIVALENT_CODE_SEQUENCE_WOULD_DROP",
        ),
        (
            tags::CONTEXT_GROUP_IDENTIFICATION_SEQUENCE,
            "ContextGroupIdentificationSequence",
            "CONTEXT_GROUP_IDENTIFICATION_WOULD_DROP",
        ),
        (
            tags::MAPPING_RESOURCE_IDENTIFICATION_SEQUENCE,
            "MappingResourceIdentificationSequence",
            "MAPPING_RESOURCE_IDENTIFICATION_WOULD_DROP",
        ),
        (
            tags::EXTENDED_CODE_MEANING,
            "ExtendedCodeMeaning",
            "EXTENDED_CODE_MEANING_WOULD_DROP",
        ),
        (
            tags::ANATOMIC_REGION_MODIFIER_SEQUENCE,
            "AnatomicRegionModifierSequence",
            "ANATOMIC_REGION_MODIFIER_WOULD_DROP",
        ),
        (
            tags::PRIMARY_ANATOMIC_STRUCTURE_MODIFIER_SEQUENCE,
            "PrimaryAnatomicStructureModifierSequence",
            "PRIMARY_ANATOMIC_STRUCTURE_MODIFIER_WOULD_DROP",
        ),
    ] {
        if item.get(tag).is_some() {
            diagnostics.push(InteroperabilityDiagnostic::new(
                code,
                DiagnosticSeverity::Warning,
                format!("{path}.{keyword}"),
                DiagnosticDisposition::WouldDrop,
                format!("{keyword} is outside the preserved semantic allowlist"),
            ));
        }
    }
    let scheme = optional_string(item, tags::CODING_SCHEME_DESIGNATOR).unwrap_or_default();
    let code = match kind {
        DicomCodeValueKind::Short => DicomCode::new(
            value.clone(),
            scheme,
            required_string(item, tags::CODE_MEANING)?,
        )?,
        DicomCodeValueKind::Long => DicomCode::new_long(
            value.clone(),
            scheme,
            required_string(item, tags::CODE_MEANING)?,
        )?,
        DicomCodeValueKind::Urn => DicomCode::new_urn(
            value.clone(),
            scheme,
            required_string(item, tags::CODE_MEANING)?,
        )?,
    };
    let extension = optional_string(item, tags::CONTEXT_GROUP_EXTENSION_FLAG)
        .map(|value| match value.as_str() {
            "Y" => Ok(true),
            "N" => Ok(false),
            _ => Err(Error::InvalidInput(format!(
                "{path}.ContextGroupExtensionFlag is not Y or N"
            ))),
        })
        .transpose()?;
    code.with_optional_qualifiers(DicomCodeQualifiers {
        coding_scheme_version: optional_string(item, tags::CODING_SCHEME_VERSION),
        context_identifier: optional_string(item, tags::CONTEXT_IDENTIFIER),
        context_uid: optional_string(item, tags::CONTEXT_UID),
        mapping_resource: optional_string(item, tags::MAPPING_RESOURCE),
        mapping_resource_uid: optional_string(item, tags::MAPPING_RESOURCE_UID),
        context_group_version: optional_string(item, tags::CONTEXT_GROUP_VERSION),
        context_group_local_version: optional_string(item, tags::CONTEXT_GROUP_LOCAL_VERSION),
        context_group_extension: extension,
        context_group_extension_creator_uid: optional_string(
            item,
            tags::CONTEXT_GROUP_EXTENSION_CREATOR_UID,
        ),
    })
}

pub(crate) fn write_algorithm(algorithm: &AlgorithmIdentification) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    item.put(sequence(
        tags::ALGORITHM_FAMILY_CODE_SEQUENCE,
        vec![code_item(algorithm.family())],
    ));
    if let Some(name_code) = algorithm.name_code() {
        item.put(sequence(
            tags::ALGORITHM_NAME_CODE_SEQUENCE,
            vec![code_item(name_code)],
        ));
    }
    put_text(&mut item, tags::ALGORITHM_NAME, VR::LO, algorithm.name());
    put_text(
        &mut item,
        tags::ALGORITHM_VERSION,
        VR::LO,
        algorithm.version(),
    );
    put_optional_text(
        &mut item,
        tags::ALGORITHM_PARAMETERS,
        VR::LT,
        algorithm.parameters(),
    );
    put_optional_text(
        &mut item,
        tags::ALGORITHM_SOURCE,
        VR::LO,
        algorithm.source(),
    );
    item
}

pub(crate) fn read_algorithms(
    object: &InMemDicomObject,
    tag: Tag,
    path: &str,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<Vec<AlgorithmIdentification>> {
    let Some(element) = object.get(tag) else {
        return Ok(Vec::new());
    };
    let items = element
        .items()
        .ok_or_else(|| Error::InvalidInput(format!("{path} is present but is not a sequence")))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let path = format!("{path}[{index}]");
            let family = read_code_at(
                item,
                tags::ALGORITHM_FAMILY_CODE_SEQUENCE,
                &format!("{path}.AlgorithmFamilyCodeSequence"),
                diagnostics,
            )?;
            let mut algorithm = AlgorithmIdentification::new(
                family,
                required_string(item, tags::ALGORITHM_NAME)?,
                required_string(item, tags::ALGORITHM_VERSION)?,
            )?;
            if item.get(tags::ALGORITHM_NAME_CODE_SEQUENCE).is_some() {
                algorithm = algorithm.with_name_code(read_code_at(
                    item,
                    tags::ALGORITHM_NAME_CODE_SEQUENCE,
                    &format!("{path}.AlgorithmNameCodeSequence"),
                    diagnostics,
                )?);
            }
            if let Some(parameters) = optional_string(item, tags::ALGORITHM_PARAMETERS) {
                algorithm = algorithm.with_parameters(parameters)?;
            }
            if let Some(source) = optional_string(item, tags::ALGORITHM_SOURCE) {
                algorithm = algorithm.with_source(source)?;
            }
            Ok(algorithm)
        })
        .collect()
}
