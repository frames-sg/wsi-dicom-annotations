use std::path::Path;

use dicom_core::value::PrimitiveValue;
use dicom_core::{DataElement, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;

use crate::{Error, Result};

use super::super::coded_content::{code_item, write_algorithm};
use super::super::derived_object::{
    add_common_instance_reference, build_common_object, copy_or_empty, series_reference_item,
    sop_reference_item,
};
use super::super::dicom_dataset::{dicom_now, put_optional_text, put_text, sequence};
use super::super::dicom_file::{atomic_write_dicom, ensure_sidecar_destination};
use super::super::model::{AnnotationGroup, AnnotationMeasurement};
use super::geometry_codec::encoded_geometry;
use super::AnnotationDocument;

pub(super) fn write_ann(
    document: &AnnotationDocument,
    path: &Path,
    allow_lossy: bool,
) -> Result<()> {
    ensure_sidecar_destination(path, document.source.source_path())?;
    document.validate()?;
    document.ensure_roundtrip_safe(allow_lossy)?;
    let mut object = build_common_object(
        &document.source,
        uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE,
        &document.sop_instance_uid,
        &document.series_instance_uid,
        "ANN",
        &document.series_equipment,
    )?;
    let source_metadata = document.source.source_metadata()?;
    let (date, time) = dicom_now();
    put_text(&mut object, tags::CONTENT_DATE, VR::DA, &date);
    put_text(&mut object, tags::CONTENT_TIME, VR::TM, &time);
    put_text(
        &mut object,
        tags::CONTENT_LABEL,
        VR::CS,
        &document.content_label,
    );
    put_text(
        &mut object,
        tags::CONTENT_DESCRIPTION,
        VR::LO,
        &document.content_description,
    );
    put_optional_text(
        &mut object,
        tags::CONTENT_CREATOR_NAME,
        VR::PN,
        document.content_creator_name.as_deref(),
    );
    put_text(
        &mut object,
        tags::ANNOTATION_COORDINATE_TYPE,
        VR::CS,
        &document.coordinate_type,
    );
    if document.coordinate_type == "2D" {
        put_text(
            &mut object,
            tags::PIXEL_ORIGIN_INTERPRETATION,
            VR::CS,
            document
                .pixel_origin_interpretation
                .as_deref()
                .ok_or_else(|| {
                    Error::InvalidInput("2D ANN has no Pixel Origin Interpretation".into())
                })?,
        );
    } else if let Some(frame_of_reference_uid) = document.source.frame_of_reference_uid() {
        put_text(
            &mut object,
            tags::FRAME_OF_REFERENCE_UID,
            VR::UI,
            frame_of_reference_uid,
        );
        copy_or_empty(
            &source_metadata,
            &mut object,
            tags::POSITION_REFERENCE_INDICATOR,
            VR::LO,
        );
    } else {
        return Err(Error::InvalidInput(
            "3D ANN source has no Frame of Reference UID".into(),
        ));
    }
    if document.references_source_image {
        let mut reference = sop_reference_item(
            document.source.sop_class_uid(),
            document.source.sop_instance_uid(),
        );
        if let Some(frame) = document.referenced_frame_number {
            put_text(
                &mut reference,
                tags::REFERENCED_FRAME_NUMBER,
                VR::IS,
                &frame.to_string(),
            );
        }
        object.put(sequence(tags::REFERENCED_IMAGE_SEQUENCE, vec![reference]));
    }
    let groups = document
        .groups
        .iter()
        .enumerate()
        .map(|(index, group)| write_group(index, group, document.coordinate_type == "3D"))
        .collect::<Result<Vec<_>>>()?;
    object.put(sequence(tags::ANNOTATION_GROUP_SEQUENCE, groups));
    if document.references_source_image {
        add_common_instance_reference(&mut object, &document.source);
    }

    if let Some(predecessor_uid) = &document.predecessor_sop_instance_uid {
        let mut referenced_series = Vec::new();
        if document.references_source_image {
            referenced_series.push(series_reference_item(
                document.source.series_instance_uid(),
                document.source.sop_class_uid(),
                document.source.sop_instance_uid(),
            ));
        }
        referenced_series.push(series_reference_item(
            &document.series_instance_uid,
            uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE,
            predecessor_uid,
        ));
        object.put(sequence(
            tags::REFERENCED_SERIES_SEQUENCE,
            referenced_series,
        ));
    }
    atomic_write_dicom(
        path,
        object,
        uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE,
        &document.sop_instance_uid,
    )
}

fn write_group(
    index: usize,
    group: &AnnotationGroup,
    three_dimensional: bool,
) -> Result<InMemDicomObject> {
    let group_number = u16::try_from(index + 1).map_err(|_| {
        Error::InvalidInput("annotation group number exceeds DICOM US range".into())
    })?;
    let annotation_count = u32::try_from(group.geometry().annotation_count())
        .map_err(|_| Error::InvalidInput("annotation count exceeds DICOM UL range".into()))?;
    let mut item = InMemDicomObject::new_empty();
    item.put(DataElement::new(
        tags::ANNOTATION_GROUP_NUMBER,
        VR::US,
        PrimitiveValue::from(group_number),
    ));
    put_text(&mut item, tags::ANNOTATION_GROUP_UID, VR::UI, group.uid());
    put_text(
        &mut item,
        tags::ANNOTATION_GROUP_LABEL,
        VR::LO,
        group.label(),
    );
    if !group.description().is_empty() {
        put_text(
            &mut item,
            tags::ANNOTATION_GROUP_DESCRIPTION,
            VR::UT,
            group.description(),
        );
    }
    put_text(
        &mut item,
        tags::ANNOTATION_GROUP_GENERATION_TYPE,
        VR::CS,
        group.generation_type().dicom_value(),
    );
    if !group.algorithms().is_empty() {
        item.put(sequence(
            tags::ANNOTATION_GROUP_ALGORITHM_IDENTIFICATION_SEQUENCE,
            group.algorithms().iter().map(write_algorithm).collect(),
        ));
    }
    if !group.anatomic_regions().is_empty() {
        item.put(sequence(
            tags::ANATOMIC_REGION_SEQUENCE,
            group.anatomic_regions().iter().map(code_item).collect(),
        ));
    }
    if !group.primary_anatomic_structures().is_empty() {
        item.put(sequence(
            tags::PRIMARY_ANATOMIC_STRUCTURE_SEQUENCE,
            group
                .primary_anatomic_structures()
                .iter()
                .map(code_item)
                .collect(),
        ));
    }
    item.put(sequence(
        tags::ANNOTATION_PROPERTY_CATEGORY_CODE_SEQUENCE,
        vec![code_item(group.category())],
    ));
    item.put(sequence(
        tags::ANNOTATION_PROPERTY_TYPE_CODE_SEQUENCE,
        vec![code_item(group.property_type())],
    ));
    if !group.property_type_modifiers().is_empty() {
        item.put(sequence(
            tags::ANNOTATION_PROPERTY_TYPE_MODIFIER_CODE_SEQUENCE,
            group
                .property_type_modifiers()
                .iter()
                .map(code_item)
                .collect(),
        ));
    }
    item.put(DataElement::new(
        tags::NUMBER_OF_ANNOTATIONS,
        VR::UL,
        PrimitiveValue::from(annotation_count),
    ));
    put_text(
        &mut item,
        tags::GRAPHIC_TYPE,
        VR::CS,
        group.geometry().graphic_type().dicom_value(),
    );
    put_text(
        &mut item,
        tags::ANNOTATION_APPLIES_TO_ALL_OPTICAL_PATHS,
        VR::CS,
        if group.applies_to_all_optical_paths() {
            "YES"
        } else {
            "NO"
        },
    );
    if !group.applies_to_all_optical_paths() {
        put_text(
            &mut item,
            tags::REFERENCED_OPTICAL_PATH_IDENTIFIER,
            VR::SH,
            &group.referenced_optical_paths().join("\\"),
        );
    }
    if three_dimensional {
        put_text(
            &mut item,
            tags::ANNOTATION_APPLIES_TO_ALL_Z_PLANES,
            VR::CS,
            if group.applies_to_all_z_planes() {
                "YES"
            } else {
                "NO"
            },
        );
        if !group.common_z_coordinates().is_empty() {
            item.put(DataElement::new(
                tags::COMMON_Z_COORDINATE_VALUE,
                VR::FD,
                PrimitiveValue::F64(group.common_z_coordinates().to_vec().into()),
            ));
        }
    }
    item.put(DataElement::new(
        tags::RECOMMENDED_DISPLAY_CIE_LAB_VALUE,
        VR::US,
        PrimitiveValue::from(group.recommended_display_cielab()),
    ));

    let (coordinates, indices) = encoded_geometry(group.geometry())?;
    item.put(DataElement::new(
        tags::DOUBLE_POINT_COORDINATES_DATA,
        VR::OD,
        PrimitiveValue::F64(coordinates.into()),
    ));
    if let Some(indices) = indices {
        item.put(DataElement::new(
            tags::LONG_PRIMITIVE_POINT_INDEX_LIST,
            VR::OL,
            PrimitiveValue::U32(indices.into()),
        ));
    }
    if !group.measurements().is_empty() {
        item.put(sequence(
            tags::MEASUREMENTS_SEQUENCE,
            group.measurements().iter().map(write_measurement).collect(),
        ));
    }
    Ok(item)
}

fn write_measurement(measurement: &AnnotationMeasurement) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    item.put(sequence(
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        vec![code_item(measurement.concept())],
    ));
    item.put(sequence(
        tags::MEASUREMENT_UNITS_CODE_SEQUENCE,
        vec![code_item(measurement.units())],
    ));
    let mut values_item = InMemDicomObject::new_empty();
    values_item.put(DataElement::new(
        tags::FLOATING_POINT_VALUES,
        VR::OF,
        PrimitiveValue::F32(
            measurement
                .values()
                .iter()
                .map(|value| *value as f32)
                .collect::<Vec<_>>()
                .into(),
        ),
    ));
    if let Some(indices) = measurement.annotation_indices() {
        values_item.put(DataElement::new(
            tags::ANNOTATION_INDEX_LIST,
            VR::OL,
            PrimitiveValue::U32(indices.to_vec().into()),
        ));
    }
    item.put(sequence(
        tags::MEASUREMENT_VALUES_SEQUENCE,
        vec![values_item],
    ));
    item
}
