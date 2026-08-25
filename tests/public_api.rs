use std::path::Path;

#[cfg(feature = "parametric-map")]
use std::io::Write;

use dicom_core::value::{DataSetSequence, PrimitiveValue, Value};
use dicom_core::{DataElement, Length, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};
use wsi_dicom_annotations::{
    AnnotationDocument, AnnotationGroup, DerivedObjectProducer, DicomAnnotationContext, DicomCode,
    LinearMeasurementSpec, MeasurementReportSemantics, Point2, SegToAnnConversionPolicy,
    SegmentationDocument, SegmentationSegment, StructuredReportDocument, TrackingIdentity,
};

#[cfg(feature = "parametric-map")]
use wsi_dicom_annotations::{ParametricMapDocument, RasterChannelSelection, RasterProfile};

fn code(value: &str, meaning: &str) -> DicomCode {
    DicomCode::new(value, "99DOWNSTREAM", meaning).unwrap()
}

#[test]
fn downstream_ann_seg_and_sr_workflow_uses_only_public_apis() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    write_source_wsi(&source_path);
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    assert_eq!(source.container_identifier(), Some("PUBLIC-API-SLIDE"));
    let producer = DerivedObjectProducer::new(
        71,
        "Downstream Pathology",
        "Downstream Workstation",
        "DOWNSTREAM-1",
        "3.0",
    )
    .unwrap();
    let category = code("MORPH", "Morphology");
    let property = code("TUMOR", "Tumor");

    let mut group = AnnotationGroup::points(
        "Cells",
        category.clone(),
        property.clone(),
        [1, 2, 3],
        vec![Point2::new(1.0, 1.0)],
    )
    .unwrap();
    group.replace_points(vec![Point2::new(2.0, 2.0)]).unwrap();
    let mut ann = AnnotationDocument::new(source.clone(), vec![group])
        .unwrap()
        .with_producer(producer.clone());
    ann.replace_group(
        0,
        AnnotationGroup::points(
            "Edited cells",
            category.clone(),
            property.clone(),
            [1, 2, 3],
            vec![Point2::new(3.0, 3.0)],
        )
        .unwrap(),
    )
    .unwrap();
    let ann_path = directory.path().join("annotations.dcm");
    ann.write_ann(&ann_path).unwrap();
    let imported_ann = AnnotationDocument::read_ann(&ann_path, &source).unwrap();
    assert_eq!(imported_ann.groups()[0].label(), "Edited cells");
    assert_eq!(
        imported_ann.producer().manufacturer(),
        "Downstream Pathology"
    );
    let raw_ann = dicom_object::open_file(&ann_path).unwrap();
    assert_eq!(
        raw_ann
            .element(tags::FRAME_OF_REFERENCE_UID)
            .unwrap()
            .to_str()
            .unwrap(),
        "2.25.7003"
    );
    assert_eq!(
        raw_ann
            .element(tags::CONTAINER_IDENTIFIER)
            .unwrap()
            .to_str()
            .unwrap(),
        "PUBLIC-API-SLIDE"
    );

    let seg_path = directory.path().join("segmentation.dcm");
    SegmentationDocument::binary(
        source.clone(),
        vec![SegmentationSegment::new(
            "Tumor",
            category.clone(),
            property.clone(),
            [1, 2, 3],
            vec![vec![
                Point2::new(0.0, 0.0),
                Point2::new(4.0, 0.0),
                Point2::new(4.0, 4.0),
                Point2::new(0.0, 4.0),
            ]],
            Vec::new(),
        )
        .unwrap()],
    )
    .unwrap()
    .with_producer(producer.clone())
    .write_seg(&seg_path)
    .unwrap();
    let imported_seg = SegmentationDocument::read_seg(&seg_path, &source).unwrap();
    let vectorized = imported_seg
        .vectorized_annotations(SegToAnnConversionPolicy::AllowLoss)
        .unwrap();
    assert!(!vectorized.groups().is_empty());
    assert!(vectorized
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code() == "SEGMENT_NUMBER_NOT_REPRESENTABLE"));

    let sr_path = directory.path().join("measurements.dcm");
    let measurement = LinearMeasurementSpec::new(
        TrackingIdentity::new("length-1", "2.25.7101").unwrap(),
        category,
        property,
        Point2::new(1.0, 1.0),
        Point2::new(4.0, 1.0),
    )
    .unwrap();
    StructuredReportDocument::from_linear_measurements(
        source.clone(),
        &MeasurementReportSemantics::pathology_v1(),
        &[measurement],
    )
    .unwrap()
    .with_producer(producer)
    .write_sr(&sr_path)
    .unwrap();
    let imported_sr = StructuredReportDocument::read_sr(&sr_path, &source, None).unwrap();
    assert_eq!(imported_sr.groups().len(), 1);
    assert_eq!(imported_sr.producer().series_number(), "71");
}

#[cfg(feature = "parametric-map")]
#[test]
fn downstream_parametric_map_workflow_is_feature_gated_and_public() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    let raster_path = directory.path().join("probability.npy");
    let output_path = directory.path().join("probability.dcm");
    write_source_wsi(&source_path);
    write_npy_f32(&raster_path, &[2, 2], &[0.1, 0.2, 0.3, 0.4]);
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let profile = RasterProfile::from_json(PM_PROFILE.as_bytes()).unwrap();
    let document = ParametricMapDocument::open(
        source.clone(),
        source,
        profile,
        &raster_path,
        RasterChannelSelection::Auto,
    )
    .unwrap()
    .with_producer(
        DerivedObjectProducer::new(72, "Downstream", "Downstream PM", "PM-1", "3.0").unwrap(),
    );
    let instance = document.write_single(&output_path, 2_000_000_000).unwrap();
    assert_eq!(instance.frame_count(), 1);
    assert!(output_path.is_file());
}

fn write_source_wsi(path: &Path) {
    const SOP_UID: &str = "1.2.826.0.1.3680043.10.777.101";
    let mut origin = InMemDicomObject::new_empty();
    origin.put(DataElement::new(
        tags::X_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        "0",
    ));
    origin.put(DataElement::new(
        tags::Y_OFFSET_IN_SLIDE_COORDINATE_SYSTEM,
        VR::DS,
        "0",
    ));
    let mut object = InMemDicomObject::new_empty();
    for element in [
        DataElement::new(
            tags::SOP_CLASS_UID,
            VR::UI,
            uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE,
        ),
        DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, SOP_UID),
        DataElement::new(tags::STUDY_INSTANCE_UID, VR::UI, "2.25.7001"),
        DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, "2.25.7002"),
        DataElement::new(tags::FRAME_OF_REFERENCE_UID, VR::UI, "2.25.7003"),
        DataElement::new(tags::CONTAINER_IDENTIFIER, VR::LO, "PUBLIC-API-SLIDE"),
        DataElement::new(tags::PATIENT_NAME, VR::PN, "Downstream^Test"),
        DataElement::new(tags::PATIENT_ID, VR::LO, "PUBLIC-API"),
        DataElement::new(tags::STUDY_DATE, VR::DA, "20260820"),
        DataElement::new(tags::STUDY_TIME, VR::TM, "120000"),
        DataElement::new(tags::STUDY_ID, VR::SH, "PUBLIC"),
        DataElement::new(tags::ACCESSION_NUMBER, VR::SH, ""),
        DataElement::new(tags::DIMENSION_ORGANIZATION_TYPE, VR::CS, "TILED_FULL"),
        DataElement::new(tags::NUMBER_OF_FRAMES, VR::IS, "1"),
        DataElement::new(
            tags::NUMBER_OF_OPTICAL_PATHS,
            VR::UL,
            PrimitiveValue::from(1_u32),
        ),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_FOCAL_PLANES,
            VR::UL,
            PrimitiveValue::from(1_u32),
        ),
        DataElement::new(tags::ROWS, VR::US, PrimitiveValue::from(8_u16)),
        DataElement::new(tags::COLUMNS, VR::US, PrimitiveValue::from(8_u16)),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_ROWS,
            VR::UL,
            PrimitiveValue::from(8_u32),
        ),
        DataElement::new(
            tags::TOTAL_PIXEL_MATRIX_COLUMNS,
            VR::UL,
            PrimitiveValue::from(8_u32),
        ),
        DataElement::new(tags::IMAGE_ORIENTATION_SLIDE, VR::DS, "1\\0\\0\\0\\1\\0"),
        DataElement::new(tags::PIXEL_SPACING, VR::DS, "0.00025\\0.00025"),
        DataElement::new(tags::SLICE_THICKNESS, VR::DS, "0.001"),
    ] {
        object.put(element);
    }
    object.put(DataElement::new(
        tags::TOTAL_PIXEL_MATRIX_ORIGIN_SEQUENCE,
        VR::SQ,
        Value::from(DataSetSequence::new(vec![origin], Length::UNDEFINED)),
    ));
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        PrimitiveValue::from(vec![0_u8; 192]),
    ));
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .media_storage_sop_class_uid(uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE)
                .media_storage_sop_instance_uid(SOP_UID)
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .unwrap()
        .write_to_file(path)
        .unwrap();
}

#[cfg(feature = "parametric-map")]
fn write_npy_f32(path: &Path, shape: &[usize], values: &[f32]) {
    let shape = shape
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let mut header = format!("{{'descr': '<f4', 'fortran_order': False, 'shape': ({shape},), }}");
    while (10 + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(b"\x93NUMPY\x01\x00").unwrap();
    file.write_all(&(header.len() as u16).to_le_bytes())
        .unwrap();
    file.write_all(header.as_bytes()).unwrap();
    for value in values {
        file.write_all(&value.to_le_bytes()).unwrap();
    }
}

#[cfg(feature = "parametric-map")]
const PM_PROFILE: &str = r#"
{
  "schema_version": 1,
  "input_format": "npy",
  "dtype": "float32",
  "axes": ["y", "x"],
  "grid_origin": {"x": 0.0, "y": 0.0},
  "sample_spacing": {"x": 1.0, "y": 1.0},
  "coordinate_space": "level0-pixels",
  "channels": [{
    "name": "tumor",
    "quantity": {"code_value":"TUMOR","coding_scheme_designator":"99WSI","code_meaning":"Tumor probability"},
    "unit": {"code_value":"1","coding_scheme_designator":"UCUM","code_meaning":"no units"}
  }],
  "algorithm": {
    "family": {"code_value":"123110","coding_scheme_designator":"DCM","code_meaning":"Artificial Intelligence"},
    "name": "Downstream model",
    "version": "1.0"
  }
}
"#;
