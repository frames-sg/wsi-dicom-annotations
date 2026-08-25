use std::hint::black_box;
use std::time::{Duration, Instant};

use crate::test_support::write_source_wsi;

use super::*;

const SEGMENTS: usize = 16;
const FRAMES_PER_SEGMENT: usize = 16;
const TILE: usize = 64;

fn code(value: &str, meaning: &str) -> DicomCode {
    DicomCode::new(value, "99FRAMES", meaning).unwrap()
}

pub(super) fn benchmark_documents() -> (SegmentationDocument, SegmentationDocument) {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.dcm");
    write_source_wsi(&source_path, 1024, 1024, TILE as u16, TILE as u16);
    let source = DicomAnnotationContext::from_source(&source_path).unwrap();
    let mut segments = (0..SEGMENTS)
        .map(|index| {
            let offset = (index * 8) as f64;
            SegmentationSegment::new(
                format!("Segment {index}"),
                code("MORPH", "Morphology"),
                code("TUMOR", "Tumor"),
                [1, 2, 3],
                vec![vec![
                    Point2::new(offset, offset),
                    Point2::new(offset + 4.0, offset),
                    Point2::new(offset + 4.0, offset + 4.0),
                    Point2::new(offset, offset + 4.0),
                ]],
                Vec::new(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    for (index, segment) in segments.iter_mut().enumerate() {
        segment.source_segment_number = Some((index as u16) * 3 + 1);
    }
    let binary_frames = segments
        .iter()
        .flat_map(|segment| {
            (0..FRAMES_PER_SEGMENT).map(move |frame_index| {
                let dense = frame_index % 4 == 0;
                let mask = (0..TILE * TILE)
                    .map(|pixel| dense || (pixel + frame_index) % 31 == 0)
                    .collect();
                BinarySegmentationFrame {
                    segment_number: segment.source_segment_number.unwrap(),
                    tile_col: (frame_index % 4) as u32,
                    tile_row: (frame_index / 4) as u32,
                    width: TILE as u16,
                    height: TILE as u16,
                    mask,
                }
            })
        })
        .collect::<Vec<_>>();
    let fractional_frames = binary_frames
        .iter()
        .enumerate()
        .map(|(frame_index, frame)| FractionalSegmentationFrame {
            segment_number: frame.segment_number,
            tile_col: frame.tile_col,
            tile_row: frame.tile_row,
            width: frame.width,
            height: frame.height,
            maximum_fractional_value: 255,
            values: frame
                .mask
                .iter()
                .enumerate()
                .map(|(pixel, value)| {
                    if *value {
                        ((pixel + frame_index) % 255 + 1) as u16
                    } else {
                        0
                    }
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let mut binary = SegmentationDocument::binary(source, segments).unwrap();
    binary.imported_binary_frames = Some(binary_frames);
    let mut fractional = binary.clone();
    fractional.kind = SegmentationKind::Fractional;
    fractional.imported_binary_frames = None;
    fractional.imported_fractional_frames = Some(fractional_frames);
    (binary, fractional)
}

fn measure(iterations: usize, mut operation: impl FnMut() -> usize) -> (Duration, usize) {
    let start = Instant::now();
    let mut checksum = 0_usize;
    for _ in 0..iterations {
        checksum = checksum.wrapping_add(black_box(operation()));
    }
    (start.elapsed(), checksum)
}

#[test]
#[ignore = "repeatable local performance measurement; not a wall-time CI gate"]
fn seg_normalization_benchmark() {
    let (binary, fractional) = benchmark_documents();
    let (binary_elapsed, binary_checksum) = measure(20, || binary.binary_runs().unwrap().len());
    let (fractional_elapsed, fractional_checksum) =
        measure(20, || fractional.fractional_runs().unwrap().len());
    let (vector_elapsed, vector_checksum) = measure(5, || {
        binary
            .vectorized_annotations(SegToAnnConversionPolicy::AllowLoss)
            .unwrap()
            .groups()
            .iter()
            .map(AnnotationGroup::annotation_count)
            .sum()
    });

    assert!(binary_checksum > 0);
    assert!(fractional_checksum > 0);
    assert!(vector_checksum > 0);
    println!(
        "frames={} segments={} binary_20={binary_elapsed:?} binary_checksum={binary_checksum} fractional_20={fractional_elapsed:?} fractional_checksum={fractional_checksum} vector_5={vector_elapsed:?} vector_checksum={vector_checksum}",
        SEGMENTS * FRAMES_PER_SEGMENT,
        SEGMENTS,
    );
}
