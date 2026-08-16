use crate::{DicomCode, Error, Point2, Result};

use super::source::PixelTile;
use super::ParametricMapDocument;

const PREVIEW_CHUNK_SIDE: u32 = 256;
const MAX_PREVIEW_PIXELS: usize = 1_048_576;

#[derive(Debug, Clone)]
pub struct ParametricMapPreview {
    width: u32,
    height: u32,
    normalized_values: Vec<f32>,
    minimum: f64,
    maximum: f64,
    channel_name: String,
    quantity: DicomCode,
    unit: DicomCode,
    base_corners: [Point2; 4],
}

impl ParametricMapPreview {
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    #[must_use]
    pub fn normalized_values(&self) -> &[f32] {
        &self.normalized_values
    }

    #[must_use]
    pub const fn value_range(&self) -> (f64, f64) {
        (self.minimum, self.maximum)
    }

    #[must_use]
    pub fn channel_name(&self) -> &str {
        &self.channel_name
    }

    #[must_use]
    pub const fn quantity(&self) -> &DicomCode {
        &self.quantity
    }

    #[must_use]
    pub const fn unit(&self) -> &DicomCode {
        &self.unit
    }

    #[must_use]
    pub const fn base_corners(&self) -> &[Point2; 4] {
        &self.base_corners
    }
}

impl ParametricMapDocument {
    /// Build a bounded, display-only heatmap for one selected parameter.
    ///
    /// Values are averaged after normalization into the parameter's finite
    /// Real World Value range. Missing bins remain NaN. This scans the source
    /// in bounded chunks and never allocates the full raster.
    pub fn preview(
        &self,
        selected_channel_ordinal: usize,
        maximum_pixels: usize,
    ) -> Result<ParametricMapPreview> {
        if maximum_pixels == 0 || maximum_pixels > MAX_PREVIEW_PIXELS {
            return Err(Error::InvalidInput(format!(
                "PM preview pixel limit must be 1..={MAX_PREVIEW_PIXELS}"
            )));
        }
        let channel = *self
            .selected_channels
            .get(selected_channel_ordinal)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "selected PM preview channel ordinal {selected_channel_ordinal} is outside 0..{}",
                    self.selected_channels.len()
                ))
            })?;
        let range = self.channel_ranges[selected_channel_ordinal];
        let (width, height) = preview_dimensions(
            self.descriptor.width,
            self.descriptor.height,
            maximum_pixels,
        )?;
        let normalized_values = read_preview_values(self, channel, width, height, range)?;
        let channel_profile = &self.profile.channels[channel];
        Ok(ParametricMapPreview {
            width,
            height,
            normalized_values,
            minimum: range.minimum,
            maximum: range.maximum,
            channel_name: channel_profile.name.clone(),
            quantity: channel_profile.quantity.clone(),
            unit: channel_profile.unit.clone(),
            base_corners: base_corners(self)?,
        })
    }
}

fn read_preview_values(
    document: &ParametricMapDocument,
    channel: usize,
    width: u32,
    height: u32,
    range: super::document::ValueRange,
) -> Result<Vec<f32>> {
    let mut accumulator = PreviewAccumulator::new(
        document.descriptor.width,
        document.descriptor.height,
        width,
        height,
        range,
    )?;
    for origin_y in (0..document.descriptor.height).step_by(PREVIEW_CHUNK_SIDE as usize) {
        let chunk_height = PREVIEW_CHUNK_SIDE.min(document.descriptor.height - origin_y);
        for origin_x in (0..document.descriptor.width).step_by(PREVIEW_CHUNK_SIDE as usize) {
            let chunk_width = PREVIEW_CHUNK_SIDE.min(document.descriptor.width - origin_x);
            let tile = document.raster.read_tile(
                channel,
                origin_y,
                origin_x,
                chunk_height,
                chunk_width,
            )?;
            accumulator.add_tile(&tile, origin_y, origin_x, chunk_width);
        }
    }
    Ok(accumulator.finish())
}

fn preview_dimensions(width: u32, height: u32, maximum_pixels: usize) -> Result<(u32, u32)> {
    let source_pixels = u64::from(width) * u64::from(height);
    if source_pixels <= maximum_pixels as u64 {
        return Ok((width, height));
    }
    let aspect = f64::from(width) / f64::from(height);
    let mut preview_width = ((maximum_pixels as f64 * aspect).sqrt().round() as u64)
        .clamp(1, u64::from(width).min(maximum_pixels as u64));
    let preview_height = (maximum_pixels as u64 / preview_width).clamp(1, u64::from(height));
    preview_width = preview_width.min(maximum_pixels as u64 / preview_height);
    if preview_width == 0 || preview_height == 0 {
        return Err(Error::InvalidInput(
            "PM preview dimensions resolved to zero".into(),
        ));
    }
    Ok((preview_width as u32, preview_height as u32))
}

struct PreviewAccumulator {
    source_width: u32,
    source_height: u32,
    preview_width: u32,
    preview_height: u32,
    range: super::document::ValueRange,
    sums: Vec<f64>,
    counts: Vec<u64>,
}

impl PreviewAccumulator {
    fn new(
        source_width: u32,
        source_height: u32,
        preview_width: u32,
        preview_height: u32,
        range: super::document::ValueRange,
    ) -> Result<Self> {
        let length = usize::try_from(u64::from(preview_width) * u64::from(preview_height))
            .map_err(|_| {
                Error::InvalidInput("PM preview dimensions exceed addressable memory".into())
            })?;
        Ok(Self {
            source_width,
            source_height,
            preview_width,
            preview_height,
            range,
            sums: vec![0.0; length],
            counts: vec![0; length],
        })
    }

    fn add_tile(&mut self, tile: &PixelTile, origin_y: u32, origin_x: u32, chunk_width: u32) {
        match tile {
            PixelTile::F32(values) => values.iter().enumerate().for_each(|(index, value)| {
                self.observe(index, f64::from(*value), origin_y, origin_x, chunk_width);
            }),
            PixelTile::F64(values) => values.iter().enumerate().for_each(|(index, value)| {
                self.observe(index, *value, origin_y, origin_x, chunk_width);
            }),
        }
    }

    fn observe(
        &mut self,
        index: usize,
        value: f64,
        origin_y: u32,
        origin_x: u32,
        chunk_width: u32,
    ) {
        if !value.is_finite() {
            return;
        }
        let row = origin_y + index as u32 / chunk_width;
        let column = origin_x + index as u32 % chunk_width;
        let preview_row =
            u64::from(row) * u64::from(self.preview_height) / u64::from(self.source_height);
        let preview_column =
            u64::from(column) * u64::from(self.preview_width) / u64::from(self.source_width);
        let preview_index = (preview_row * u64::from(self.preview_width) + preview_column) as usize;
        let normalized = if self.range.minimum == self.range.maximum {
            0.5
        } else {
            ((value - self.range.minimum) / (self.range.maximum - self.range.minimum))
                .clamp(0.0, 1.0)
        };
        self.sums[preview_index] += normalized;
        self.counts[preview_index] += 1;
    }

    fn finish(self) -> Vec<f32> {
        self.sums
            .into_iter()
            .zip(self.counts)
            .map(|(sum, count)| {
                if count == 0 {
                    f32::NAN
                } else {
                    (sum / count as f64) as f32
                }
            })
            .collect()
    }
}

fn base_corners(document: &ParametricMapDocument) -> Result<[Point2; 4]> {
    let right = f64::from(document.descriptor.width) - 0.5;
    let bottom = f64::from(document.descriptor.height) - 0.5;
    let point = |column, row| {
        let position = document.geometry.position_at(column, row);
        document
            .source
            .slide_coordinate_to_pixel3(position[0], position[1], position[2])
    };
    Ok([
        point(-0.5, -0.5)?,
        point(right, -0.5)?,
        point(right, bottom)?,
        point(-0.5, bottom)?,
    ])
}
