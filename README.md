# wsi-dicom-annotations

`wsi-dicom-annotations` provides UI-independent Rust models, readers, writers,
and conversion boundaries for DICOM Whole Slide Microscopy derived objects:

- Microscopy Bulk Simple Annotations (ANN)
- Segmentation (SEG)
- Comprehensive 3D Structured Reports (SR)
- Parametric Maps (PM)
- explicitly profiled pathology and QuPath GeoJSON conversion

The default `parametric-map` feature includes profiled TIFF, NPY, Zarr, and
tiled-raster PM conversion. Applications that only need ANN/SEG/SR and GeoJSON
conversion can disable default features to avoid those raster dependencies.

Applications provide an existing DICOM VL Whole Slide Microscopy instance as
the source context. The crate preserves Study and Frame of Reference identity
while assigning new Series and SOP Instance identities to derived objects.

## GeoJSON to DICOM

QuPath classification labels are never guessed as clinical terminology. A
bounded mapping profile must explicitly connect every label, measurement, and
qualitative value to its DICOM coded meaning.

```no_run
use std::fs;
use wsi_dicom_annotations::{
    DicomAnnotationContext, PathologyAnnotationSet, PathologyCoordinateSpace,
};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let source = DicomAnnotationContext::from_source("level-0-wsi.dcm")?;
let geojson = fs::read("qupath.geojson")?;
let mapping = fs::read("pathology-mapping-v1.json")?;
let annotations = PathologyAnnotationSet::from_json(
    &geojson,
    &mapping,
    &source,
    &source,
    PathologyCoordinateSpace::Level0Pixels,
    false,
)?;
annotations.to_ann()?.write_ann("annotations.dcm")?;
# Ok(())
# }
```

Polygon holes and mask-style regions belong in SEG rather than ANN. Numeric
measurements and coded evaluations can be retained in SR, including SR that
references the exact SEG generated from the same profiled feature set.

## Scope

This crate owns DICOM interchange, terminology primitives, annotation schemes,
tracking identity, and profiled external-data conversion. It deliberately does
not contain a viewer, renderer, editing history, workspace persistence, PACS or
DICOMweb transport, clinical workflow claims, or collaborative review state.

`dicom-viewer` consumes this library for import and export. The separate
`wsi-annotation-interop` project remains an independent file-level conformance
harness and does not share the production implementation.

## Dependency security note

Local Zarr support currently resolves `lru 0.16.4` through `zarrs 0.23.14`.
RUSTSEC-2026-0253 requires a key with a panicking `Drop` implementation and
unwind recovery; this dependency path uses only `StoreKey(String)` and
`ChunkIndices(Vec<u64>)`, whose drops cannot panic. The repository records this
reachability decision explicitly, and `dicom-viewer` additionally applies the
upstream source-level backport. Remove the exception when a compatible `zarrs`
release permits `lru >= 0.18.2`.
