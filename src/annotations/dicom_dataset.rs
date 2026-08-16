use chrono::{Datelike, Local, Timelike};
use dicom_core::value::{DataSetSequence, PrimitiveValue, Value};
use dicom_core::{DataElement, Length, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use crate::{Error, Result};

pub(crate) fn sequence(tag: Tag, items: Vec<InMemDicomObject>) -> DataElement<InMemDicomObject> {
    DataElement::new(
        tag,
        VR::SQ,
        Value::from(DataSetSequence::new(items, Length::UNDEFINED)),
    )
}

pub(crate) fn put_text(object: &mut InMemDicomObject, tag: Tag, vr: VR, value: &str) {
    object.put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
}

pub(crate) fn dimension_index_item(
    uid: &str,
    pointer: Tag,
    functional_group_pointer: Tag,
    description: &str,
) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    put_text(&mut item, tags::DIMENSION_ORGANIZATION_UID, VR::UI, uid);
    item.put(DataElement::new(
        tags::DIMENSION_INDEX_POINTER,
        VR::AT,
        PrimitiveValue::from(pointer),
    ));
    item.put(DataElement::new(
        tags::FUNCTIONAL_GROUP_POINTER,
        VR::AT,
        PrimitiveValue::from(functional_group_pointer),
    ));
    put_text(
        &mut item,
        tags::DIMENSION_DESCRIPTION_LABEL,
        VR::LO,
        description,
    );
    item
}

pub(crate) fn put_optional_text(
    object: &mut InMemDicomObject,
    tag: Tag,
    vr: VR,
    value: Option<&str>,
) {
    if let Some(value) = value {
        put_text(object, tag, vr, value);
    }
}

pub(crate) fn new_dicom_uid() -> String {
    format!("2.25.{}", uuid::Uuid::new_v4().as_u128())
}

pub(crate) fn dicom_now() -> (String, String) {
    let now = Local::now();
    (
        format!("{:04}{:02}{:02}", now.year(), now.month(), now.day()),
        format!(
            "{:02}{:02}{:02}.{:06}",
            now.hour(),
            now.minute(),
            now.second(),
            now.nanosecond() / 1_000
        ),
    )
}

pub(crate) fn sequence_items(object: &InMemDicomObject, tag: Tag) -> Result<&[InMemDicomObject]> {
    object
        .get(tag)
        .and_then(|element| element.items())
        .ok_or_else(|| Error::InvalidInput(format!("missing or invalid sequence {tag}")))
}

pub(crate) fn required_string(object: &InMemDicomObject, tag: Tag) -> Result<String> {
    optional_string(object, tag).ok_or_else(|| {
        Error::InvalidInput(format!("missing or empty required DICOM attribute {tag}"))
    })
}

pub(crate) fn required_u16(object: &InMemDicomObject, tag: Tag) -> Result<u16> {
    object
        .get(tag)
        .and_then(|element| element.to_int::<u16>().ok())
        .ok_or_else(|| Error::InvalidInput(format!("missing or invalid attribute {tag}")))
}

pub(crate) fn required_u32(object: &InMemDicomObject, tag: Tag) -> Result<u32> {
    optional_u32(object, tag)
        .ok_or_else(|| Error::InvalidInput(format!("missing or invalid attribute {tag}")))
}

pub(crate) fn optional_string(object: &InMemDicomObject, tag: Tag) -> Option<String> {
    object
        .get(tag)
        .and_then(|element| element.to_str().ok())
        .map(|value| value.trim_end_matches('\0').trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn optional_strings(object: &InMemDicomObject, tag: Tag) -> Vec<String> {
    optional_string(object, tag)
        .map(|value| {
            value
                .split('\\')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn optional_u32(object: &InMemDicomObject, tag: Tag) -> Option<u32> {
    object
        .get(tag)
        .and_then(|element| element.to_int::<u32>().ok())
}

pub(crate) fn read_yes_no(object: &InMemDicomObject, tag: Tag, default: bool) -> Result<bool> {
    match optional_string(object, tag).as_deref() {
        Some("YES") => Ok(true),
        Some("NO") => Ok(false),
        None => Ok(default),
        Some(value) => Err(Error::InvalidInput(format!(
            "attribute {tag} must be YES or NO, got {value:?}"
        ))),
    }
}
