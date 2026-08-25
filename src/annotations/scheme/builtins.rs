use super::model::{AnnotationClass, AnnotationClassGeometry, AnnotationScheme};
use super::DCMR_UID;
use crate::annotations::model::DicomCode;
use crate::Result;

pub(super) fn builtin_general_pathology() -> Result<AnnotationScheme> {
    let category = |value, meaning| {
        contextual_code(
            value,
            "SCT",
            meaning,
            "7150",
            "20260319",
            "1.2.840.10008.6.1.496",
        )
    };
    let tissue_type = |value, meaning| {
        contextual_code(
            value,
            "SCT",
            meaning,
            "7166",
            "20240611",
            "1.2.840.10008.6.1.963",
        )
    };
    let lesion_type = |value, meaning| {
        contextual_code(
            value,
            "SCT",
            meaning,
            "7159",
            "20240611",
            "1.2.840.10008.6.1.505",
        )
    };
    let microscopy_type = |value, meaning| {
        contextual_code(
            value,
            "SCT",
            meaning,
            "8135",
            "20210712",
            "1.2.840.10008.6.1.1365",
        )
    };
    let class = |id: &str, label: &str, geometry, color, category, property_type| {
        AnnotationClass::new(
            id.into(),
            label.into(),
            geometry,
            color,
            category,
            property_type,
            Vec::new(),
        )
    };
    let cell_category = || DicomCode::new("4421005", "SCT", "Cell Structure");
    AnnotationScheme::new(
        "org.frames.general-pathology".into(),
        1,
        "General Pathology".into(),
        vec![
            class(
                "tissue",
                "Tissue",
                AnnotationClassGeometry::Region,
                [0x99, 0x99, 0x99],
                category("85756007", "Tissue")?,
                tissue_type("85756007", "Tissue")?,
            )?,
            class(
                "neoplasm",
                "Neoplasm",
                AnnotationClassGeometry::Region,
                [0xD5, 0x5E, 0x00],
                category("49755003", "Morphologically Abnormal Structure")?,
                lesion_type("108369006", "Neoplasm")?,
            )?,
            class(
                "necrosis",
                "Necrosis",
                AnnotationClassGeometry::Region,
                [0xE6, 0x9F, 0x00],
                category("49755003", "Morphologically Abnormal Structure")?,
                lesion_type("6574001", "Necrosis")?,
            )?,
            class(
                "inflammation",
                "Inflammation",
                AnnotationClassGeometry::Region,
                [0xF0, 0xE4, 0x42],
                category("49755003", "Morphologically Abnormal Structure")?,
                lesion_type("409774005", "Inflammation")?,
            )?,
            class(
                "stroma",
                "Stroma / connective tissue",
                AnnotationClassGeometry::Region,
                [0x00, 0x9E, 0x73],
                category("85756007", "Tissue")?,
                tissue_type("21793004", "Connective tissue")?,
            )?,
            class(
                "unusable-tissue",
                "Unusable tissue",
                AnnotationClassGeometry::Region,
                [0xCC, 0x79, 0xA7],
                category("263496004", "Quality")?,
                contextual_code(
                    "131502",
                    "DCM",
                    "Unusable tissue",
                    "7164",
                    "20260319",
                    "1.2.840.10008.6.1.1568",
                )?,
            )?,
            class(
                "cell",
                "Cell",
                AnnotationClassGeometry::Point,
                [0x56, 0xB4, 0xE9],
                cell_category()?,
                microscopy_type("4421005", "Cell")?,
            )?,
            class(
                "nucleus",
                "Nucleus",
                AnnotationClassGeometry::Point,
                [0x00, 0x72, 0xB2],
                cell_category()?,
                microscopy_type("84640000", "Nucleus")?,
            )?,
        ],
        Vec::new(),
    )
}

pub(super) fn builtin_tumor_compatibility() -> Result<AnnotationScheme> {
    AnnotationScheme::new(
        "org.frames.tumor-mask-compatibility".into(),
        1,
        "Tumor Mask Compatibility".into(),
        vec![
            AnnotationClass::new(
                "viable-tumor".into(),
                "Viable tumor".into(),
                AnnotationClassGeometry::Region,
                [0x00, 0x9E, 0x73],
                contextual_code(
                    "49755003",
                    "SCT",
                    "Morphologically Abnormal Structure",
                    "7150",
                    "20260319",
                    "1.2.840.10008.6.1.496",
                )?,
                DicomCode::new("VIABLE_TUMOR", "99FRAMES", "Viable tumor")?,
                Vec::new(),
            )?,
            AnnotationClass::new(
                "cell".into(),
                "Cell".into(),
                AnnotationClassGeometry::Point,
                [0x56, 0xB4, 0xE9],
                DicomCode::new("49755003", "SCT", "Morphologically Abnormal Structure")?,
                DicomCode::new("4421005", "SCT", "Cell")?,
                Vec::new(),
            )?,
        ],
        Vec::new(),
    )
}

fn contextual_code(
    value: &str,
    scheme: &str,
    meaning: &str,
    context_identifier: &str,
    context_group_version: &str,
    context_uid: &str,
) -> Result<DicomCode> {
    DicomCode::new(value, scheme, meaning)?
        .with_context_identifier(context_identifier, "DCMR", context_group_version)?
        .with_context_uid(context_uid, DCMR_UID)
}
