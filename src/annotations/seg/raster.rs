use super::*;

pub(super) fn rasterize_segments(
    source: &DicomAnnotationContext,
    segments: &[SegmentationSegment],
) -> Result<Vec<BinarySegmentationFrame>> {
    let (matrix_width, matrix_height) = source.total_pixel_matrix_dimensions();
    let (tile_width, tile_height) = source.tile_dimensions();
    let tiles_across = matrix_width.div_ceil(u32::from(tile_width));
    let tiles_down = matrix_height.div_ceil(u32::from(tile_height));
    let pixels_per_tile = usize::from(tile_width) * usize::from(tile_height);
    let mut frames = Vec::new();
    let mut candidate_pixel_count = 0_usize;
    for (segment_index, segment) in segments.iter().enumerate() {
        let segment_number = u16::try_from(segment_index + 1)
            .map_err(|_| Error::InvalidInput("segment number exceeds DICOM US range".into()))?;
        let candidate_tiles = segment
            .outer_polygons()
            .iter()
            .flat_map(|polygon| {
                polygon_candidate_tiles(polygon, tile_width, tile_height, tiles_across, tiles_down)
            })
            .collect::<BTreeSet<_>>();
        if candidate_tiles.len() > MAX_SEGMENTATION_FRAMES.saturating_sub(frames.len()) {
            return Err(Error::InvalidInput(format!(
                "SEG annotated area exceeds the {MAX_SEGMENTATION_FRAMES}-frame rasterization limit"
            )));
        }
        candidate_pixel_count = candidate_pixel_count
            .checked_add(candidate_tiles.len().saturating_mul(pixels_per_tile))
            .ok_or_else(|| Error::InvalidInput("SEG rasterization size overflows".into()))?;
        if candidate_pixel_count > MAX_SEGMENTATION_PIXELS {
            return Err(Error::InvalidInput(format!(
                "SEG annotated tiles contain {candidate_pixel_count} pixels, exceeding the {MAX_SEGMENTATION_PIXELS}-pixel rasterization limit"
            )));
        }
        for (tile_row, tile_col) in candidate_tiles {
            let mut mask = vec![false; pixels_per_tile];
            let base_x = tile_col * u32::from(tile_width);
            let base_y = tile_row * u32::from(tile_height);
            for local_y in 0..u32::from(tile_height) {
                let y = base_y + local_y;
                if y >= matrix_height {
                    break;
                }
                for local_x in 0..u32::from(tile_width) {
                    let x = base_x + local_x;
                    if x >= matrix_width {
                        break;
                    }
                    let point = Point2::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                    let included = segment_contains_point(segment, point);
                    mask[local_y as usize * usize::from(tile_width) + local_x as usize] = included;
                }
            }
            if mask.iter().any(|value| *value) {
                frames.push(BinarySegmentationFrame {
                    segment_number,
                    tile_col,
                    tile_row,
                    width: tile_width,
                    height: tile_height,
                    mask,
                });
            }
        }
    }
    Ok(frames)
}

fn segment_contains_point(segment: &SegmentationSegment, point: Point2) -> bool {
    if let Some(component_holes) = &segment.component_holes {
        return segment
            .outer_polygons
            .iter()
            .zip(component_holes)
            .any(|(outer, holes)| {
                polygon_contains_point(outer, point)
                    && !holes.iter().any(|hole| polygon_contains_point(hole, point))
            });
    }
    segment
        .outer_polygons
        .iter()
        .any(|polygon| polygon_contains_point(polygon, point))
        && !segment
            .exclusion_polygons
            .iter()
            .any(|polygon| polygon_contains_point(polygon, point))
}

fn polygon_candidate_tiles(
    polygon: &[Point2],
    tile_width: u16,
    tile_height: u16,
    tiles_across: u32,
    tiles_down: u32,
) -> impl Iterator<Item = (u32, u32)> {
    let (min_x, max_x) = polygon
        .iter()
        .map(|point| point.x)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    let (min_y, max_y) = polygon
        .iter()
        .map(|point| point.y)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    let last_col = tiles_across.saturating_sub(1);
    let last_row = tiles_down.saturating_sub(1);
    let first_col = ((min_x.max(0.0) / f64::from(tile_width)).floor() as u32).min(last_col);
    let final_col = ((max_x.max(0.0) / f64::from(tile_width)).floor() as u32).min(last_col);
    let first_row = ((min_y.max(0.0) / f64::from(tile_height)).floor() as u32).min(last_row);
    let final_row = ((max_y.max(0.0) / f64::from(tile_height)).floor() as u32).min(last_row);
    (first_row..=final_row)
        .flat_map(move |tile_row| (first_col..=final_col).map(move |tile_col| (tile_row, tile_col)))
}

pub(super) fn merge_binary_runs(runs: Vec<BinaryMaskRun>) -> Result<Vec<BinaryMaskRun>> {
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

pub(super) fn merge_fractional_runs(
    runs: Vec<FractionalMaskRun>,
) -> Result<Vec<FractionalMaskRun>> {
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
