use std::path::Path;

use dicom_core::value::PrimitiveValue;
use dicom_core::{DataElement, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;

use crate::Result;

use super::{
    RegionReference, SpatialCoordinates, StructuredReportDocument, StructuredReportMeasurement,
    StructuredReportMeasurementGroup, StructuredReportQualitativeEvaluation,
};
use crate::annotations::coded_content::code_item;
use crate::annotations::context::DicomAnnotationContext;
use crate::annotations::derived_object::{
    build_common_object, sop_reference_item, DerivedObjectProducer,
};
use crate::annotations::dicom_dataset::{dicom_now, put_text, sequence};
use crate::annotations::dicom_file::{atomic_write_dicom, ensure_sidecar_destination};
use crate::annotations::dicom_value::format_ds;
use crate::annotations::model::{AlgorithmIdentification, DicomCode};

pub(super) fn write_sr(document: &StructuredReportDocument, path: &Path) -> Result<()> {
    ensure_sidecar_destination(path, document.source.source_path())?;
    let mut object = build_common_object(
        &document.source,
        uids::COMPREHENSIVE3_DSR_STORAGE,
        &document.sop_instance_uid,
        &document.series_instance_uid,
        "SR",
        &document.producer,
    )?;
    for tag in [
        tags::CONTAINER_IDENTIFIER,
        tags::ISSUER_OF_THE_CONTAINER_IDENTIFIER_SEQUENCE,
        tags::CONTAINER_TYPE_CODE_SEQUENCE,
        tags::SPECIMEN_DESCRIPTION_SEQUENCE,
    ] {
        object.remove_element(tag);
    }
    let (date, time) = dicom_now();
    for (tag, vr, value) in [
        (tags::CONTENT_DATE, VR::DA, date.as_str()),
        (tags::CONTENT_TIME, VR::TM, time.as_str()),
        (
            tags::COMPLETION_FLAG,
            VR::CS,
            document.completion_flag.as_str(),
        ),
        (
            tags::VERIFICATION_FLAG,
            VR::CS,
            document.verification_flag.as_str(),
        ),
        (
            tags::PRELIMINARY_FLAG,
            VR::CS,
            document.preliminary_flag.as_str(),
        ),
    ] {
        put_text(&mut object, tag, vr, value);
    }
    put_text(&mut object, tags::CONTENT_QUALIFICATION, VR::CS, "SERVICE");
    object.put(sequence(
        tags::PERFORMED_PROCEDURE_CODE_SEQUENCE,
        document.procedures_reported.iter().map(code_item).collect(),
    ));
    object.put(sequence(
        tags::REFERENCED_PERFORMED_PROCEDURE_STEP_SEQUENCE,
        Vec::new(),
    ));
    add_evidence(&mut object, document);
    write_root_content(&mut object, document)?;
    atomic_write_dicom(
        path,
        object,
        uids::COMPREHENSIVE3_DSR_STORAGE,
        &document.sop_instance_uid,
    )
}

fn write_root_content(
    object: &mut InMemDicomObject,
    document: &StructuredReportDocument,
) -> Result<()> {
    put_text(object, tags::VALUE_TYPE, VR::CS, "CONTAINER");
    put_text(object, tags::CONTINUITY_OF_CONTENT, VR::CS, "SEPARATE");
    object.put(sequence(
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        vec![code_item(&document.report_title)],
    ));
    object.put(sequence(
        tags::CONTENT_TEMPLATE_SEQUENCE,
        vec![template_item("1500")],
    ));
    let mut content = language_items()?;
    content.extend(device_observer_items(
        &document.device_observer_uid,
        &document.producer,
    )?);
    for procedure in &document.procedures_reported {
        content.push(code_content_item(
            "HAS CONCEPT MOD",
            &concept("121058", "DCM", "Procedure reported")?,
            procedure,
        ));
    }
    let groups = document
        .groups
        .iter()
        .map(|group| write_measurement_group(group, &document.source))
        .collect::<Result<Vec<_>>>()?;
    content.push(container_item(
        "CONTAINS",
        &concept("126010", "DCM", "Imaging Measurements")?,
        None,
        groups,
    ));
    object.put(sequence(tags::CONTENT_SEQUENCE, content));
    Ok(())
}

fn write_measurement_group(
    group: &StructuredReportMeasurementGroup,
    source: &DicomAnnotationContext,
) -> Result<InMemDicomObject> {
    let mut content = vec![
        text_content_item(
            "HAS OBS CONTEXT",
            &concept("112039", "DCM", "Tracking Identifier")?,
            &group.tracking_id,
        ),
        uid_content_item(
            "HAS OBS CONTEXT",
            &concept("112040", "DCM", "Tracking Unique Identifier")?,
            &group.tracking_uid,
        ),
        code_content_item(
            "CONTAINS",
            &concept("276214006", "SCT", "Finding category")?,
            &group.finding_category,
        ),
        code_content_item(
            "CONTAINS",
            &concept("121071", "DCM", "Finding")?,
            &group.finding_type,
        ),
    ];
    for site in &group.finding_sites {
        content.push(code_content_item(
            "CONTAINS",
            &concept("363698007", "SCT", "Finding Site")?,
            site,
        ));
    }
    for algorithm in &group.algorithms {
        content.extend(algorithm_items(algorithm)?);
    }
    match &group.reference {
        RegionReference::Coordinates(coordinates) => {
            content.push(coordinates_item("CONTAINS", coordinates, true)?);
        }
        RegionReference::Segmentation(reference) => {
            content.push(segmentation_item(reference)?);
            content.push(source_image_item(
                source.sop_class_uid(),
                source.sop_instance_uid(),
            )?);
        }
        RegionReference::MeasurementCoordinates => {}
    }
    content.extend(
        group
            .measurements
            .iter()
            .map(measurement_item)
            .collect::<Result<Vec<_>>>()?,
    );
    content.extend(group.qualitative_evaluations.iter().map(qualitative_item));
    Ok(container_item(
        "CONTAINS",
        &concept("125007", "DCM", "Measurement Group")?,
        Some(group.template_id),
        content,
    ))
}

fn measurement_item(measurement: &StructuredReportMeasurement) -> Result<InMemDicomObject> {
    let mut measured_value = InMemDicomObject::new_empty();
    put_text(
        &mut measured_value,
        tags::NUMERIC_VALUE,
        VR::DS,
        &format_ds(measurement.value),
    );
    measured_value.put(sequence(
        tags::MEASUREMENT_UNITS_CODE_SEQUENCE,
        vec![code_item(&measurement.unit)],
    ));
    let mut item = base_content_item("CONTAINS", "NUM", &measurement.concept);
    item.put(sequence(
        tags::MEASURED_VALUE_SEQUENCE,
        vec![measured_value],
    ));
    if !measurement.coordinates.is_empty() {
        item.put(sequence(
            tags::CONTENT_SEQUENCE,
            measurement
                .coordinates
                .iter()
                .map(|coordinates| coordinates_item("INFERRED FROM", coordinates, false))
                .collect::<Result<Vec<_>>>()?,
        ));
    }
    Ok(item)
}

fn qualitative_item(evaluation: &StructuredReportQualitativeEvaluation) -> InMemDicomObject {
    code_content_item("CONTAINS", &evaluation.concept, &evaluation.value)
}

fn coordinates_item(
    relationship: &str,
    coordinates: &SpatialCoordinates,
    image_region: bool,
) -> Result<InMemDicomObject> {
    let name = if image_region {
        concept("111030", "DCM", "Image Region")?
    } else {
        concept("260753009", "SCT", "Source")?
    };
    let mut item = base_content_item(relationship, "SCOORD3D", &name);
    put_text(
        &mut item,
        tags::GRAPHIC_TYPE,
        VR::CS,
        coordinates.graphic.dicom_value(),
    );
    let mut graphic_data = coordinates
        .points
        .iter()
        .flat_map(|point| [point.x as f32, point.y as f32, point.z as f32])
        .collect::<Vec<_>>();
    if coordinates.graphic == super::CoordinateGraphic::Polygon
        && coordinates.points.first() != coordinates.points.last()
    {
        let first = coordinates.points[0];
        graphic_data.extend([first.x as f32, first.y as f32, first.z as f32]);
    }
    item.put(DataElement::new(
        tags::GRAPHIC_DATA,
        VR::FL,
        PrimitiveValue::F32(graphic_data.into()),
    ));
    put_text(
        &mut item,
        tags::REFERENCED_FRAME_OF_REFERENCE_UID,
        VR::UI,
        &coordinates.frame_of_reference_uid,
    );
    Ok(item)
}

fn segmentation_item(reference: &super::SegmentationReference) -> Result<InMemDicomObject> {
    let mut reference_item =
        sop_reference_item(uids::SEGMENTATION_STORAGE, &reference.sop_instance_uid);
    // TID 1410 requires both selectors for a tiled SEG: the frames identify
    // the tiles in one plane and the segment identifies their shared ROI.
    put_text(
        &mut reference_item,
        tags::REFERENCED_FRAME_NUMBER,
        VR::IS,
        &reference
            .frame_numbers
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("\\"),
    );
    reference_item.put(DataElement::new(
        tags::REFERENCED_SEGMENT_NUMBER,
        VR::US,
        PrimitiveValue::from(reference.segment_number),
    ));
    let mut item = base_content_item(
        "CONTAINS",
        "IMAGE",
        &concept("121214", "DCM", "Referenced Segmentation Frame")?,
    );
    item.put(sequence(
        tags::REFERENCED_SOP_SEQUENCE,
        vec![reference_item],
    ));
    Ok(item)
}

fn source_image_item(sop_class_uid: &str, sop_instance_uid: &str) -> Result<InMemDicomObject> {
    let mut item = base_content_item(
        "CONTAINS",
        "IMAGE",
        &concept("121233", "DCM", "Source Image for Segmentation")?,
    );
    item.put(sequence(
        tags::REFERENCED_SOP_SEQUENCE,
        vec![sop_reference_item(sop_class_uid, sop_instance_uid)],
    ));
    Ok(item)
}

fn algorithm_items(algorithm: &AlgorithmIdentification) -> Result<Vec<InMemDicomObject>> {
    let mut items = vec![
        text_content_item(
            "HAS CONCEPT MOD",
            &concept("111001", "DCM", "Algorithm Name")?,
            algorithm.name(),
        ),
        text_content_item(
            "HAS CONCEPT MOD",
            &concept("111003", "DCM", "Algorithm Version")?,
            algorithm.version(),
        ),
        code_content_item(
            "HAS CONCEPT MOD",
            &concept("111000", "DCM", "Algorithm Family")?,
            algorithm.family(),
        ),
    ];
    if let Some(name_code) = algorithm.name_code() {
        items.push(code_content_item(
            "HAS CONCEPT MOD",
            &concept("111001", "DCM", "Algorithm Name")?,
            name_code,
        ));
    }
    if let Some(parameters) = algorithm.parameters() {
        items.push(text_content_item(
            "HAS CONCEPT MOD",
            &concept("111002", "DCM", "Algorithm Parameters")?,
            parameters,
        ));
    }
    if let Some(source) = algorithm.source() {
        items.push(text_content_item(
            "HAS CONCEPT MOD",
            &concept("122405", "DCM", "Algorithm Manufacturer")?,
            source,
        ));
    }
    Ok(items)
}

fn language_items() -> Result<Vec<InMemDicomObject>> {
    Ok(vec![code_content_item(
        "HAS CONCEPT MOD",
        &concept("121049", "DCM", "Language of Content Item and Descendants")?,
        &concept("en-US", "RFC5646", "English (United States)")?,
    )])
}

fn device_observer_items(
    device_uid: &str,
    producer: &DerivedObjectProducer,
) -> Result<Vec<InMemDicomObject>> {
    Ok(vec![
        code_content_item(
            "HAS OBS CONTEXT",
            &concept("121005", "DCM", "Observer Type")?,
            &concept("121007", "DCM", "Device")?,
        ),
        uid_content_item(
            "HAS OBS CONTEXT",
            &concept("121012", "DCM", "Device Observer UID")?,
            device_uid,
        ),
        text_content_item(
            "HAS OBS CONTEXT",
            &concept("121013", "DCM", "Device Observer Name")?,
            producer.manufacturer_model_name(),
        ),
        text_content_item(
            "HAS OBS CONTEXT",
            &concept("121014", "DCM", "Device Observer Manufacturer")?,
            producer.manufacturer(),
        ),
        text_content_item(
            "HAS OBS CONTEXT",
            &concept("121015", "DCM", "Device Observer Model Name")?,
            producer.manufacturer_model_name(),
        ),
        text_content_item(
            "HAS OBS CONTEXT",
            &concept("121016", "DCM", "Device Observer Serial Number")?,
            producer.device_serial_number(),
        ),
    ])
}

fn add_evidence(object: &mut InMemDicomObject, document: &StructuredReportDocument) {
    let mut series = vec![series_evidence(
        document.source.series_instance_uid(),
        document.source.sop_class_uid(),
        document.source.sop_instance_uid(),
    )];
    for reference in document
        .groups
        .iter()
        .filter_map(|group| match &group.reference {
            RegionReference::Segmentation(reference) => Some(reference),
            _ => None,
        })
    {
        if !series.iter().any(|item| {
            item.get(tags::SERIES_INSTANCE_UID)
                .and_then(|element| element.to_str().ok())
                .is_some_and(|uid| uid.trim() == reference.series_instance_uid)
        }) {
            series.push(series_evidence(
                &reference.series_instance_uid,
                uids::SEGMENTATION_STORAGE,
                &reference.sop_instance_uid,
            ));
        }
    }
    let mut study = InMemDicomObject::new_empty();
    put_text(
        &mut study,
        tags::STUDY_INSTANCE_UID,
        VR::UI,
        document.source.study_instance_uid(),
    );
    study.put(sequence(tags::REFERENCED_SERIES_SEQUENCE, series));
    object.put(sequence(
        tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE,
        vec![study],
    ));
}

fn series_evidence(
    series_instance_uid: &str,
    sop_class_uid: &str,
    sop_instance_uid: &str,
) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    put_text(
        &mut item,
        tags::SERIES_INSTANCE_UID,
        VR::UI,
        series_instance_uid,
    );
    item.put(sequence(
        tags::REFERENCED_SOP_SEQUENCE,
        vec![sop_reference_item(sop_class_uid, sop_instance_uid)],
    ));
    item
}

fn container_item(
    relationship: &str,
    name: &DicomCode,
    template_id: Option<&str>,
    children: Vec<InMemDicomObject>,
) -> InMemDicomObject {
    let mut item = base_content_item(relationship, "CONTAINER", name);
    put_text(&mut item, tags::CONTINUITY_OF_CONTENT, VR::CS, "SEPARATE");
    if let Some(template_id) = template_id {
        item.put(sequence(
            tags::CONTENT_TEMPLATE_SEQUENCE,
            vec![template_item(template_id)],
        ));
    }
    if !children.is_empty() {
        item.put(sequence(tags::CONTENT_SEQUENCE, children));
    }
    item
}

fn code_content_item(relationship: &str, name: &DicomCode, value: &DicomCode) -> InMemDicomObject {
    let mut item = base_content_item(relationship, "CODE", name);
    item.put(sequence(
        tags::CONCEPT_CODE_SEQUENCE,
        vec![code_item(value)],
    ));
    item
}

fn text_content_item(relationship: &str, name: &DicomCode, value: &str) -> InMemDicomObject {
    let mut item = base_content_item(relationship, "TEXT", name);
    put_text(&mut item, tags::TEXT_VALUE, VR::UT, value);
    item
}

fn uid_content_item(relationship: &str, name: &DicomCode, value: &str) -> InMemDicomObject {
    let mut item = base_content_item(relationship, "UIDREF", name);
    put_text(&mut item, tags::UID, VR::UI, value);
    item
}

fn base_content_item(relationship: &str, value_type: &str, name: &DicomCode) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    put_text(&mut item, tags::RELATIONSHIP_TYPE, VR::CS, relationship);
    put_text(&mut item, tags::VALUE_TYPE, VR::CS, value_type);
    item.put(sequence(
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        vec![code_item(name)],
    ));
    item
}

fn template_item(identifier: &str) -> InMemDicomObject {
    let mut item = InMemDicomObject::new_empty();
    put_text(&mut item, tags::MAPPING_RESOURCE, VR::CS, "DCMR");
    put_text(&mut item, tags::TEMPLATE_IDENTIFIER, VR::CS, identifier);
    item
}

fn concept(value: &str, scheme: &str, meaning: &str) -> Result<DicomCode> {
    DicomCode::new(value, scheme, meaning)
}

#[cfg(test)]
mod tests {
    use dicom_dictionary_std::tags;

    use super::{coordinates_item, segmentation_item};
    use crate::annotations::model::Point3;
    use crate::annotations::sr::{CoordinateGraphic, SpatialCoordinates};

    #[test]
    fn scoord3d_polygon_repeats_its_first_point_at_the_end() {
        let coordinates = SpatialCoordinates {
            graphic: CoordinateGraphic::Polygon,
            points: vec![
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(5.0, 1.0, 0.0),
                Point3::new(5.0, 5.0, 0.0),
                Point3::new(1.0, 5.0, 0.0),
            ],
            frame_of_reference_uid: "2.25.1".to_owned(),
        };

        let item = coordinates_item("CONTAINS", &coordinates, true).unwrap();
        let values = item
            .element(tags::GRAPHIC_DATA)
            .unwrap()
            .to_multi_float32()
            .unwrap();

        assert_eq!(values.len(), 15);
        assert_eq!(&values[..3], &values[values.len() - 3..]);
    }

    #[test]
    fn tiled_segmentation_reference_selects_frames_and_the_segment() {
        let item = segmentation_item(&crate::annotations::sr::SegmentationReference {
            sop_instance_uid: "2.25.2".to_owned(),
            series_instance_uid: "2.25.3".to_owned(),
            segment_number: 4,
            frame_numbers: vec![2, 5],
        })
        .unwrap();
        let references = item
            .element(tags::REFERENCED_SOP_SEQUENCE)
            .unwrap()
            .items()
            .unwrap();

        assert_eq!(
            references[0]
                .element(tags::REFERENCED_FRAME_NUMBER)
                .unwrap()
                .to_str()
                .unwrap(),
            "2\\5"
        );
        assert_eq!(
            references[0]
                .element(tags::REFERENCED_SEGMENT_NUMBER)
                .unwrap()
                .to_int::<u16>()
                .unwrap(),
            4
        );
    }
}
