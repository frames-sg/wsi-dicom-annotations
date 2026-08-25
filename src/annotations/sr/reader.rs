use std::path::Path;

use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;

use crate::{Error, Result};

use super::{
    CoordinateGraphic, RegionReference, SegmentationReference, SpatialCoordinates,
    StructuredReportDocument, StructuredReportMeasurement, StructuredReportMeasurementGroup,
    StructuredReportQualitativeEvaluation,
};
use crate::annotations::coded_content::read_code_at;
use crate::annotations::context::DicomAnnotationContext;
use crate::annotations::derived_object::DerivedObjectProducer;
use crate::annotations::dicom_dataset::{optional_string, required_string, sequence_items};
use crate::annotations::dicom_file::enforce_file_limit;
use crate::annotations::model::{
    AlgorithmIdentification, DicomCode, InteroperabilityDiagnostic, Point3,
};
use crate::annotations::seg::SegmentationDocument;

const MAX_SR_FILE_BYTES: u64 = 512 * 1024 * 1024;

pub(super) fn read_sr(
    path: &Path,
    source: &DicomAnnotationContext,
    segmentation: Option<&SegmentationDocument>,
) -> Result<StructuredReportDocument> {
    enforce_file_limit(path, MAX_SR_FILE_BYTES, "SR")?;
    let object = dicom_object::open_file(path).map_err(|error| Error::DicomRead {
        path: path.to_path_buf(),
        source: Box::new(error),
    })?;
    if object.meta().media_storage_sop_class_uid() != uids::COMPREHENSIVE3_DSR_STORAGE {
        return Err(Error::Unsupported(format!(
            "{} is not a Comprehensive 3D SR instance",
            path.display()
        )));
    }
    if required_string(&object, tags::STUDY_INSTANCE_UID)? != source.study_instance_uid() {
        return Err(Error::InvalidInput(
            "SR Study Instance UID does not match the source WSI".into(),
        ));
    }
    validate_document_flags(&object)?;
    validate_source_evidence(&object, source)?;
    let mut diagnostics = Vec::<InteroperabilityDiagnostic>::new();
    let report_title = read_code_at(
        &object,
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        "ConceptNameCodeSequence",
        &mut diagnostics,
    )?;
    if template_identifier(&object)?.as_deref() != Some("1500") {
        return Err(Error::InvalidInput(
            "SR root does not declare TID 1500".into(),
        ));
    }
    let content = content_items(&object)?;
    let procedures_reported = content
        .iter()
        .filter(|item| has_concept(item, "121058"))
        .map(|item| {
            read_code_at(
                item,
                tags::CONCEPT_CODE_SEQUENCE,
                "ProcedureReported.ConceptCodeSequence",
                &mut diagnostics,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let imaging = content
        .iter()
        .find(|item| has_concept(item, "126010"))
        .ok_or_else(|| Error::InvalidInput("TID 1500 has no Imaging Measurements".into()))?;
    let groups = content_items(imaging)?
        .iter()
        .filter(|item| has_concept(item, "125007"))
        .map(|item| read_group(item, source, segmentation, &mut diagnostics))
        .collect::<Result<Vec<_>>>()?;
    if groups.is_empty() {
        return Err(Error::InvalidInput(
            "TID 1500 Imaging Measurements has no measurement groups".into(),
        ));
    }
    let device_observer_uid = find_text_value(content, "121012", tags::UID)?;
    Ok(StructuredReportDocument {
        source: source.clone(),
        sop_instance_uid: required_string(&object, tags::SOP_INSTANCE_UID)?,
        series_instance_uid: required_string(&object, tags::SERIES_INSTANCE_UID)?,
        device_observer_uid,
        producer: DerivedObjectProducer::read(&object),
        report_title,
        procedures_reported,
        groups,
        completion_flag: required_string(&object, tags::COMPLETION_FLAG)?,
        verification_flag: required_string(&object, tags::VERIFICATION_FLAG)?,
        preliminary_flag: required_string(&object, tags::PRELIMINARY_FLAG)?,
    })
}

fn validate_document_flags(object: &InMemDicomObject) -> Result<()> {
    for (tag, expected, name) in [
        (tags::COMPLETION_FLAG, "COMPLETE", "Completion Flag"),
        (tags::VERIFICATION_FLAG, "UNVERIFIED", "Verification Flag"),
        (tags::PRELIMINARY_FLAG, "PRELIMINARY", "Preliminary Flag"),
    ] {
        let actual = required_string(object, tag)?;
        if actual != expected {
            return Err(Error::InvalidInput(format!(
                "SR {name} is {actual:?}; expected {expected:?} for research conversion output"
            )));
        }
    }
    Ok(())
}

fn read_group(
    item: &InMemDicomObject,
    source: &DicomAnnotationContext,
    segmentation: Option<&SegmentationDocument>,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<StructuredReportMeasurementGroup> {
    let template_id = match template_identifier(item)?.as_deref() {
        Some("1410") => "1410",
        Some("1501") => "1501",
        other => {
            return Err(Error::InvalidInput(format!(
                "measurement group has unexpected template identifier {other:?}"
            )));
        }
    };
    let content = content_items(item)?;
    let tracking_id = find_text_value(content, "112039", tags::TEXT_VALUE)?;
    let tracking_uid = find_text_value(content, "112040", tags::UID)?;
    let finding_category = find_code_value(content, "276214006", diagnostics)?;
    let finding_type = find_code_value(content, "121071", diagnostics)?;
    let finding_sites = content
        .iter()
        .filter(|child| has_concept(child, "363698007"))
        .map(|child| {
            read_code_at(
                child,
                tags::CONCEPT_CODE_SEQUENCE,
                "FindingSite.ConceptCodeSequence",
                diagnostics,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let algorithms = read_algorithms(content, diagnostics)?;
    let measurements = content
        .iter()
        .filter(|child| optional_string(child, tags::VALUE_TYPE).as_deref() == Some("NUM"))
        .map(|child| read_measurement(child, source, diagnostics))
        .collect::<Result<Vec<_>>>()?;
    let reference = if let Some(coordinates) = content.iter().find(|child| {
        optional_string(child, tags::VALUE_TYPE).as_deref() == Some("SCOORD3D")
            && has_concept(child, "111030")
    }) {
        RegionReference::Coordinates(read_coordinates(coordinates, source)?)
    } else if let Some(reference) = content.iter().find(|child| {
        optional_string(child, tags::VALUE_TYPE).as_deref() == Some("IMAGE")
            && has_concept(child, "121214")
    }) {
        RegionReference::Segmentation(read_segmentation_reference(
            reference,
            &tracking_id,
            &tracking_uid,
            segmentation,
        )?)
    } else if measurements
        .iter()
        .any(|measurement| !measurement.coordinates.is_empty())
    {
        RegionReference::MeasurementCoordinates
    } else {
        return Err(Error::InvalidInput(
            "measurement group has no lossless region reference".into(),
        ));
    };
    let qualitative_evaluations = content
        .iter()
        .filter(|child| {
            optional_string(child, tags::VALUE_TYPE).as_deref() == Some("CODE")
                && !matches!(
                    concept_value(child).as_deref(),
                    Ok("276214006" | "121071" | "363698007" | "111000" | "111001")
                )
        })
        .map(|child| {
            Ok(StructuredReportQualitativeEvaluation {
                concept: read_code_at(
                    child,
                    tags::CONCEPT_NAME_CODE_SEQUENCE,
                    "QualitativeEvaluation.ConceptNameCodeSequence",
                    diagnostics,
                )?,
                value: read_code_at(
                    child,
                    tags::CONCEPT_CODE_SEQUENCE,
                    "QualitativeEvaluation.ConceptCodeSequence",
                    diagnostics,
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(StructuredReportMeasurementGroup {
        template_id,
        tracking_id,
        tracking_uid,
        finding_category,
        finding_type,
        algorithms,
        finding_sites,
        reference,
        measurements,
        qualitative_evaluations,
    })
}

fn read_measurement(
    item: &InMemDicomObject,
    source: &DicomAnnotationContext,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<StructuredReportMeasurement> {
    let measured = sequence_items(item, tags::MEASURED_VALUE_SEQUENCE)?;
    if measured.len() != 1 {
        return Err(Error::InvalidInput(
            "SR NUM item must contain exactly one Measured Value item".into(),
        ));
    }
    let value = measured[0]
        .get(tags::NUMERIC_VALUE)
        .ok_or_else(|| Error::InvalidInput("SR measurement has no Numeric Value".into()))?
        .to_float64()
        .map_err(|error| Error::InvalidInput(format!("SR Numeric Value is invalid: {error}")))?;
    if !value.is_finite() {
        return Err(Error::InvalidInput(
            "SR Numeric Value must be finite".into(),
        ));
    }
    let coordinates = optional_content_items(item)
        .iter()
        .filter(|child| optional_string(child, tags::VALUE_TYPE).as_deref() == Some("SCOORD3D"))
        .map(|child| read_coordinates(child, source))
        .collect::<Result<Vec<_>>>()?;
    Ok(StructuredReportMeasurement {
        concept: read_code_at(
            item,
            tags::CONCEPT_NAME_CODE_SEQUENCE,
            "Measurement.ConceptNameCodeSequence",
            diagnostics,
        )?,
        unit: read_code_at(
            &measured[0],
            tags::MEASUREMENT_UNITS_CODE_SEQUENCE,
            "Measurement.MeasurementUnitsCodeSequence",
            diagnostics,
        )?,
        value,
        coordinates,
    })
}

fn read_coordinates(
    item: &InMemDicomObject,
    source: &DicomAnnotationContext,
) -> Result<SpatialCoordinates> {
    let graphic = match required_string(item, tags::GRAPHIC_TYPE)?.as_str() {
        "POINT" => CoordinateGraphic::Point,
        "MULTIPOINT" => CoordinateGraphic::Multipoint,
        "POLYLINE" => CoordinateGraphic::Polyline,
        "POLYGON" => CoordinateGraphic::Polygon,
        other => {
            return Err(Error::Unsupported(format!(
                "SR SCOORD3D graphic type {other:?} is not supported"
            )));
        }
    };
    let values = item
        .get(tags::GRAPHIC_DATA)
        .ok_or_else(|| Error::InvalidInput("SR SCOORD3D has no Graphic Data".into()))?
        .to_multi_float64()
        .map_err(|error| Error::InvalidInput(format!("invalid SR Graphic Data: {error}")))?;
    if values.is_empty() || values.len() % 3 != 0 || values.iter().any(|value| !value.is_finite()) {
        return Err(Error::InvalidInput(
            "SR SCOORD3D Graphic Data must contain finite triplets".into(),
        ));
    }
    let frame_of_reference_uid = required_string(item, tags::REFERENCED_FRAME_OF_REFERENCE_UID)?;
    if Some(frame_of_reference_uid.as_str()) != source.frame_of_reference_uid() {
        return Err(Error::InvalidInput(
            "SR SCOORD3D Frame of Reference UID does not match the source WSI".into(),
        ));
    }
    Ok(SpatialCoordinates {
        graphic,
        points: values
            .chunks_exact(3)
            .map(|point| Point3::new(point[0], point[1], point[2]))
            .collect(),
        frame_of_reference_uid,
    })
}

fn read_segmentation_reference(
    item: &InMemDicomObject,
    tracking_id: &str,
    tracking_uid: &str,
    segmentation: Option<&SegmentationDocument>,
) -> Result<SegmentationReference> {
    let references = sequence_items(item, tags::REFERENCED_SOP_SEQUENCE)?;
    if references.len() != 1
        || required_string(&references[0], tags::REFERENCED_SOP_CLASS_UID)?
            != uids::SEGMENTATION_STORAGE
    {
        return Err(Error::InvalidInput(
            "SR segmentation reference is not a single binary SEG SOP reference".into(),
        ));
    }
    let sop_instance_uid = required_string(&references[0], tags::REFERENCED_SOP_INSTANCE_UID)?;
    let segment_number = references[0]
        .get(tags::REFERENCED_SEGMENT_NUMBER)
        .ok_or_else(|| {
            Error::InvalidInput("SR segmentation reference has no Segment Number".into())
        })?
        .to_int::<u16>()
        .map_err(|error| {
            Error::InvalidInput(format!("invalid referenced Segment Number: {error}"))
        })?;
    let frame_numbers = references[0]
        .get(tags::REFERENCED_FRAME_NUMBER)
        .ok_or_else(|| {
            Error::InvalidInput("SR tiled segmentation reference has no Frame Numbers".into())
        })?
        .to_multi_int::<u32>()
        .map_err(|error| {
            Error::InvalidInput(format!("invalid referenced Frame Numbers: {error}"))
        })?;
    let segmentation = segmentation.ok_or_else(|| {
        Error::InvalidInput(
            "SR references SEG geometry, but no SEG document was supplied for verification".into(),
        )
    })?;
    if segmentation.sop_instance_uid() != sop_instance_uid {
        return Err(Error::InvalidInput(
            "SR referenced SEG SOP Instance UID does not match the supplied SEG".into(),
        ));
    }
    let segment = segmentation
        .segment_for_number(segment_number)
        .ok_or_else(|| {
            Error::InvalidInput("SR references a SEG segment that does not exist".into())
        })?;
    if segment.tracking_id() != Some(tracking_id) || segment.tracking_uid() != Some(tracking_uid) {
        return Err(Error::InvalidInput(
            "SR and SEG tracking identifiers do not agree".into(),
        ));
    }
    let expected_frames = segmentation
        .rasterized_frames()?
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame.segment_number() == segment_number)
        .map(|(index, _)| u32::try_from(index + 1))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::InvalidInput("SEG frame number exceeds u32".into()))?;
    if frame_numbers != expected_frames {
        return Err(Error::InvalidInput(
            "SR referenced SEG frames do not exactly cover the tracked segment".into(),
        ));
    }
    Ok(SegmentationReference {
        sop_instance_uid,
        series_instance_uid: segmentation.series_instance_uid().to_string(),
        segment_number,
        frame_numbers,
    })
}

fn read_algorithms(
    content: &[InMemDicomObject],
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<Vec<AlgorithmIdentification>> {
    let Some(name_item) = find_item(content, "111001", Some("TEXT")) else {
        return Ok(Vec::new());
    };
    let version_item = find_item(content, "111003", Some("TEXT"))
        .ok_or_else(|| Error::InvalidInput("SR Algorithm Name has no Algorithm Version".into()))?;
    let family_item = find_item(content, "111000", Some("CODE")).ok_or_else(|| {
        Error::InvalidInput("SR algorithm identification has no Algorithm Family".into())
    })?;
    let mut algorithm = AlgorithmIdentification::new(
        read_code_at(
            family_item,
            tags::CONCEPT_CODE_SEQUENCE,
            "AlgorithmFamily.ConceptCodeSequence",
            diagnostics,
        )?,
        required_string(name_item, tags::TEXT_VALUE)?,
        required_string(version_item, tags::TEXT_VALUE)?,
    )?;
    if let Some(name_code) = find_item(content, "111001", Some("CODE")) {
        algorithm = algorithm.with_name_code(read_code_at(
            name_code,
            tags::CONCEPT_CODE_SEQUENCE,
            "AlgorithmName.ConceptCodeSequence",
            diagnostics,
        )?);
    }
    if let Some(parameters) = find_item(content, "111002", Some("TEXT")) {
        algorithm = algorithm.with_parameters(required_string(parameters, tags::TEXT_VALUE)?)?;
    }
    if let Some(source) = find_item(content, "122405", Some("TEXT")) {
        algorithm = algorithm.with_source(required_string(source, tags::TEXT_VALUE)?)?;
    }
    Ok(vec![algorithm])
}

fn validate_source_evidence(
    object: &InMemDicomObject,
    source: &DicomAnnotationContext,
) -> Result<()> {
    let studies = sequence_items(object, tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE)?;
    let found = studies.iter().any(|study| {
        optional_string(study, tags::STUDY_INSTANCE_UID).as_deref()
            == Some(source.study_instance_uid())
            && optional_content_items_at(study, tags::REFERENCED_SERIES_SEQUENCE)
                .iter()
                .any(|series| {
                    optional_string(series, tags::SERIES_INSTANCE_UID).as_deref()
                        == Some(source.series_instance_uid())
                        && optional_content_items_at(series, tags::REFERENCED_SOP_SEQUENCE)
                            .iter()
                            .any(|reference| {
                                optional_string(reference, tags::REFERENCED_SOP_INSTANCE_UID)
                                    .as_deref()
                                    == Some(source.sop_instance_uid())
                                    && optional_string(reference, tags::REFERENCED_SOP_CLASS_UID)
                                        .as_deref()
                                        == Some(source.sop_class_uid())
                            })
                })
    });
    if !found {
        return Err(Error::InvalidInput(
            "SR evidence does not reference the source WSI".into(),
        ));
    }
    Ok(())
}

fn find_code_value(
    content: &[InMemDicomObject],
    concept: &str,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<DicomCode> {
    let item = find_item(content, concept, Some("CODE")).ok_or_else(|| {
        Error::InvalidInput(format!(
            "SR measurement group has no coded concept {concept}"
        ))
    })?;
    read_code_at(
        item,
        tags::CONCEPT_CODE_SEQUENCE,
        "ConceptCodeSequence",
        diagnostics,
    )
}

fn find_text_value(
    content: &[InMemDicomObject],
    concept: &str,
    value_tag: dicom_core::Tag,
) -> Result<String> {
    let item = content
        .iter()
        .find(|item| has_concept(item, concept))
        .ok_or_else(|| {
            Error::InvalidInput(format!("SR content has no required concept {concept}"))
        })?;
    required_string(item, value_tag)
}

fn find_item<'a>(
    content: &'a [InMemDicomObject],
    concept: &str,
    value_type: Option<&str>,
) -> Option<&'a InMemDicomObject> {
    content.iter().find(|item| {
        has_concept(item, concept)
            && value_type.is_none_or(|value_type| {
                optional_string(item, tags::VALUE_TYPE).as_deref() == Some(value_type)
            })
    })
}

fn concept_value(item: &InMemDicomObject) -> Result<String> {
    let names = sequence_items(item, tags::CONCEPT_NAME_CODE_SEQUENCE)?;
    if names.len() != 1 {
        return Err(Error::InvalidInput(
            "SR content item must have exactly one Concept Name Code".into(),
        ));
    }
    for tag in [
        tags::CODE_VALUE,
        tags::LONG_CODE_VALUE,
        tags::URN_CODE_VALUE,
    ] {
        if let Some(value) = optional_string(&names[0], tag) {
            return Ok(value);
        }
    }
    Err(Error::InvalidInput(
        "SR Concept Name Code has no code value".into(),
    ))
}

fn has_concept(item: &InMemDicomObject, expected: &str) -> bool {
    concept_value(item).is_ok_and(|actual| actual == expected)
}

fn template_identifier(item: &InMemDicomObject) -> Result<Option<String>> {
    let templates = optional_content_items_at(item, tags::CONTENT_TEMPLATE_SEQUENCE);
    if templates.len() > 1 {
        return Err(Error::InvalidInput(
            "SR content item has multiple Content Template items".into(),
        ));
    }
    Ok(templates
        .first()
        .and_then(|template| optional_string(template, tags::TEMPLATE_IDENTIFIER)))
}

fn content_items(item: &InMemDicomObject) -> Result<&[InMemDicomObject]> {
    item.get(tags::CONTENT_SEQUENCE)
        .and_then(|element| element.items())
        .ok_or_else(|| Error::InvalidInput("SR content item has no Content Sequence".into()))
}

fn optional_content_items(item: &InMemDicomObject) -> &[InMemDicomObject] {
    optional_content_items_at(item, tags::CONTENT_SEQUENCE)
}

fn optional_content_items_at(item: &InMemDicomObject, tag: dicom_core::Tag) -> &[InMemDicomObject] {
    item.get(tag)
        .and_then(|element| element.items())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
