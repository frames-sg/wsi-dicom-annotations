use std::ops::Range;

use super::{
    BinaryMaskRun, FractionalMaskRun, SegmentationDocument, SegmentationKind, MAX_VECTORIZED_RUNS,
};
use crate::{Error, Result};

pub(super) struct RunBudget {
    count: usize,
    limit: usize,
}

impl RunBudget {
    pub(super) const fn new(limit: usize) -> Self {
        Self { count: 0, limit }
    }

    fn record(&mut self) -> Result<()> {
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| Error::InvalidInput("SEG run count overflows".into()))?;
        if self.count > self.limit {
            return Err(Error::InvalidInput(format!(
                "SEG normalization exceeds the {}-run resource limit",
                self.limit
            )));
        }
        Ok(())
    }
}

pub(super) fn scan_nonzero_ranges<T>(
    values: &[T],
    mut is_nonzero: impl FnMut(&T) -> bool,
    budget: &mut RunBudget,
    mut emit: impl FnMut(Range<usize>) -> Result<()>,
) -> Result<()> {
    let mut column = 0_usize;
    while column < values.len() {
        if !is_nonzero(&values[column]) {
            column += 1;
            continue;
        }
        let start = column;
        while column < values.len() && is_nonzero(&values[column]) {
            column += 1;
        }
        budget.record()?;
        emit(start..column)?;
    }
    Ok(())
}

pub(super) fn binary_runs(document: &SegmentationDocument) -> Result<Vec<BinaryMaskRun>> {
    if document.kind == SegmentationKind::Fractional {
        return Err(Error::Unsupported(
            "fractional SEG does not contain binary mask runs".into(),
        ));
    }
    let frames = document.binary_frames()?;
    let mut runs = Vec::new();
    let mut budget = RunBudget::new(MAX_VECTORIZED_RUNS);
    for frame in frames.iter() {
        for_each_checked_frame_row(
            frame.segment_number,
            frame.tile_col,
            frame.tile_row,
            frame.width,
            frame.height,
            &frame.mask,
            |segment_number, absolute_row, base_column, values| {
                scan_nonzero_ranges(
                    values,
                    |value| *value,
                    &mut budget,
                    |range| {
                        let start = u32::try_from(range.start).map_err(|_| {
                            Error::InvalidInput("SEG column index exceeds DICOM range".into())
                        })?;
                        let length = u32::try_from(range.len()).map_err(|_| {
                            Error::InvalidInput("SEG run length exceeds DICOM range".into())
                        })?;
                        let column_start = base_column.checked_add(start).ok_or_else(|| {
                            Error::InvalidInput("SEG column position overflows".into())
                        })?;
                        runs.push(BinaryMaskRun {
                            segment_number,
                            row: absolute_row,
                            column_start,
                            length,
                        });
                        Ok(())
                    },
                )
            },
        )?;
    }
    runs.sort_by_key(|run| (run.segment_number, run.row, run.column_start));
    merge_binary_runs(runs)
}

pub(super) fn fractional_runs(document: &SegmentationDocument) -> Result<Vec<FractionalMaskRun>> {
    let frames = document
        .fractional_frames()
        .ok_or_else(|| Error::Unsupported("SEG does not contain fractional mask values".into()))?;
    let mut runs = Vec::new();
    let mut budget = RunBudget::new(MAX_VECTORIZED_RUNS);
    for frame in frames {
        for_each_checked_frame_row(
            frame.segment_number,
            frame.tile_col,
            frame.tile_row,
            frame.width,
            frame.height,
            &frame.values,
            |segment_number, absolute_row, base_column, values| {
                scan_nonzero_ranges(
                    values,
                    |value| *value != 0,
                    &mut budget,
                    |range| {
                        let start = u32::try_from(range.start).map_err(|_| {
                            Error::InvalidInput("SEG column index exceeds DICOM range".into())
                        })?;
                        let column_start = base_column.checked_add(start).ok_or_else(|| {
                            Error::InvalidInput("SEG column position overflows".into())
                        })?;
                        runs.push(FractionalMaskRun {
                            segment_number,
                            row: absolute_row,
                            column_start,
                            maximum_fractional_value: frame.maximum_fractional_value,
                            values: values[range].to_vec(),
                        });
                        Ok(())
                    },
                )
            },
        )?;
    }
    runs.sort_by_key(|run| (run.segment_number, run.row, run.column_start));
    merge_fractional_runs(runs)
}

#[allow(clippy::too_many_arguments)]
fn for_each_checked_frame_row<T>(
    segment_number: u16,
    tile_col: u32,
    tile_row: u32,
    width: u16,
    height: u16,
    values: &[T],
    mut visit: impl FnMut(u16, u32, u32, &[T]) -> Result<()>,
) -> Result<()> {
    let width_usize = usize::from(width);
    let height_usize = usize::from(height);
    let expected = width_usize
        .checked_mul(height_usize)
        .ok_or_else(|| Error::InvalidInput("SEG frame dimensions overflow".into()))?;
    if values.len() != expected {
        return Err(Error::InvalidInput(format!(
            "SEG frame contains {} samples for {width}x{height} dimensions",
            values.len()
        )));
    }
    let base_column = tile_col
        .checked_mul(u32::from(width))
        .ok_or_else(|| Error::InvalidInput("SEG column position overflows".into()))?;
    let base_row = tile_row
        .checked_mul(u32::from(height))
        .ok_or_else(|| Error::InvalidInput("SEG row position overflows".into()))?;
    for row in 0..height_usize {
        let row_start = row * width_usize;
        let row = u32::try_from(row)
            .map_err(|_| Error::InvalidInput("SEG row index exceeds DICOM range".into()))?;
        let absolute_row = base_row
            .checked_add(row)
            .ok_or_else(|| Error::InvalidInput("SEG row position overflows".into()))?;
        visit(
            segment_number,
            absolute_row,
            base_column,
            &values[row_start..row_start + width_usize],
        )?;
    }
    Ok(())
}

fn merge_binary_runs(runs: Vec<BinaryMaskRun>) -> Result<Vec<BinaryMaskRun>> {
    let mut merged: Vec<BinaryMaskRun> = Vec::with_capacity(runs.len());
    for run in runs {
        if let Some(previous) = merged.last_mut() {
            let previous_end = previous
                .column_start
                .checked_add(previous.length)
                .ok_or_else(|| Error::InvalidInput("SEG run end overflows".into()))?;
            let run_end = run
                .column_start
                .checked_add(run.length)
                .ok_or_else(|| Error::InvalidInput("SEG run end overflows".into()))?;
            if previous.segment_number == run.segment_number
                && previous.row == run.row
                && run.column_start <= previous_end
            {
                previous.length = previous_end.max(run_end) - previous.column_start;
                continue;
            }
        }
        merged.push(run);
    }
    Ok(merged)
}

fn merge_fractional_runs(runs: Vec<FractionalMaskRun>) -> Result<Vec<FractionalMaskRun>> {
    let mut merged: Vec<FractionalMaskRun> = Vec::with_capacity(runs.len());
    for run in runs {
        if let Some(previous) = merged.last_mut() {
            let previous_end = previous
                .column_start
                .checked_add(u32::try_from(previous.values.len()).map_err(|_| {
                    Error::InvalidInput("fractional SEG run length exceeds DICOM range".into())
                })?)
                .ok_or_else(|| Error::InvalidInput("SEG run end overflows".into()))?;
            if previous.segment_number == run.segment_number && previous.row == run.row {
                if run.column_start < previous_end {
                    return Err(Error::InvalidInput(
                        "fractional SEG frames overlap within the same segment".into(),
                    ));
                }
                if run.column_start == previous_end
                    && previous.maximum_fractional_value == run.maximum_fractional_value
                {
                    previous.values.extend(run.values);
                    continue;
                }
            }
        }
        merged.push(run);
    }
    Ok(merged)
}
