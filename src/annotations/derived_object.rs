use dicom_core::{Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use crate::Result;

use super::context::DicomAnnotationContext;
use super::dicom_dataset::{dicom_now, optional_string, put_optional_text, put_text, sequence};
#[cfg(feature = "parametric-map")]
use super::model::AlgorithmIdentification;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SeriesEquipmentMetadata {
    series_number: String,
    series_description: Option<String>,
    manufacturer: String,
    manufacturer_model_name: String,
    device_serial_number: String,
    software_versions: String,
}

impl SeriesEquipmentMetadata {
    pub(crate) fn viewer(series_number: &str) -> Self {
        Self {
            series_number: series_number.into(),
            series_description: Some("WSI annotations".into()),
            manufacturer: "Frames".into(),
            manufacturer_model_name: "DICOM Viewer".into(),
            device_serial_number: "LOCAL".into(),
            software_versions: env!("CARGO_PKG_VERSION").into(),
        }
    }

    #[cfg(feature = "parametric-map")]
    pub(crate) fn parametric_map(algorithm: &AlgorithmIdentification) -> Self {
        Self {
            series_number: "9401".into(),
            series_description: Some("WSI parametric maps".into()),
            manufacturer: algorithm.source().unwrap_or("Frames").into(),
            manufacturer_model_name: algorithm.name().into(),
            device_serial_number: "LOCAL".into(),
            software_versions: algorithm.version().into(),
        }
    }

    pub(crate) fn read(object: &InMemDicomObject) -> Self {
        Self {
            series_number: optional_string(object, tags::SERIES_NUMBER).unwrap_or_default(),
            series_description: optional_string(object, tags::SERIES_DESCRIPTION),
            manufacturer: optional_string(object, tags::MANUFACTURER).unwrap_or_default(),
            manufacturer_model_name: optional_string(object, tags::MANUFACTURER_MODEL_NAME)
                .unwrap_or_default(),
            device_serial_number: optional_string(object, tags::DEVICE_SERIAL_NUMBER)
                .unwrap_or_default(),
            software_versions: optional_string(object, tags::SOFTWARE_VERSIONS).unwrap_or_default(),
        }
    }

    fn write(&self, object: &mut InMemDicomObject) {
        put_text(object, tags::SERIES_NUMBER, VR::IS, &self.series_number);
        put_optional_text(
            object,
            tags::SERIES_DESCRIPTION,
            VR::LO,
            self.series_description.as_deref(),
        );
        for (tag, value) in [
            (tags::MANUFACTURER, self.manufacturer.as_str()),
            (
                tags::MANUFACTURER_MODEL_NAME,
                self.manufacturer_model_name.as_str(),
            ),
            (
                tags::DEVICE_SERIAL_NUMBER,
                self.device_serial_number.as_str(),
            ),
            (tags::SOFTWARE_VERSIONS, self.software_versions.as_str()),
        ] {
            put_text(object, tag, VR::LO, value);
        }
    }
}

pub(crate) fn build_common_object(
    context: &DicomAnnotationContext,
    sop_class_uid: &str,
    sop_instance_uid: &str,
    series_instance_uid: &str,
    modality: &str,
    series_equipment: &SeriesEquipmentMetadata,
) -> Result<InMemDicomObject> {
    let source = context.source_metadata()?;
    let mut object = InMemDicomObject::new_empty();
    copy_or_empty(&source, &mut object, tags::PATIENT_NAME, VR::PN);
    copy_or_empty(&source, &mut object, tags::PATIENT_ID, VR::LO);
    copy_or_empty(&source, &mut object, tags::PATIENT_BIRTH_DATE, VR::DA);
    copy_or_empty(&source, &mut object, tags::PATIENT_SEX, VR::CS);
    put_text(
        &mut object,
        tags::STUDY_INSTANCE_UID,
        VR::UI,
        context.study_instance_uid(),
    );
    for (tag, vr) in [
        (tags::STUDY_DATE, VR::DA),
        (tags::STUDY_TIME, VR::TM),
        (tags::REFERRING_PHYSICIAN_NAME, VR::PN),
        (tags::STUDY_ID, VR::SH),
        (tags::ACCESSION_NUMBER, VR::SH),
    ] {
        copy_or_empty(&source, &mut object, tag, vr);
    }
    for tag in [
        tags::ISSUER_OF_PATIENT_ID,
        tags::OTHER_PATIENT_I_DS_SEQUENCE,
        tags::ISSUER_OF_ACCESSION_NUMBER_SEQUENCE,
        tags::PATIENT_IDENTITY_REMOVED,
        tags::DEIDENTIFICATION_METHOD,
        tags::DEIDENTIFICATION_METHOD_CODE_SEQUENCE,
        tags::SPECIMEN_DESCRIPTION_SEQUENCE,
        tags::CONTAINER_IDENTIFIER,
        tags::ISSUER_OF_THE_CONTAINER_IDENTIFIER_SEQUENCE,
        tags::CONTAINER_TYPE_CODE_SEQUENCE,
    ] {
        copy_if_present(&source, &mut object, tag);
    }
    put_text(&mut object, tags::MODALITY, VR::CS, modality);
    copy_if_present(&source, &mut object, tags::LATERALITY);
    put_text(
        &mut object,
        tags::SERIES_INSTANCE_UID,
        VR::UI,
        series_instance_uid,
    );
    series_equipment.write(&mut object);
    let (date, time) = dicom_now();
    put_text(&mut object, tags::INSTANCE_CREATION_DATE, VR::DA, &date);
    put_text(&mut object, tags::INSTANCE_CREATION_TIME, VR::TM, &time);
    put_text(&mut object, tags::SOP_CLASS_UID, VR::UI, sop_class_uid);
    put_text(
        &mut object,
        tags::SOP_INSTANCE_UID,
        VR::UI,
        sop_instance_uid,
    );
    put_text(&mut object, tags::INSTANCE_NUMBER, VR::IS, "1");
    put_text(
        &mut object,
        tags::SPECIFIC_CHARACTER_SET,
        VR::CS,
        "ISO_IR 192",
    );
    Ok(object)
}

pub(crate) fn add_common_instance_reference(
    object: &mut InMemDicomObject,
    source: &DicomAnnotationContext,
) {
    object.put(sequence(
        tags::REFERENCED_SERIES_SEQUENCE,
        vec![series_reference_item(
            source.series_instance_uid(),
            source.sop_class_uid(),
            source.sop_instance_uid(),
        )],
    ));
}

pub(crate) fn series_reference_item(
    series_instance_uid: &str,
    sop_class_uid: &str,
    sop_instance_uid: &str,
) -> InMemDicomObject {
    let mut series = InMemDicomObject::new_empty();
    put_text(
        &mut series,
        tags::SERIES_INSTANCE_UID,
        VR::UI,
        series_instance_uid,
    );
    series.put(sequence(
        tags::REFERENCED_INSTANCE_SEQUENCE,
        vec![sop_reference_item(sop_class_uid, sop_instance_uid)],
    ));
    series
}

pub(crate) fn sop_reference_item(sop_class_uid: &str, sop_instance_uid: &str) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    put_text(
        &mut item,
        tags::REFERENCED_SOP_CLASS_UID,
        VR::UI,
        sop_class_uid,
    );
    put_text(
        &mut item,
        tags::REFERENCED_SOP_INSTANCE_UID,
        VR::UI,
        sop_instance_uid,
    );
    item
}

pub(crate) fn copy_or_empty(
    source: &dicom_object::DefaultDicomObject,
    target: &mut InMemDicomObject,
    tag: Tag,
    vr: VR,
) {
    if let Some(element) = source.get(tag) {
        target.put(element.clone());
    } else {
        put_text(target, tag, vr, "");
    }
}

fn copy_if_present(
    source: &dicom_object::DefaultDicomObject,
    target: &mut InMemDicomObject,
    tag: Tag,
) {
    if let Some(element) = source.get(tag) {
        target.put(element.clone());
    }
}
