use super::raster::{scan_nonzero_ranges, RunBudget};
use super::*;

#[test]
fn shared_row_scanner_handles_empty_sparse_and_dense_rows() {
    let cases = [
        (Vec::<u8>::new(), Vec::new()),
        (vec![0, 0, 0], Vec::new()),
        (vec![1, 1, 1], std::iter::once(0..3).collect()),
        (vec![0, 1, 1, 0, 1, 0], vec![1..3, 4..5]),
    ];
    for (row, expected) in cases {
        let mut actual = Vec::new();
        let mut budget = RunBudget::new(10);
        scan_nonzero_ranges(
            &row,
            |value| *value != 0,
            &mut budget,
            |range| {
                actual.push(range);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn shared_row_scanner_enforces_one_consistent_run_budget() {
    let mut budget = RunBudget::new(2);
    let error = scan_nonzero_ranges(
        &[1, 0, 1, 0, 1],
        |value| *value != 0,
        &mut budget,
        |_| Ok(()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("2-run resource limit"));
}

#[test]
fn canonical_runs_drive_deterministic_vectorization_for_nonsequential_segments() {
    let (mut binary, _) = super::performance_tests::benchmark_documents();
    binary.imported_binary_frames.as_mut().unwrap().reverse();
    let runs = binary.binary_runs().unwrap();
    assert!(runs.windows(2).all(|pair| {
        (pair[0].segment_number, pair[0].row, pair[0].column_start)
            <= (pair[1].segment_number, pair[1].row, pair[1].column_start)
    }));

    let vectorized = binary
        .vectorized_annotations(SegToAnnConversionPolicy::AllowLoss)
        .unwrap();
    let groups = vectorized.groups();
    assert_eq!(groups.len(), 16);
    assert_eq!(
        groups
            .iter()
            .map(AnnotationGroup::annotation_count)
            .sum::<usize>(),
        runs.len()
    );
    for (index, group) in groups.iter().enumerate() {
        assert_eq!(group.label(), format!("Segment {index}"));
        let segment_number = (index as u16) * 3 + 1;
        let expected = runs
            .iter()
            .filter(|run| run.segment_number == segment_number)
            .collect::<Vec<_>>();
        assert_eq!(group.annotation_count(), expected.len());
        for (polygon, run) in group.polygon_annotations().unwrap().iter().zip(expected) {
            let x0 = f64::from(run.column_start);
            let x1 = f64::from(run.column_start + run.length);
            let y0 = f64::from(run.row);
            assert_eq!(
                polygon,
                &[
                    Point2::new(x0, y0),
                    Point2::new(x1, y0),
                    Point2::new(x1, y0 + 1.0),
                    Point2::new(x0, y0 + 1.0),
                ]
            );
        }
    }
}

#[test]
fn run_extraction_rejects_tile_offset_overflow() {
    let (mut binary, _) = super::performance_tests::benchmark_documents();
    let frame = &mut binary.imported_binary_frames.as_mut().unwrap()[0];
    frame.tile_col = u32::MAX;
    assert!(binary
        .binary_runs()
        .unwrap_err()
        .to_string()
        .contains("overflows"));
}
