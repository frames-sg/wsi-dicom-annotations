use std::path::Path;

use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;

use crate::{Error, Result};

use super::super::coded_content::{read_algorithms, read_code_at, read_codes_at};
use super::super::context::DicomAnnotationContext;
use super::super::derived_object::DerivedObjectProducer;
use super::super::dicom_dataset::{
    optional_string, optional_strings, read_yes_no, required_string, sequence_items,
};
use super::super::dicom_file::enforce_file_limit;
use super::super::model::{
    AnnotationGeometry, AnnotationGraphicType, AnnotationGroup, AnnotationMeasurement,
    DiagnosticDisposition, DiagnosticSeverity, FindingSemantics, GenerationType,
    InteroperabilityDiagnostic, Point2,
};
use super::geometry_codec::{decode_polygons, primitive_point_offset, validate_encoded_geometry};
use super::{AnnotationDocument, MAX_ANNOTATION_GROUPS, MAX_ANN_FILE_BYTES, MAX_COORDINATE_VALUES};

pub(super) fn read_ann(path: &Path, source: &DicomAnnotationContext) -> Result<AnnotationDocument> {
    enforce_file_limit(path, MAX_ANN_FILE_BYTES, "ANN")?;
    let object = dicom_object::open_file(path).map_err(|error| Error::DicomRead {
        path: path.to_path_buf(),
        source: Box::new(error),
    })?;
    if object.meta().media_storage_sop_class_uid()
        != uids::MICROSCOPY_BULK_SIMPLE_ANNOTATIONS_STORAGE
    {
        return Err(Error::Unsupported(format!(
            "{} is not a Microscopy Bulk Simple Annotations instance",
            path.display()
        )));
    }
    if required_string(&object, tags::STUDY_INSTANCE_UID)? != source.study_instance_uid() {
        return Err(Error::InvalidInput(
            "ANN Study Instance UID does not match the open WSI".into(),
        ));
    }
    let coordinate_type = required_string(&object, tags::ANNOTATION_COORDINATE_TYPE)?;
    let pixel_origin = optional_string(&object, tags::PIXEL_ORIGIN_INTERPRETATION);
    let mut references_source_image = false;
    let mut referenced_frame_number = None;
    match coordinate_type.as_str() {
        "2D" => {
            let referenced = sequence_items(&object, tags::REFERENCED_IMAGE_SEQUENCE)?;
            if referenced.len() != 1 {
                return Err(Error::InvalidInput(
                    "2D ANN must reference exactly one source image".into(),
                ));
            }
            let referenced_uid =
                required_string(&referenced[0], tags::REFERENCED_SOP_INSTANCE_UID)?;
            let referenced_class = required_string(&referenced[0], tags::REFERENCED_SOP_CLASS_UID)?;
            if referenced_uid != source.sop_instance_uid()
                || referenced_class != source.sop_class_uid()
            {
                return Err(Error::InvalidInput(format!(
                    "ANN references SOP Instance {referenced_uid} with Class {referenced_class}, not the open WSI {}",
                    source.sop_instance_uid()
                )));
            }
            references_source_image = true;
            referenced_frame_number = optional_referenced_frame_number(&referenced[0])?;
            match pixel_origin.as_deref() {
                Some("FRAME") if referenced_frame_number.is_none() => {
                    return Err(Error::InvalidInput(
                        "FRAME-relative ANN must reference exactly one source frame".into(),
                    ));
                }
                Some("VOLUME") if referenced_frame_number.is_some() => {
                    return Err(Error::InvalidInput(
                        "VOLUME-relative ANN cannot designate a source frame".into(),
                    ));
                }
                Some("FRAME" | "VOLUME") => {}
                Some(other) => {
                    return Err(Error::Unsupported(format!(
                        "ANN Pixel Origin Interpretation {other:?} is not recognized"
                    )));
                }
                None => {
                    return Err(Error::InvalidInput(
                        "2D ANN is missing Pixel Origin Interpretation".into(),
                    ));
                }
            }
        }
        "3D" => {
            let source_frame_of_reference = source.frame_of_reference_uid().ok_or_else(|| {
                Error::InvalidInput("3D ANN source has no Frame of Reference UID".into())
            })?;
            if required_string(&object, tags::FRAME_OF_REFERENCE_UID)? != source_frame_of_reference
            {
                return Err(Error::InvalidInput(
                    "3D ANN Frame of Reference UID does not match the open WSI".into(),
                ));
            }
            if let Some(referenced) = object
                .get(tags::REFERENCED_IMAGE_SEQUENCE)
                .and_then(|element| element.items())
            {
                if referenced.len() != 1
                    || required_string(&referenced[0], tags::REFERENCED_SOP_INSTANCE_UID)?
                        != source.sop_instance_uid()
                    || required_string(&referenced[0], tags::REFERENCED_SOP_CLASS_UID)?
                        != source.sop_class_uid()
                {
                    return Err(Error::InvalidInput(
                        "3D ANN Referenced Image Sequence does not identify the open WSI".into(),
                    ));
                }
                references_source_image = true;
                referenced_frame_number = optional_referenced_frame_number(&referenced[0])?;
            }
        }
        other => {
            return Err(Error::Unsupported(format!(
                "ANN coordinate type {other:?} is not recognized"
            )))
        }
    }
    let editable_coordinates = coordinate_type == "2D" && pixel_origin.as_deref() == Some("VOLUME");
    let group_items = sequence_items(&object, tags::ANNOTATION_GROUP_SEQUENCE)?;
    if group_items.is_empty() || group_items.len() > MAX_ANNOTATION_GROUPS {
        return Err(Error::InvalidInput(format!(
            "ANN contains {} groups; supported range is 1..={MAX_ANNOTATION_GROUPS}",
            group_items.len()
        )));
    }
    let mut diagnostics = Vec::new();
    let mut groups = Vec::with_capacity(group_items.len());
    for (index, item) in group_items.iter().enumerate() {
        let coordinate_dimensions =
            if coordinate_type == "3D" && item.get(tags::COMMON_Z_COORDINATE_VALUE).is_none() {
                3
            } else {
                2
            };
        groups.push(read_group(
            item,
            coordinate_dimensions,
            editable_coordinates,
            coordinate_type == "3D",
            index,
            &mut diagnostics,
        )?);
    }
    let document = AnnotationDocument {
        source: source.clone(),
        sop_instance_uid: required_string(&object, tags::SOP_INSTANCE_UID)?,
        series_instance_uid: required_string(&object, tags::SERIES_INSTANCE_UID)?,
        predecessor_sop_instance_uid: None,
        coordinate_type,
        pixel_origin_interpretation: pixel_origin,
        references_source_image,
        referenced_frame_number,
        content_label: required_string(&object, tags::CONTENT_LABEL)?,
        content_description: optional_string(&object, tags::CONTENT_DESCRIPTION)
            .unwrap_or_default(),
        content_creator_name: optional_string(&object, tags::CONTENT_CREATOR_NAME),
        producer: DerivedObjectProducer::read(&object),
        groups,
        diagnostics,
    };
    document.validate()?;
    Ok(document)
}

fn read_group(
    item: &InMemDicomObject,
    coordinate_dimensions: usize,
    editable_coordinates: bool,
    three_dimensional: bool,
    group_index: usize,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<AnnotationGroup> {
    let path = format!("AnnotationGroupSequence[{group_index}]");
    let graphic_type =
        AnnotationGraphicType::from_dicom(&required_string(item, tags::GRAPHIC_TYPE)?)?;
    let mut coordinates = item
        .get(tags::DOUBLE_POINT_COORDINATES_DATA)
        .or_else(|| item.get(tags::POINT_COORDINATES_DATA))
        .ok_or_else(|| Error::InvalidInput("ANN group has no coordinate data".into()))?
        .to_multi_float64()
        .map_err(|error| Error::InvalidInput(format!("invalid ANN coordinates: {error}")))?;
    if coordinates.is_empty()
        || coordinates.len() > MAX_COORDINATE_VALUES
        || coordinate_dimensions == 0
        || coordinates.len() % coordinate_dimensions != 0
    {
        return Err(Error::InvalidInput(
            "ANN group coordinate count is invalid or exceeds the resource limit".into(),
        ));
    }
    if coordinates.iter().any(|value| !value.is_finite()) {
        return Err(Error::InvalidInput(
            "ANN group coordinates must be finite".into(),
        ));
    }
    let mut indices = item
        .get(tags::LONG_PRIMITIVE_POINT_INDEX_LIST)
        .map(|element| element.to_multi_int::<u32>())
        .transpose()
        .map_err(|error| Error::InvalidInput(format!("invalid ANN primitive indices: {error}")))?
        .unwrap_or_default();
    validate_encoded_geometry(
        item,
        graphic_type,
        &coordinates,
        &indices,
        coordinate_dimensions,
    )?;
    let mut coordinate_dimensions = coordinate_dimensions;
    let mut common_z_coordinates = item
        .get(tags::COMMON_Z_COORDINATE_VALUE)
        .map(|element| element.to_multi_float64())
        .transpose()
        .map_err(|error| Error::InvalidInput(format!("invalid common Z values: {error}")))?
        .unwrap_or_default();
    if coordinate_dimensions == 3 {
        let first_z = coordinates[2];
        if coordinates.chunks_exact(3).all(|point| point[2] == first_z) {
            coordinates = coordinates
                .chunks_exact(3)
                .flat_map(|point| [point[0], point[1]])
                .collect();
            for index in &mut indices {
                let point_index = primitive_point_offset(*index, 3)
                    .ok_or_else(|| Error::InvalidInput("invalid 3D ANN primitive index".into()))?;
                *index = u32::try_from(point_index * 2 + 1).map_err(|_| {
                    Error::InvalidInput("normalized ANN primitive index exceeds DICOM range".into())
                })?;
            }
            coordinate_dimensions = 2;
            common_z_coordinates.push(first_z);
            diagnostics.push(InteroperabilityDiagnostic::new(
                "UNIFORM_Z_FACTORED",
                DiagnosticSeverity::Info,
                format!("{path}.DoublePointCoordinatesData"),
                DiagnosticDisposition::Normalized,
                "uniform per-point Z values were factored into Common Z Coordinate Value",
            ));
        }
    }
    let geometry = if editable_coordinates && graphic_type == AnnotationGraphicType::Point {
        AnnotationGeometry::Points(
            coordinates
                .chunks_exact(2)
                .map(|point| Point2::new(point[0], point[1]))
                .collect(),
        )
    } else if editable_coordinates && graphic_type == AnnotationGraphicType::Polygon {
        AnnotationGeometry::Polygons(decode_polygons(&coordinates, &indices)?)
    } else {
        AnnotationGeometry::ReadOnly {
            graphic_type,
            coordinates,
            primitive_point_indices: indices,
            coordinate_dimensions,
        }
    };
    let color = item
        .get(tags::RECOMMENDED_DISPLAY_CIE_LAB_VALUE)
        .and_then(|element| element.to_multi_int::<u16>().ok())
        .filter(|values| values.len() == 3)
        .map_or([0, 0, 0], |values| [values[0], values[1], values[2]]);
    let measurements = item
        .get(tags::MEASUREMENTS_SEQUENCE)
        .and_then(|element| element.items())
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, item)| read_measurement(item, index, &path, diagnostics))
                .collect()
        })
        .transpose()?
        .unwrap_or_default();
    let generation_type = GenerationType::from_dicom(&required_string(
        item,
        tags::ANNOTATION_GROUP_GENERATION_TYPE,
    )?)?;
    let algorithms = read_algorithms(
        item,
        tags::ANNOTATION_GROUP_ALGORITHM_IDENTIFICATION_SEQUENCE,
        &format!("{path}.AnnotationGroupAlgorithmIdentificationSequence"),
        diagnostics,
    )?;
    let applies_to_all_optical_paths =
        read_yes_no(item, tags::ANNOTATION_APPLIES_TO_ALL_OPTICAL_PATHS, true)?;
    let referenced_optical_paths = optional_strings(item, tags::REFERENCED_OPTICAL_PATH_IDENTIFIER);
    if three_dimensional && item.get(tags::ANNOTATION_APPLIES_TO_ALL_Z_PLANES).is_none() {
        return Err(Error::InvalidInput(format!(
            "{path} is missing Annotation Applies to All Z Planes"
        )));
    }
    let has_z_attributes = item.get(tags::ANNOTATION_APPLIES_TO_ALL_Z_PLANES).is_some()
        || item.get(tags::COMMON_Z_COORDINATE_VALUE).is_some();
    if !three_dimensional && has_z_attributes {
        diagnostics.push(InteroperabilityDiagnostic::new(
            "TWO_DIMENSIONAL_Z_APPLICABILITY_WOULD_DROP",
            DiagnosticSeverity::Warning,
            format!("{path}.AnnotationAppliesToAllZPlanes"),
            DiagnosticDisposition::WouldDrop,
            "Z-plane applicability on a 2D ANN is outside the rewrite allowlist",
        ));
    }
    let applies_to_all_z_planes =
        read_yes_no(item, tags::ANNOTATION_APPLIES_TO_ALL_Z_PLANES, true)?;
    AnnotationGroup::from_parts(
        required_string(item, tags::ANNOTATION_GROUP_UID)?,
        required_string(item, tags::ANNOTATION_GROUP_LABEL)?,
        optional_string(item, tags::ANNOTATION_GROUP_DESCRIPTION).unwrap_or_default(),
        FindingSemantics::new(
            generation_type,
            algorithms,
            read_code_at(
                item,
                tags::ANNOTATION_PROPERTY_CATEGORY_CODE_SEQUENCE,
                &format!("{path}.AnnotationPropertyCategoryCodeSequence"),
                diagnostics,
            )?,
            read_code_at(
                item,
                tags::ANNOTATION_PROPERTY_TYPE_CODE_SEQUENCE,
                &format!("{path}.AnnotationPropertyTypeCodeSequence"),
                diagnostics,
            )?,
            read_codes_at(
                item,
                tags::ANNOTATION_PROPERTY_TYPE_MODIFIER_CODE_SEQUENCE,
                &format!("{path}.AnnotationPropertyTypeModifierCodeSequence"),
                diagnostics,
            )?,
            read_codes_at(
                item,
                tags::ANATOMIC_REGION_SEQUENCE,
                &format!("{path}.AnatomicRegionSequence"),
                diagnostics,
            )?,
            read_codes_at(
                item,
                tags::PRIMARY_ANATOMIC_STRUCTURE_SEQUENCE,
                &format!("{path}.PrimaryAnatomicStructureSequence"),
                diagnostics,
            )?,
            color,
        )?,
        applies_to_all_optical_paths,
        referenced_optical_paths,
        applies_to_all_z_planes,
        common_z_coordinates,
        geometry,
        measurements,
    )
}

fn read_measurement(
    item: &InMemDicomObject,
    index: usize,
    group_path: &str,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<AnnotationMeasurement> {
    let values_items = sequence_items(item, tags::MEASUREMENT_VALUES_SEQUENCE)?;
    if values_items.len() != 1 {
        return Err(Error::Unsupported(
            "ANN measurements with multiple value sequence items are not supported".into(),
        ));
    }
    let values = values_items[0]
        .get(tags::FLOATING_POINT_VALUES)
        .ok_or_else(|| Error::InvalidInput("ANN measurement has no values".into()))?
        .to_multi_float64()
        .map_err(|error| Error::InvalidInput(format!("invalid ANN measurement: {error}")))?;
    let indices = values_items[0]
        .get(tags::ANNOTATION_INDEX_LIST)
        .map(|element| element.to_multi_int::<u32>())
        .transpose()
        .map_err(|error| Error::InvalidInput(format!("invalid measurement indices: {error}")))?;
    let path = format!("{group_path}.MeasurementsSequence[{index}]");
    let concept = read_code_at(
        item,
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        &format!("{path}.ConceptNameCodeSequence"),
        diagnostics,
    )?;
    let units = read_code_at(
        item,
        tags::MEASUREMENT_UNITS_CODE_SEQUENCE,
        &format!("{path}.MeasurementUnitsCodeSequence"),
        diagnostics,
    )?;
    match indices {
        Some(indices) => AnnotationMeasurement::for_annotations(concept, units, values, indices),
        None => Ok(AnnotationMeasurement::new(concept, units, values)),
    }
}

fn optional_referenced_frame_number(object: &InMemDicomObject) -> Result<Option<u32>> {
    let Some(element) = object.get(tags::REFERENCED_FRAME_NUMBER) else {
        return Ok(None);
    };
    let values = element.to_multi_int::<u32>().map_err(|error| {
        Error::InvalidInput(format!("invalid Referenced Frame Number: {error}"))
    })?;
    if values.len() != 1 || values[0] == 0 {
        return Err(Error::InvalidInput(
            "ANN must reference exactly one non-zero frame".into(),
        ));
    }
    Ok(Some(values[0]))
}
