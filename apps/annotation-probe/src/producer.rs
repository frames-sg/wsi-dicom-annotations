use wsi_dicom_annotations::{DerivedObjectProducer, Result};

/// Identify files created by this headless application independently of the viewer.
pub(crate) fn annotation_probe_producer(
    series_number: i32,
    series_description: &str,
) -> Result<DerivedObjectProducer> {
    DerivedObjectProducer::new(
        series_number,
        "Frames",
        "Annotation Probe",
        "not-applicable",
        env!("CARGO_PKG_VERSION"),
    )?
    .with_series_description(series_description)
}
