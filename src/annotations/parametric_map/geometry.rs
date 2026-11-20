use crate::{DicomAnnotationContext, Error, Result};

use super::profile::{RasterCoordinateSpace, RasterProfile};
use super::source::RasterDescriptor;

#[derive(Debug, Clone, Copy)]
pub(super) struct RasterGeometry {
    pub(super) origin: [f64; 3],
    pub(super) orientation: [f64; 6],
    pub(super) pixel_spacing: [f64; 2],
    pub(super) slice_thickness: f64,
}

impl RasterGeometry {
    pub(super) fn resolve(
        source: &DicomAnnotationContext,
        canonical_source: &DicomAnnotationContext,
        profile: &RasterProfile,
        descriptor: RasterDescriptor,
    ) -> Result<Self> {
        let base = match profile.coordinate_space {
            RasterCoordinateSpace::Level0Pixels => {
                source.require_shared_slide_frame(canonical_source)?;
                canonical_source
            }
            RasterCoordinateSpace::SourcePixels | RasterCoordinateSpace::SlideMm => source,
        };
        let orientation = base.image_orientation_slide().ok_or_else(|| {
            Error::InvalidInput(
                "raster conversion requires source Image Orientation (Slide)".into(),
            )
        })?;
        let slice_thickness = base.slice_thickness().ok_or_else(|| {
            Error::InvalidInput(
                "raster conversion requires source Slice Thickness/depth of field".into(),
            )
        })?;
        let (origin, pixel_spacing) = match profile.coordinate_space {
            RasterCoordinateSpace::Level0Pixels | RasterCoordinateSpace::SourcePixels => {
                let source_spacing = base.pixel_spacing().ok_or_else(|| {
                    Error::InvalidInput(
                        "pixel-coordinate raster conversion requires source Pixel Spacing".into(),
                    )
                })?;
                let origin =
                    base.pixel_to_slide_coordinate(profile.grid_origin.x, profile.grid_origin.y)?;
                validate_last_pixel_sample(base, profile, descriptor)?;
                (
                    [origin.x, origin.y, origin.z],
                    [
                        profile.sample_spacing.y * source_spacing[0],
                        profile.sample_spacing.x * source_spacing[1],
                    ],
                )
            }
            RasterCoordinateSpace::SlideMm => {
                let z = profile
                    .grid_origin
                    .z
                    .ok_or_else(|| Error::InvalidInput("slide-mm grid origin requires z".into()))?;
                let origin = [profile.grid_origin.x, profile.grid_origin.y, z];
                (origin, [profile.sample_spacing.y, profile.sample_spacing.x])
            }
        };
        let geometry = Self {
            origin,
            orientation,
            pixel_spacing,
            slice_thickness,
        };
        if profile.coordinate_space == RasterCoordinateSpace::SlideMm {
            for (column, row) in [(0, 0), (descriptor.width - 1, descriptor.height - 1)] {
                let [x, y, z] = geometry.position_at(f64::from(column), f64::from(row));
                let pixel = source.slide_coordinate_to_pixel3(x, y, z)?;
                validate_sample(source, pixel.x, pixel.y)?;
            }
        }
        Ok(geometry)
    }

    pub(super) fn frame_position(self, column_position: u32, row_position: u32) -> [f64; 3] {
        self.position_at(f64::from(column_position - 1), f64::from(row_position - 1))
    }

    pub(super) fn position_at(self, column_offset: f64, row_offset: f64) -> [f64; 3] {
        let column_distance = column_offset * self.pixel_spacing[1];
        let row_distance = row_offset * self.pixel_spacing[0];
        [
            self.origin[0]
                + column_distance * self.orientation[0]
                + row_distance * self.orientation[3],
            self.origin[1]
                + column_distance * self.orientation[1]
                + row_distance * self.orientation[4],
            self.origin[2]
                + column_distance * self.orientation[2]
                + row_distance * self.orientation[5],
        ]
    }
}

fn validate_last_pixel_sample(
    base: &DicomAnnotationContext,
    profile: &RasterProfile,
    descriptor: RasterDescriptor,
) -> Result<()> {
    let x = profile.grid_origin.x + f64::from(descriptor.width - 1) * profile.sample_spacing.x;
    let y = profile.grid_origin.y + f64::from(descriptor.height - 1) * profile.sample_spacing.y;
    validate_sample(base, x, y)
}

fn validate_sample(source: &DicomAnnotationContext, x: f64, y: f64) -> Result<()> {
    let (width, height) = source.total_pixel_matrix_dimensions();
    // The source and raster origins are pixel centers. Allow roundoff when
    // projecting millimeters back onto the source grid.
    const TOLERANCE_PX: f64 = 1e-6;
    if !(-TOLERANCE_PX..=f64::from(width - 1) + TOLERANCE_PX).contains(&x)
        || !(-TOLERANCE_PX..=f64::from(height - 1) + TOLERANCE_PX).contains(&y)
    {
        return Err(Error::InvalidInput(format!(
            "raster sample ({x}, {y}) is outside the {width} × {height} source pixel grid"
        )));
    }
    Ok(())
}
