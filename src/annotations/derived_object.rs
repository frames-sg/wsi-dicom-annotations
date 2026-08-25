use dicom_core::{Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use crate::{Error, Result};

use super::context::DicomAnnotationContext;
use super::dicom_dataset::{dicom_now, optional_string, put_optional_text, put_text, sequence};

/// Caller-owned identity and series metadata for a newly derived DICOM object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedObjectProducer {
    series_number: String,
    series_description: Option<String>,
    manufacturer: String,
    manufacturer_model_name: String,
    device_serial_number: String,
    software_versions: String,
}

impl DerivedObjectProducer {
    /// Creates validated producer metadata with an explicit DICOM Series Number.
    pub fn new(
        series_number: i32,
        manufacturer: impl Into<String>,
        manufacturer_model_name: impl Into<String>,
        device_serial_number: impl Into<String>,
        software_versions: impl Into<String>,
    ) -> Result<Self> {
        let producer = Self {
            series_number: series_number.to_string(),
            series_description: None,
            manufacturer: manufacturer.into(),
            manufacturer_model_name: manufacturer_model_name.into(),
            device_serial_number: device_serial_number.into(),
            software_versions: software_versions.into(),
        };
        producer.validate()?;
        Ok(producer)
    }

    /// Adds an explicit DICOM Series Description.
    pub fn with_series_description(mut self, description: impl Into<String>) -> Result<Self> {
        self.series_description = Some(description.into());
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn library_default(series_number: i32, series_description: &str) -> Self {
        Self {
            series_number: series_number.to_string(),
            series_description: Some(series_description.into()),
            manufacturer: "wsi-dicom-annotations project".into(),
            manufacturer_model_name: env!("CARGO_PKG_NAME").into(),
            device_serial_number: "not-applicable".into(),
            software_versions: env!("CARGO_PKG_VERSION").into(),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.series_number.len() > 12 || self.series_number.parse::<i32>().is_err() {
            return Err(Error::InvalidInput(
                "producer series number is not a valid DICOM IS integer".into(),
            ));
        }
        if let Some(description) = &self.series_description {
            validate_equipment_text("series description", description, false)?;
        }
        validate_equipment_text("manufacturer", &self.manufacturer, true)?;
        validate_equipment_text(
            "manufacturer model name",
            &self.manufacturer_model_name,
            true,
        )?;
        validate_equipment_text("device serial number", &self.device_serial_number, true)?;
        validate_equipment_text("software versions", &self.software_versions, true)
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

    pub(crate) fn write(&self, object: &mut InMemDicomObject) {
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

    #[must_use]
    pub fn series_number(&self) -> &str {
        &self.series_number
    }

    #[must_use]
    pub fn series_description(&self) -> Option<&str> {
        self.series_description.as_deref()
    }

    #[must_use]
    pub fn manufacturer(&self) -> &str {
        &self.manufacturer
    }

    #[must_use]
    pub fn manufacturer_model_name(&self) -> &str {
        &self.manufacturer_model_name
    }

    #[must_use]
    pub fn device_serial_number(&self) -> &str {
        &self.device_serial_number
    }

    #[must_use]
    pub fn software_versions(&self) -> &str {
        &self.software_versions
    }
}

fn validate_equipment_text(name: &str, value: &str, allow_empty: bool) -> Result<()> {
    if (!allow_empty && value.trim().is_empty()) || value.len() > 64 || value.contains(['\\', '\0'])
    {
        return Err(Error::InvalidInput(format!(
            "producer {name} must {}fit in 64 bytes and contain no DICOM separator or NUL",
            if allow_empty { "" } else { "be nonempty and " }
        )));
    }
    Ok(())
}

pub(crate) fn build_common_object(
    context: &DicomAnnotationContext,
    sop_class_uid: &str,
    sop_instance_uid: &str,
    series_instance_uid: &str,
    modality: &str,
    producer: &DerivedObjectProducer,
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
    producer.write(&mut object);
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
