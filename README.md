# wsi-dicom-annotations

This README describes the 0.1.2 library API, including the shared metadata
reader. The headless CLI is available from the 0.1.2 source checkout.

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
For 2D ANN in the `VOLUME` pixel-origin interpretation, the writer preserves
the exact referenced source SOP Class and Instance UIDs and copies a valid
source Frame of Reference UID and Container Identifier when present. It omits
either optional identity when the source omits it; these copied attributes aid
viewer association and do not replace the normative 2D image reference.

## Headless annotation CLI

This repository also owns `wsi-annotation-probe`, a standalone command-line package
with no viewer or GUI dependencies:

```console
cargo build -p wsi-annotation-probe --bin annotation_probe --locked
cargo test -p wsi-annotation-probe --locked
```

`target/debug/annotation_probe` supports `inspect`, `roundtrip`, `convert-geojson`,
and `convert-raster`. It preserves the versioned JSON stdout reports and exit codes
used by `wsi-annotation-interop`; diagnostics go to stderr. Newly converted objects
identify the producer as Frames / Annotation Probe and use the CLI package version.
Existing DICOM identities are retained by the inspection and roundtrip paths.

`metadata::open_metadata_object` is the shared bounded, pixel-free metadata reader
for this library and the viewer. Its API documents admission limits and error types.

## Checked editing and derived-object identity

Invariant-bearing ANN collections are edited atomically. Replace point or
polygon geometry with `AnnotationGroup::replace_points` or
`AnnotationGroup::replace_polygons`, and replace document groups with
`AnnotationDocument::replace_group` or `AnnotationDocument::replace_groups`.
An invalid replacement returns an error and leaves the original value intact.

New ANN, SEG, SR, and PM objects use a neutral library producer by default.
Applications should identify themselves explicitly when publishing derived
objects:

```no_run
use wsi_dicom_annotations::{DerivedObjectProducer, Result};

# fn main() -> Result<()> {
let producer = DerivedObjectProducer::new(
    71,
    "Example Pathology",
    "Example Workstation",
    "WORKSTATION-1",
    "3.0",
)?
.with_series_description("Reviewed annotations")?;
// Apply the same value with `document.with_producer(producer)` to ANN, SEG,
// SR, or feature-gated PM documents before writing.
# let _ = producer;
# Ok(())
# }
```

`PathologyDicomDocuments::with_producers` applies distinct caller-owned ANN,
SEG, and SR series metadata to a companion-document set before publication.

SEG-to-ANN conversion is an explicit editing projection. Call
`vectorized_annotations(SegToAnnConversionPolicy::RejectLoss)` to block
nonrepresentable identity or applicability semantics. Use `AllowLoss` only
when the caller will inspect and surface `VectorizedAnnotations::diagnostics`.
The removed `vectorized_annotation_groups` API has no silent compatibility
wrapper.

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
