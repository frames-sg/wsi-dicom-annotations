use dicom_core::value::PrimitiveValue;
use dicom_core::{DataElement, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use crate::annotations::coded_content::code_item;
use crate::annotations::dicom_dataset::{put_text, sequence};
use crate::{DicomCode, Result};

use super::super::document::ParametricMapDocument;

pub(super) fn real_world_value_item(
    document: &ParametricMapDocument,
    channel_index: usize,
    channel_ordinal: u32,
) -> Result<InMemDicomObject> {
    let channel = &document.profile.channels[channel_index];
    let range = document.channel_ranges[channel_ordinal as usize - 1];
    let mut item = InMemDicomObject::new_empty();
    item.put(DataElement::new(
        tags::DOUBLE_FLOAT_REAL_WORLD_VALUE_FIRST_VALUE_MAPPED,
        VR::FD,
        PrimitiveValue::F64(vec![range.minimum].into()),
    ));
    item.put(DataElement::new(
        tags::DOUBLE_FLOAT_REAL_WORLD_VALUE_LAST_VALUE_MAPPED,
        VR::FD,
        PrimitiveValue::F64(vec![range.maximum].into()),
    ));
    item.put(DataElement::new(
        tags::REAL_WORLD_VALUE_INTERCEPT,
        VR::FD,
        PrimitiveValue::F64(vec![0.0].into()),
    ));
    item.put(DataElement::new(
        tags::REAL_WORLD_VALUE_SLOPE,
        VR::FD,
        PrimitiveValue::F64(vec![1.0].into()),
    ));
    put_text(&mut item, tags::LUT_EXPLANATION, VR::LO, &channel.name);
    put_text(
        &mut item,
        tags::LUT_LABEL,
        VR::SH,
        &format!("PARAM{channel_ordinal:04}"),
    );
    item.put(sequence(
        tags::MEASUREMENT_UNITS_CODE_SEQUENCE,
        vec![code_item(&channel.unit)],
    ));
    item.put(sequence(
        tags::QUANTITY_DEFINITION_SEQUENCE,
        quantity_definition_items(document, &channel.quantity)?,
    ));
    Ok(item)
}

fn quantity_definition_items(
    document: &ParametricMapDocument,
    quantity: &DicomCode,
) -> Result<Vec<InMemDicomObject>> {
    let algorithm = &document.profile.algorithm;
    let mut items = vec![coded_content_item(
        DicomCode::new("246205007", "SCT", "Quantity")?,
        quantity.clone(),
    )];
    items.push(text_content_item(
        DicomCode::new("111001", "DCM", "Algorithm Name")?,
        algorithm.name(),
    ));
    if let Some(name_code) = algorithm.name_code() {
        items.push(coded_content_item(
            DicomCode::new("111001", "DCM", "Algorithm Name")?,
            name_code.clone(),
        ));
    }
    items.push(text_content_item(
        DicomCode::new("111003", "DCM", "Algorithm Version")?,
        algorithm.version(),
    ));
    items.push(coded_content_item(
        DicomCode::new("111000", "DCM", "Algorithm Family")?,
        algorithm.family().clone(),
    ));
    if let Some(parameters) = algorithm.parameters() {
        items.push(text_content_item(
            DicomCode::new("111002", "DCM", "Algorithm Parameters")?,
            parameters,
        ));
    }
    if let Some(source) = algorithm.source() {
        items.push(text_content_item(
            DicomCode::new("122405", "DCM", "Algorithm Manufacturer")?,
            source,
        ));
    }
    Ok(items)
}

fn coded_content_item(name: DicomCode, value: DicomCode) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    put_text(&mut item, tags::VALUE_TYPE, VR::CS, "CODE");
    item.put(sequence(
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        vec![code_item(&name)],
    ));
    item.put(sequence(
        tags::CONCEPT_CODE_SEQUENCE,
        vec![code_item(&value)],
    ));
    item
}

fn text_content_item(name: DicomCode, value: &str) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    put_text(&mut item, tags::VALUE_TYPE, VR::CS, "TEXT");
    item.put(sequence(
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        vec![code_item(&name)],
    ));
    put_text(&mut item, tags::TEXT_VALUE, VR::UT, value);
    item
}

pub(super) fn add_algorithm_provenance(
    document: &ParametricMapDocument,
    object: &mut InMemDicomObject,
) -> Result<()> {
    let algorithm = &document.profile.algorithm;
    let mut equipment = InMemDicomObject::new_empty();
    equipment.put(sequence(
        tags::PURPOSE_OF_REFERENCE_CODE_SEQUENCE,
        vec![code_item(&DicomCode::new(
            "109102",
            "DCM",
            "Processing Equipment",
        )?)],
    ));
    put_text(
        &mut equipment,
        tags::MANUFACTURER,
        VR::LO,
        algorithm
            .source()
            .unwrap_or(document.producer.manufacturer()),
    );
    put_text(
        &mut equipment,
        tags::MANUFACTURER_MODEL_NAME,
        VR::LO,
        algorithm.name(),
    );
    put_text(
        &mut equipment,
        tags::SOFTWARE_VERSIONS,
        VR::LO,
        algorithm.version(),
    );
    put_text(
        &mut equipment,
        tags::CONTRIBUTION_DESCRIPTION,
        VR::ST,
        "Generated quantitative pathology pixel values",
    );
    object.put(sequence(
        tags::CONTRIBUTING_EQUIPMENT_SEQUENCE,
        vec![equipment],
    ));
    Ok(())
}
