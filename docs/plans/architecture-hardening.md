# Architecture hardening plan

This document is the durable source of truth for the architecture-hardening task. Update it before and after every phase.

## Repository and baseline

- Repository: `frames-sg/wsi-dicom-annotations`
- Initial Git SHA: `0fa5c954f08eeb91352e2cf160a2e36efe5f9657`
- Branch: `main`
- Initial tree: clean (`git status --short` produced no output)
- Toolchain: `rustc 1.96.0 (ac68faa20 2026-05-25)`, Cargo `1.96.0 (30a34c682 2026-05-25)`
- Rust edition/MSRV: edition 2021; `rust-version = "1.96"`; `rust-toolchain.toml` pins 1.96 with rustfmt, Clippy, rust-src, macOS ARM64 and Linux x86_64 targets
- Features: default = `parametric-map`; optional `parametric-map` enables `npyz`, `tiff`, and `zarrs`
- Baseline format/tests/Clippy/build/docs: all required commands passed; default/all features ran 114 unit tests plus 1 public integration test and 1 doctest, while no-default ran 73 unit tests plus the integration test and doctest
- External tools: `cargo-audit 0.22.1`, `dciodvfy`, `dcmdump`, and `dcm2json` are installed; the ignored external-validator test passed when run explicitly
- Pre-existing LSP server: Rust server at `/Users/user/Bench/frames/wsi-rs`; it is not owned by this task and must not be stopped

## Pre-change architecture map

- `src/annotations/model/`: core annotation, geometry, code, tracking, algorithm, and diagnostic domain types.
- `src/annotations/ann.rs` plus `ann/{read,write,geometry_codec}.rs`: ANN document model and DICOM codec.
- `src/annotations/seg.rs` plus `seg/{document,raster,read,write}.rs`: SEG document model, raster/run handling, vectorization, and DICOM codec.
- `src/annotations/sr/`: SR reader/writer.
- `src/annotations/parametric_map/`: feature-gated PM source, planning, verification, and writing.
- `src/annotations/pathology_geojson/`: pathology GeoJSON documents, mapping, and ANN/SR conversion.
- `src/annotations/scheme.rs`: scheme model, JSON, validation, built-ins, semantic keys/digests, and color work (to verify).
- `src/annotations/{publication,sidecar,metadata}.rs`: staged publication, sidecar inspection, and metadata boundaries.
- Tests are primarily in `src/annotation_tests.rs` with additional focused unit modules and `tests/public_api.rs`.

## Findings

| ID | Status | Current evidence | Planned files | Regression tests | Acceptance criteria | Cross-repository implications |
|---|---|---|---|---|---|---|
| WDA-001 invariant-safe ANN mutation | complete | Removed `groups_mut` and both geometry `&mut Vec` accessors. `AnnotationGroup::validate` is the single intrinsic path; `AnnotationDocument::validate` calls it plus bounds, UID uniqueness, coordinate dimensions, and resource budgets. Writers already call document validation before object creation/publication. | `src/annotations/{ann.rs,ann_validation_tests.rs,context.rs}`, `src/annotations/model/{annotation.rs,annotation_tests.rs}` | Atomic topology/finiteness/measurement edits, indexed removal, bounds, duplicate UIDs, invalid internal writer state, valid round trip | Passed: no raw APIs remain; failed edits preserve equality; writer leaves no destination; valid checked edits round trip | No searched downstream consumer used removed methods; checked API migration is direct |
| WDA-002 caller-owned producer provenance | complete | Public validated `DerivedObjectProducer` owns series number/description and equipment identity. ANN, SEG, SR, and PM expose `with_producer`/`producer`; imported ANN/SEG/SR retain DICOM equipment on rewrite. Neutral defaults identify the library project, never Frames/DICOM Viewer. SR observer and PM contributing equipment use the same identity. | `derived_object.rs`, ANN/SEG/SR/PM model/read/write modules, pathology document bundle, `provenance_tests.rs`, `parametric_map_tests.rs`, exports | Caller fields in ANN/SEG/SR/PM, pathology companion documents, SR imported rewrite, invalid metadata, neutral default | Passed: caller metadata is present; imported rewrite stable; generic model is `wsi-dicom-annotations`; series numbers explicit | Complete: `dicom-viewer` applies one explicit Frames/DICOM Viewer producer factory to new ANN/SEG/SR/PM and GeoJSON companion documents |
| WDA-003 SEG-to-ANN semantic preservation and loss reporting | complete | Bare `vectorized_annotation_groups` was replaced by `vectorized_annotations(policy) -> VectorizedAnnotations`. Description, generation/algorithms, modifiers, anatomy, category/type/color survive; valid Tracking UID becomes ANN Group UID. Tracking ID, source Segment Number, empty segments, invalid generation, invalid/duplicate tracking UID, and carried import losses have exact typed diagnostics and block by default. | `src/annotations/seg/{document.rs,vectorization_tests.rs,run_tests.rs,performance_tests.rs}`, `seg.rs`, exports, fractional regression call | Full semantic segment, exact codes, RejectLoss/AllowLoss, imported applicability diagnostic, empty segment, invalid generation, exact canonical rectangles | Passed: nothing disappears without preservation/diagnostic; blocking is explicit; canonical raster identity remains independently tested | Complete: `dicom-viewer` uses `AllowLoss` for editable projection, returns typed diagnostics from the import boundary, and surfaces blocking diagnostic codes in UI status |
| WDA-004 shared row-run scanning | complete | `scan_nonzero_ranges` plus `RunBudget` is the only contiguous-row scanner. Binary/fractional builders share checked offsets and limits; vectorization consumes sorted/merged `binary_runs` and creates exact pixel-edge rectangles. | `src/annotations/seg/{raster.rs,document.rs,run_tests.rs,performance_tests.rs}`, `seg.rs` | Empty/sparse/dense scanner, shared limit, overflow, deterministic ordering, canonical rectangle identity, non-sequential segments | Passed: one scanner; exact fractional payload regression; vectorization is canonical and deterministic | None |
| WDA-005 shared finding-semantic core | complete | Crate-private `FindingSemantics` owns generation/algorithms, category/type/modifiers, anatomy, and display CIELab with one validation path. ANN group, SEG segment, and pathology label mapping embed it; pathology and SEG projection transfer it as one value. Labels/descriptions, geometry, measurements, tracking, and applicability remain format-local. | `src/annotations/model/{finding.rs,finding_tests.rs,annotation.rs}`, `model.rs`, SEG/read/vectorization, pathology mapping/conversion/preview/SR conversion | One shared value populates ANN/SEG identically; generation mismatch rejected once; extended ANN/SEG round trip; all pathology tests | Passed: parallel fields/validation and long conversion copy sequences removed; format ownership preserved | Private representation; no external API migration |
| WDA-006 scheme module cohesion | complete | Replaced 830-line `scheme.rs` with `scheme/{model,json,builtins,concept_key,color}.rs`; 15-line `mod.rs` deliberately re-exports the unchanged public API. Content/concept digest namespaces and byte ordering are unchanged. | `src/annotations/scheme/` (6 files); removed `src/annotations/scheme.rs` | Strict JSON/duplicates/unknowns, exact content digest and concept key, contextual built-ins/inventory, color round trip | Passed: DTOs, built-ins, and color math have separate owners; stable paths/digests/JSON retained | No external migration |
| WDA-007 test fixture and test-module boundaries | complete | `annotation_tests.rs` is now an 86-line module root. Behavioral tests are split into ANN round-trip, SEG round-trip, sidecar boundary, coordinate projection, and external-validator modules. Reusable native ANN/SEG/WSI writers live under crate-private `test_support/annotation_dicom.rs`; all other test modules use that owner. | `src/annotation_tests.rs`, `src/annotation_tests/*.rs`, `src/test_support/*.rs`, fixture consumer imports, `src/lib.rs` | Entire existing suite retained under focused module names | Passed: reusable fixture construction separated; external validator remains independent; nothing exported publicly | None |
| WDA-008 sidecar preflight deduplication | complete | `read_sidecar_header` now owns size-limit, preamble, seek, and file-meta parsing. Strict classification propagates its errors; discovery deliberately maps them to a skipped candidate, preserving the two public error policies. | `src/annotations/sidecar.rs` | Existing classification, malformed/non-DICOM, discovery, reference, and bounded-payload tests | Passed: one shared header parser and unchanged strict-versus-best-effort behavior | None |
| WDA-009 explicit module dependencies/imports | complete | SEG document/raster/read/layout/write production modules now import their exact std, DICOM, crate, and sibling dependencies. Parent imports only the types it owns. Remaining `use super::*` matches are confined to three focused test modules. | Stable SEG production modules only | Entire all-feature/no-default suites plus both Clippy matrices | Passed: no production glob imports and no behavior change | None |
| WDA-010 CI and feature matrix | complete | `.github/workflows/ci.yml` uses minimal `contents: read` permission and pins the sole action (`actions/checkout` v4.4.0) to commit `11d5960a326750d5838078e36cf38b85af677262`. It installs Rust 1.96 explicitly and covers default/no-default/all tests, both Clippy modes, all-feature/no-default release builds, docs with warnings denied, audit, and release benchmark compilation. Optional DICOM tools run only via a boolean `workflow_dispatch` input. | `.github/workflows/ci.yml`, `README.md` | YAML parse, immutable-reference search, local command matrix, release benchmark compilation, external validator | Passed locally; audit stale check requires both exact `lru 0.16.4` reachability and the live RustSec advisory file | GitHub-hosted Ubuntu 24.04 supplies optional validators through `dcmtk` and `dicom3tools`; actual hosted workflow execution awaits push/PR authorization |
| WDA-011 measured SEG performance/allocation cleanup | complete | Internal normalization uses `Cow` to borrow imported binary frames, eliminating one payload clone; vectorization builds one `HashMap<u16, usize>`, reserves polygon vectors from run counts, and reuses canonical runs. Repeatable ignored release benchmark checks outputs without CI timing thresholds. | `src/annotations/seg/{document.rs,performance_tests.rs}` | 256 mixed sparse/dense frames, 16 non-sequential segments, binary/fractional/vector workloads | Passed with measured evidence; canonical vectorization tradeoff documented | No production dependency or brittle CI threshold added |
| WDA-012 meaningful public API integration tests | complete | Replaced type-name smoke test with an external-crate workflow: construct/edit/write/read ANN; construct/write/read SEG; vectorize with explicit loss diagnostics; construct/write/read SR; supply producer identity; feature-gated PM open/write. It builds its own DICOM/NPY fixtures and cannot access internal modules. | `tests/public_api.rs` | All-feature downstream test has ANN/SEG/SR/PM; no-default has ANN/SEG/SR | Passed in both feature modes with only public APIs | Encodes required viewer migration to typed SEG vectorization and explicit producer identity |
| WDA-013 versioned digest/contract migration policy | complete | `mask_digest` still uses byte-identical `dicom-viewer-seg-runs-v1`; the stable digest regression test passes after provenance changes. Provenance was deliberately decoupled from this externally consumed raster contract. | `src/annotations/seg/document.rs` unchanged; plan decision and existing digest regression test | `segmentation_row_runs_merge_tile_boundaries_and_have_a_stable_digest` | Passed: no namespace/schema change and exact legacy digest remains stable | No sibling update required; `wsi-annotation-interop` v1 implementation remains compatible |

## Ordered checkpoints

1. Phase 0 orientation and baseline. Dependency: none.
2. Phase 1 WDA-001 ANN invariants. Dependency: Phase 0; blocks all lower-priority refactors.
3. Phase 2 WDA-002/WDA-013 provenance and compatibility. Dependency: green Phase 1 and consumer trace.
4. Phase 3 WDA-004/WDA-011 canonical SEG scanning and measured low-risk cleanup. Dependency: stable provenance contracts.
5. Phase 4 WDA-003 vectorization semantics/diagnostics. Dependency: canonical run extraction.
6. Phase 5 WDA-005 shared finding semantics. Dependency: final vectorization semantic inventory.
7. Phase 6 WDA-006 scheme cohesion. Dependency: semantic-core decisions.
8. Phase 7 WDA-007/WDA-012 test/fixture boundaries and external public workflows. Dependency: public APIs stabilized.
9. Phase 8 WDA-008/WDA-009 narrow boundary cleanup. Dependency: major architecture phases complete.
10. Phase 9 WDA-010/WDA-011 CI, security, feature, and benchmark gates. Dependency: implementation matrix stable.
11. Phase 10 deletion-oriented review and complete final validation. Dependency: all prior phases.

## Decision log

- 2026-08-19: Treat every audit finding as unverified until confirmed against current HEAD; obsolete findings require concrete evidence rather than speculative edits.
- 2026-08-19: Preserve the clean initial tree and use only the requested repository. Sibling repositories may be searched read-only only to trace public consumers/contract coupling.
- 2026-08-19: No dependencies added. Any future dependency requires the full requested review entry before editing manifests.
- 2026-08-19: No digest namespace, JSON shape, or serialized schema will change incidentally; compatibility will be traced and versioned first.
- 2026-08-20: Preserve `dicom-viewer-seg-runs-v1` exactly for compatibility. It is an external contract independently consumed by `wsi-annotation-interop`; neutral provenance does not require renaming a versioned digest domain.
- 2026-08-20: Remove the three raw ANN mutation methods immediately (no compatibility deprecation) because semantic and text searches found no consumers. Use focused atomic replacement APIs rather than an editor/draft abstraction: `AnnotationGroup::{replace_points,replace_polygons}` and `AnnotationDocument::{replace_group,replace_groups}`.
- 2026-08-20: Keep existing document constructors source-compatible but change their implicit producer to a truthful neutral library identity. Applications needing branded provenance use `with_producer(DerivedObjectProducer)`. This avoids a giant options object while making series number and all equipment fields caller-owned.
- 2026-08-20: Make canonical sorted/merged binary runs the vectorization input even though the benchmark shows extra sort/merge time. This removes a third semantic scanner, unifies limits/overflow, and reduces rectangle count while retaining exact raster identity. Keep public `rasterized_frames()` owned for compatibility; internal normalization borrows imported frames through `Cow`.
- 2026-08-20: Remove the silent bare-group SEG projection API instead of retaining a compatibility wrapper that could hide diagnostics. `SegToAnnConversionPolicy::RejectLoss` is the safe default choice at call sites; `AllowLoss` returns the same typed diagnostics. Reuse a valid Tracking UID as the ANN Group UID, but diagnose Tracking ID and source Segment Number because ANN has no equivalent fields.
- 2026-08-20: Keep `FindingSemantics` crate-private and intentionally exclude labels/descriptions, tracking, applicability, geometry, and measurements. The shared value follows only fields with identical ANN/SEG/pathology meaning and validation; no `AnnotationLike` trait or public abstraction is introduced.
- 2026-08-20: Split scheme code by concrete dependency direction rather than line count: model depends on concept-key/color/builtins, JSON adapts private model fields to wire DTOs, builtins constructs model values, and `mod.rs` only declares/re-exports. Preserve both existing `frames-*` v1 digest namespaces byte-for-byte.
- 2026-08-20: Keep realistic DICOM fixture writers in one crate-private `test_support::annotation_dicom` owner, while splitting behavior by format/boundary. A 720-line ANN round-trip module remains cohesive and is preferable to scattering tightly related DICOM cases solely for a line target.
- 2026-08-20: Share only sidecar file-header parsing. Classification remains strict and returns malformed-file errors, while directory discovery remains best effort and skips malformed candidates; source-reference dataset traversal is intentionally not folded into the header helper.
- 2026-08-20: Replace production SEG glob imports only at stable module boundaries. Test modules retain `use super::*` because they deliberately exercise private parent behavior and the glob does not obscure a production dependency surface.
- 2026-08-20: Keep CI free of cache and setup actions. Pin the only action to an immutable official checkout commit and install the repository's Rust 1.96 toolchain directly with rustup. The security job pins cargo-audit 0.22.1 and rejects the configured exception when either its exact dependency or advisory disappears.
- 2026-08-20: Keep external `dciodvfy`/`dcmdump`/`dcm2json` validation opt-in through `workflow_dispatch`; Ubuntu 24.04 package availability was verified before using `dicom3tools` and `dcmtk`.
- 2026-08-21: Apply the accepted ponytail review without changing behavior: retain one private `AnnotationGroup::from_parts` constructor taking `FindingSemantics`, remove an unobservable rollback from a consumed builder, make the invariant 64-byte producer text limit explicit, and remove the test-only configurability from `RunBudget`'s operation label. This removed 72 net source lines from the reviewed state without public API changes.
- 2026-08-21: Add `PathologyDicomDocuments::with_producers` because the companion-document builder was the one creation surface that could not yet accept caller identity. The viewer now supplies distinct ANN/SEG/SR series metadata through that API, uses the same identity factory for PM, and surfaces typed SEG projection-loss codes rather than discarding them.

## Validation ledger

| Phase | Command | Result | Notes |
|---|---|---|---|
| 0 | `git rev-parse HEAD` | pass | `0fa5c954f08eeb91352e2cf160a2e36efe5f9657` |
| 0 | `git branch --show-current` | pass | `main` |
| 0 | `git status --short` | pass | clean before this plan file |
| 0 | `rustc -Vv` | pass | Rust 1.96.0, aarch64-apple-darwin |
| 0 | `cargo -V` | pass | Cargo 1.96.0 |
| 0 | `cargo metadata --no-deps --format-version 1` | pass | Single-package workspace; feature and dependency inventory recorded |
| 0 | `cargo fmt --all -- --check` | pass | No baseline formatting differences |
| 0 | `cargo test --locked` | pass | 114 unit passed, 1 external-validator test ignored, 1 integration and 1 doctest passed |
| 0 | `cargo test --locked --no-default-features` | pass | 73 unit passed, 1 external-validator test ignored, 1 integration and 1 doctest passed |
| 0 | `cargo test --locked --all-features` | pass | Same result as default features |
| 0 | `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass | No warnings |
| 0 | `cargo clippy --all-targets --no-default-features --locked -- -D warnings` | pass | No warnings |
| 0 | `cargo build --release --locked` | pass | Release build completed |
| 0 | `cargo build --release --locked --no-default-features` | pass | Release build completed |
| 0 | `cargo doc --no-deps --all-features --locked` | pass | Documentation generated |
| 0 | `cargo audit` | pass | 2 configured allowed unmaintained warnings (`encoding`, `paste`); configured RUSTSEC-2026-0253 exception; `lru 0.16.4` remains reachable through `zarrs 0.23.14` |
| 0 | `cargo test --locked annotation_tests::exported_ann_and_seg_pass_external_dicom_validation -- --ignored --nocapture` | pass | Local dciodvfy/dcmdump/dcm2json validation succeeded |
| 1 | `cargo test --locked checked_geometry_replacement checked_document_edits valid_checked_document_edit invalid_internal_document --no-run` | failed (command usage) | Cargo accepts one test filter; rerun with one filter |
| 1 | `cargo test --locked checked_geometry_replacement --no-run` before implementation | failed as expected | Nine missing-method errors proved new tests were red for the intended checked APIs |
| 1 | `cargo test --locked checked_geometry_replacement -- --nocapture` | pass | 2 focused group mutation tests passed |
| 1 | `cargo test --locked annotations::ann::validation_tests -- --nocapture` | pass | 3 focused document/writer tests passed |
| 1 | `cargo test --locked` | pass | 119 unit passed, 1 external validator ignored, integration and doctest passed |
| 1 | `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass | No warnings |
| 1 | `rg -n --glob '*.rs' 'groups_mut|point_annotations_mut|polygon_annotations_mut' src tests` | pass | No matches |
| 1 | `git diff --check` | pass | No whitespace errors |
| 2 | `cargo test --locked provenance_tests --no-run` before implementation | failed as expected | Missing public producer type and three `with_producer` methods |
| 2 | `cargo test --locked provenance_tests -- --nocapture` | pass | 3 producer creation/rewrite/default/validation tests passed |
| 2 | `cargo test --locked float32_parametric_map_streams_required_metadata_and_exact_pixels -- --nocapture` | pass | PM caller producer fields passed |
| 2 | `cargo test --locked --all-features` | pass | 122 unit passed, 1 external validator ignored, integration and doctest passed |
| 2 | `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass | No warnings |
| 2 | `cargo test --locked annotation_tests::exported_ann_and_seg_pass_external_dicom_validation -- --ignored --nocapture` | pass | dciodvfy/dcmdump/dcm2json passed with neutral defaults |
| 2 | `cargo test --locked segmentation_row_runs_merge_tile_boundaries_and_have_a_stable_digest -- --nocapture` | pass | Exact legacy v1 digest remained stable |
| 2 | `rg -n --glob '*.rs' 'DICOM Viewer|dicom-viewer-rust|SeriesEquipmentMetadata|series_equipment|"Frames"' src` | pass | Matches only synthetic source fixture and negative assertion; no production hard-code remains |
| 2 | `git diff --check` | pass | No whitespace errors |
| 3 | `cargo test --release --locked seg_normalization_benchmark -- --ignored --nocapture` before refactor | pass | 256 frames/16 segments: binary 20x 24.305708 ms, fractional 20x 42.720333 ms, vector 5x 10.453083 ms; checksums 581120/581120/147200 |
| 3 | `cargo test --locked shared_row_scanner --no-run` before implementation | failed as expected | Missing shared scanner and budget symbols |
| 3 | `cargo test --locked annotations::seg::run_tests -- --nocapture` | pass | 4 scanner/order/overflow/canonical vector tests passed |
| 3 | focused legacy binary/fractional regression tests | pass | Stable digest, tile merging, and exact fractional values passed |
| 3 | post-refactor release benchmark run 1 | pass | binary 22.728750 ms; fractional 40.991750 ms; vector 12.631667 ms; output checksums stable except canonical vector rectangle count 145280 |
| 3 | post-refactor release benchmark runs 2/3 after constant-time map | pass | binary 17.934917/18.493791 ms; fractional 39.573542/38.693833 ms; vector 13.711416/13.894708 ms; checksums 581120/581120/145280 |
| 3 | `cargo test --locked --all-features` | pass | 126 unit passed; external validator and performance benchmark ignored; integration and doctest passed |
| 3 | `cargo test --locked --no-default-features` | pass | 85 unit passed; 2 ignored; integration and doctest passed |
| 3 | first combined no-default test/Clippy command | partial failure | Tests passed; all-feature Clippy found `single_range_in_vec_init` in new test, then fixed without an allow |
| 3 | both required Clippy feature commands after fix | pass | No warnings |
| 3 | scanner/search and `git diff --check` | pass | One production row loop exists inside `scan_nonzero_ranges`; no whitespace errors |
| 4 | `cargo test --locked semantic_vectorization --no-run` before implementation | failed as expected | Missing typed result, policy, and method |
| 4 | `cargo test --locked semantic_vectorization -- --nocapture` | pass | 3 full-semantics/loss/import-diagnostic tests passed |
| 4 | `cargo clippy --all-targets --all-features --locked -- -D warnings` | pass | No warnings |
| 4 | `cargo test --locked --all-features` | pass | 129 unit passed, 2 ignored, integration and doctest passed |
| 4 | `cargo test --locked --no-default-features` | pass | 88 unit passed, 2 ignored, integration and doctest passed |
| 4 | `cargo clippy --all-targets --no-default-features --locked -- -D warnings` | pass | No warnings |
| 4 | API consumer search and `git diff --check` | pass | Old API remains only in the read-only sibling viewer consumer; repository uses typed API; no whitespace errors |
| 5 | `cargo test --locked shared_finding --no-run` before implementation | failed as expected | Missing `finding` module established red state |
| 5 | `cargo test --locked annotations::model::finding::tests -- --nocapture` | pass | 2 equivalence/validation tests passed |
| 5 | extended semantic ANN/SEG round-trip test | pass | Modifiers, anatomy, generation, and algorithms preserved |
| 5 | `cargo test --locked pathology_geojson_tests -- --nocapture` | pass | All 19 pathology conversion/mapping tests passed |
| 5 | `cargo test --locked --all-features` | pass | 131 unit passed, 2 ignored, integration and doctest passed |
| 5 | `cargo test --locked --no-default-features` | pass | 90 unit passed, 2 ignored, integration and doctest passed |
| 5 | both required Clippy feature commands | pass | No warnings |
| 5 | duplicate-field-copy search and `git diff --check` | pass | No old `semantics.<parallel-field>` access remains in pathology modules; no whitespace errors |
| 6 | `cargo test --locked scheme_tests -- --nocapture` | pass | All 6 strict JSON/digest/concept/builtin/color tests passed immediately after split |
| 6 | `cargo test --locked --all-features` | pass | 131 unit passed, 2 ignored, integration and doctest passed |
| 6 | `cargo test --locked --no-default-features` | pass | 90 unit passed, 2 ignored, integration and doctest passed |
| 6 | both required Clippy feature commands | pass | No warnings |
| 6 | module responsibility/digest search and `git diff --check` | pass | Old file absent; split files 15–316 lines; exact v1 namespaces present once each; no whitespace errors |
| 7 | `cargo test --locked --no-run` after fixture extraction | initial fail, then pass | Found and migrated two remaining private fixture imports; compile then passed |
| 7 | `cargo test --locked --test public_api --all-features -- --nocapture` | pass | 2 real downstream workflows passed |
| 7 | `cargo test --locked --test public_api --no-default-features -- --nocapture` | pass with warning, then clean in matrix | ANN/SEG/SR workflow passed; feature-gated import warning fixed |
| 7 | `cargo test --locked --all-features` | pass | 131 unit passed, 2 ignored; 2 public integration tests and doctest passed |
| 7 | `cargo test --locked --no-default-features` | pass | 90 unit passed, 2 ignored; 1 public integration test and doctest passed |
| 7 | both required Clippy feature commands | pass | No warnings |
| 7 | fixture ownership/line inventory and `git diff --check` | pass | Root 86 lines; focused modules 42–720; shared fixture owner 481; no stale `annotation_tests::write_source_wsi`; no whitespace errors |
| 8 | `cargo fmt --all -- --check` | pass | Phase 8 changes are formatted |
| 8 | `cargo test --locked annotations::sidecar::tests -- --nocapture` | pass | 3 strict classification/discovery/reference tests passed |
| 8 | `cargo test --locked --all-features` | pass | 131 unit passed, 2 ignored; 2 public integration tests and doctest passed |
| 8 | `cargo test --locked --no-default-features` | pass | 90 unit passed, 2 ignored; 1 public integration test and doctest passed |
| 8 | both required Clippy feature commands | pass | No warnings |
| 8 | SEG glob-import search and `git diff --check` | pass | Remaining 3 glob imports are test modules only; no whitespace errors |
| 9 | initial Ruby YAML validation plus audit/tree/benchmark command | partial failure | macOS Ruby 2.6 rejected the newer `YAML.load_file(..., aliases: true)` validator option; independent exact `lru` tree check and release benchmark compilation passed |
| 9 | `ruby -e 'require "yaml"; YAML.load_file(...)'` | pass | Workflow is syntactically valid YAML with the installed parser |
| 9 | immutable action-reference search | pass | Exactly one unique `actions/checkout@<40-hex>` reference across all jobs; resolved official v4.4.0 tag with `git ls-remote` |
| 9 | `cargo tree --locked --all-features --invert lru@0.16.4` plus RustSec advisory-file check | pass | Exact documented exception remains reachable through zarrs and the advisory remains live |
| 9 | `cargo test --release --locked --all-features seg_normalization_benchmark --no-run` | pass | Deterministic ignored benchmark workload compiles without a wall-time threshold |
| 9 | `cargo audit` | pass | 2 documented allowed unmaintained warnings; RUSTSEC-2026-0253 exception remains configured and reachable |
| 9 | explicit ignored external DICOM validator test | pass | Local dciodvfy/dcmdump/dcm2json validation succeeded after test-module split |
| 9 | `git diff --check` | pass | No whitespace errors |
| 10 | requested deletion-oriented `rg`/file searches | pass | Removed mutation/vectorization APIs absent; production SEG glob imports absent; brand strings test-only; one canonical row scanner; no parallel suffix files/placeholders; no new production unwrap/expect |
| 10 | `cargo fmt --all -- --check` | pass | Final tree formatted |
| 10 | `cargo test --locked` | pass | 131 unit passed, 2 ignored; 2 public integration tests and 2 doctests passed |
| 10 | `cargo test --locked --no-default-features` | pass | 90 unit passed, 2 ignored; 1 public integration test and 2 doctests passed |
| 10 | `cargo test --locked --all-features` | pass | 131 unit passed, 2 ignored; 2 public integration tests and 2 doctests passed |
| 10 | both required Clippy feature commands | pass | No warnings |
| 10 | `cargo build --release --locked` | pass | Optimized default-feature build completed |
| 10 | `cargo build --release --locked --no-default-features` | pass | Optimized core-only build completed |
| 10 | `cargo doc --no-deps --all-features --locked` | pass | Documentation and README examples completed |
| 10 | `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features --locked` | pass | CI-strength documentation gate passed |
| 10 | `cargo audit` plus exact dependency/advisory stale checks | pass | 2 documented allowed unmaintained warnings; configured RUSTSEC exception remains exact and live |
| 10 | external DICOM validator test | pass | Locally installed dciodvfy/dcmdump/dcm2json all accepted generated ANN/SEG |
| 10 | atomic publication, invalid prepublication, and legacy digest spot checks | pass | Each focused regression passed independently |
| 10 | final release benchmark | pass | 18.316959 ms binary/20, 38.713584 ms fractional/20, 15.86825 ms vector/5; checksums 581120/581120/145280 |
| 10 | `git diff --check` | pass | No whitespace errors after final documentation changes |
| 10 | `lsp server list` | pass | No servers running; no sibling server was stopped by this task |
| 11 | focused finding/provenance/SEG-run/ANN-round-trip tests before simplification | pass | Existing coverage established the behavior-preserving refactor baseline |
| 11 | `cargo check --locked --all-features` | pass | Constructor, validation, and run-budget simplifications compiled cleanly |
| 11 | focused finding/provenance/SEG-run/ANN-round-trip tests after simplification | pass | 2 finding, 3 provenance, 4 SEG run, and 1 ANN round-trip tests passed |
| 11 | `cargo test --locked --all-features` | pass | 131 unit passed, 2 ignored; 2 public integration tests and 2 doctests passed |
| 11 | `cargo test --locked --no-default-features` | pass | 90 unit passed, 2 ignored; 1 public integration test and 2 doctests passed |
| 11 | both required Clippy feature commands | pass | No warnings |
| 11 | constructor/budget residue search and `git diff --check` | pass | No `from_parts_with_finding` or configurable `RunBudget::new` call remains; no whitespace errors |
| 11 | final `lsp server list` | pass | Task-created servers for `wsi-dicom-annotations` and `dicom-viewer` were stopped; later servers at other roots (`wsi-dicom`, `wsi-rs`) were not task-owned and were left untouched |
| 12 | pathology companion producer test before implementation | failed as expected | `PathologyDicomDocuments::with_producers` was absent |
| 12 | `cargo test --locked shared_pathology_documents_build_and_verify_a_seg_referenced_sr_bundle` | pass | SEG/SR companion documents retained explicit Frames/DICOM Viewer identity |
| 12 | `cargo test --locked --all-features` | pass | 131 unit passed, 2 ignored; 2 public integration tests and 2 doctests passed |
| 12 | `cargo test --locked --no-default-features` | pass | 90 unit passed, 2 ignored; 1 public integration test and 2 doctests passed |
| 12 | both required Clippy feature commands | pass | No warnings after companion-producer API addition |
| 12 | `cargo doc --no-deps --all-features --locked` | pass | Public companion-producer API and README guidance generated successfully |
| 12 | downstream `dicom-viewer` workspace tests, Clippy, and release build | pass | 346 tests passed, 2 documented ignores; Clippy clean; optimized build completed in 2m29s |

## Performance ledger

- Method: optimized `cargo test --release` on aarch64-apple-darwin, one synthetic 1024×1024 source, 16 non-sequential segments, 16 64×64 frames per segment (256 total), each frame either dense or a deterministic 1-in-31 sparse pattern. Binary/fractional extraction ran 20 iterations; vectorization ran 5. `black_box` and output-count checks prevent elimination. No wall-time threshold is used.
- Before: binary 24.305708 ms/20; fractional 42.720333 ms/20; vector 10.453083 ms/5. Checksums: 581120 binary, 581120 fractional, 147200 vector (29,440 rectangles/iteration).
- After (median of two final warm runs): binary 18.214354 ms/20 (about 25% lower); fractional 39.133688 ms/20 (about 8% lower); vector 13.803062 ms/5 (about 32% higher). Binary/fractional checksums remain 581120; canonical merging intentionally changes vector checksum to 145280 (29,056 exact rectangles/iteration).
- Allocation/memory observation: no allocator profiler was available. Structurally, the imported binary path no longer clones 256 `Vec<bool>` buffers (1,048,576 mask bits, about 128 KiB of packed payload plus 256 allocations) on every normalization/vectorization call. Polygon vectors reserve exact canonical run counts. This is code-path accounting, not a measured peak-RSS claim.
- Limitation: one pre-change timing sample and two final warm samples on one machine; vectorization is slower because canonical sorting/merging now replaces a direct third scan. Do not generalize these numbers beyond this workload.
- Final verification sample after all phases: binary 18.316959 ms/20, fractional 38.713584 ms/20, vector 15.86825 ms/5, with the same post-change checksums. It is a verification sample, not folded into the two-run post-refactor median.

## Phase notes and changed files

- Phase 0 complete. Changed files: this plan only. All thirteen findings were confirmed against HEAD; none are obsolete.
- Phase 1 complete. Changed files: `src/annotations/ann.rs`, `src/annotations/ann_validation_tests.rs`, `src/annotations/context.rs`, `src/annotations/model/annotation.rs`, `src/annotations/model/annotation_tests.rs`, and this plan. WDA-001 is green.
- Phase 2 complete. Changed files: `src/annotations/derived_object.rs`, ANN/SEG/SR/PM model/read/write modules, `src/annotations/mod.rs`, `src/annotations/provenance_tests.rs`, `src/parametric_map_tests.rs`, and this plan. WDA-002 and WDA-013 are green.
- Phase 3 complete. Changed files: `src/annotations/seg.rs`, `src/annotations/seg/{document.rs,raster.rs,run_tests.rs,performance_tests.rs}`, and this plan. WDA-004 and WDA-011 are green.
- Phase 4 complete. Changed files: `src/annotations/seg.rs`, `src/annotations/seg/{document.rs,vectorization_tests.rs,run_tests.rs,performance_tests.rs}`, `src/annotation_tests.rs`, `src/annotations/mod.rs`, and this plan. WDA-003 is green.
- Phase 5 complete. Changed files: `src/annotations/model.rs`, `src/annotations/model/{finding.rs,finding_tests.rs,annotation.rs}`, SEG model/read/vectorization, pathology mapping/conversion/preview/SR conversion, and this plan. WDA-005 is green.
- Phase 6 complete. Changed files: removed `src/annotations/scheme.rs`; added `src/annotations/scheme/{mod.rs,model.rs,json.rs,builtins.rs,concept_key.rs,color.rs}`; updated this plan. WDA-006 is green.
- Phase 7 complete. Changed files: `src/annotation_tests.rs`, new `src/annotation_tests/*.rs`, new `src/test_support/*.rs`, `src/lib.rs`, all fixture consumer imports, `tests/public_api.rs`, and this plan. WDA-007 and WDA-012 are green.
- Phase 8 complete. Changed files: `src/annotations/sidecar.rs`, `src/annotations/seg.rs`, `src/annotations/seg/{document.rs,raster.rs,read.rs,read/layout.rs,write.rs,vectorization_tests.rs}`, and this plan. WDA-008 and WDA-009 are green.
- Phase 9 complete. Changed files: `.github/workflows/ci.yml`, `README.md`, and this plan. WDA-010 is green; WDA-011's benchmark now compiles in CI without a timing threshold.
- Phase 10 complete. The deletion review found no abandoned compatibility or parallel implementation. The entire required matrix, strict docs, audit/stale-exception checks, external validators, atomic publication, stable digest, and final benchmark passed.
- Phase 11 complete. Applied all four accepted ponytail-review cuts across ANN construction, producer validation, and SEG run budgeting; the reviewed change set is 72 net source lines smaller and remains green in both feature modes.
- Phase 12 complete. Closed the cross-repository API migration: pathology bundles accept per-format producers; the viewer supplies explicit identity to every new ANN/SEG/SR/PM path and surfaces SEG projection losses.

## Remaining risks

- The legacy SEG digest is externally reproduced by `wsi-annotation-interop` and cannot be silently renamed.
- The checked-in GitHub workflow was parsed and every command was executed locally, but cannot be observed on a hosted runner until a future authorized push or pull request.

## Current next action

Repository-local and downstream API migrations are complete. No further WDA implementation action remains; `dicom-viewer` may continue its separately planned tile-pipeline refactor.
