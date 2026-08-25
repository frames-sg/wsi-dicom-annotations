use std::path::{Path, PathBuf};

use dicom_core::Tag;
use dicom_dictionary_std::{tags, uids};
use dicom_object::DefaultDicomObject;

use crate::metadata::open_metadata_object;
use crate::{Error, Result};

use super::dicom_dataset::{
    optional_string, optional_u32, required_string, required_u16, required_u32,
};
use super::model::{Point2, Point3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameOrigin {
    column: u32,
    row: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DicomAnnotationContext {
    source_path: PathBuf,
    sop_class_uid: String,
    sop_instance_uid: String,
    series_instance_uid: String,
    study_instance_uid: String,
    frame_of_reference_uid: Option<String>,
    total_pixel_matrix_columns: u32,
    total_pixel_matrix_rows: u32,
    tile_columns: u16,
    tile_rows: u16,
    pixel_spacing: Option<[f64; 2]>,
    slice_thickness: Option<f64>,
    image_orientation_slide: Option<[f64; 6]>,
    total_pixel_matrix_origin: Option<[f64; 3]>,
    dimension_organization_type: Option<String>,
    number_of_frames: Option<u32>,
    number_of_optical_paths: Option<u32>,
    total_pixel_matrix_focal_planes: Option<u32>,
    sparse_frame_origins: Option<Vec<FrameOrigin>>,
}

impl DicomAnnotationContext {
    pub fn from_source(path: impl AsRef<Path>) -> Result<Self> {
        let source_path = path.as_ref().to_path_buf();
        let object = open_metadata_object(&source_path)?;
        let sop_class_uid = object.meta().media_storage_sop_class_uid().to_string();
        if sop_class_uid != uids::VL_WHOLE_SLIDE_MICROSCOPY_IMAGE_STORAGE {
            return Err(Error::Unsupported(format!(
                "DICOM annotations require a VL Whole Slide Microscopy Image source, got SOP Class {sop_class_uid}"
            )));
        }
        let actual_sop_instance_uid = required_string(&object, tags::SOP_INSTANCE_UID)?;
        let sop_instance_uid =
            optional_string(&object, tags::SOP_INSTANCE_UID_OF_CONCATENATION_SOURCE)
                .unwrap_or(actual_sop_instance_uid);
        let total_pixel_matrix_columns = required_u32(&object, tags::TOTAL_PIXEL_MATRIX_COLUMNS)?;
        let total_pixel_matrix_rows = required_u32(&object, tags::TOTAL_PIXEL_MATRIX_ROWS)?;
        if total_pixel_matrix_columns == 0 || total_pixel_matrix_rows == 0 {
            return Err(Error::InvalidInput(
                "DICOM source has an empty Total Pixel Matrix".into(),
            ));
        }
        let tile_columns = required_u16(&object, tags::COLUMNS)?;
        let tile_rows = required_u16(&object, tags::ROWS)?;
        if tile_columns == 0 || tile_rows == 0 {
            return Err(Error::InvalidInput(
                "DICOM source has empty tile dimensions".into(),
            ));
        }
        let dimension_organization_type =
            optional_string(&object, tags::DIMENSION_ORGANIZATION_TYPE);
        let sparse_frame_origins =
            read_sparse_frame_origins(&object, dimension_organization_type.as_deref())?;
        Ok(Self {
            source_path,
            sop_class_uid,
            sop_instance_uid,
            series_instance_uid: required_string(&object, tags::SERIES_INSTANCE_UID)?,
            study_instance_uid: required_string(&object, tags::STUDY_INSTANCE_UID)?,
            frame_of_reference_uid: optional_string(&object, tags::FRAME_OF_REFERENCE_UID),
            total_pixel_matrix_columns,
            total_pixel_matrix_rows,
            tile_columns,
            tile_rows,
            pixel_spacing: optional_pair(&object, tags::PIXEL_SPACING)
                .or_else(|| shared_pixel_measures_pair(&object, tags::PIXEL_SPACING)),
            slice_thickness: optional_positive_float(&object, tags::SLICE_THICKNESS)
                .or_else(|| shared_pixel_measures_float(&object, tags::SLICE_THICKNESS)),
            image_orientation_slide: optional_six(&object, tags::IMAGE_ORIENTATION_SLIDE),
            total_pixel_matrix_origin: optional_origin(&object),
            dimension_organization_type,
            number_of_frames: optional_u32(&object, tags::NUMBER_OF_FRAMES),
            number_of_optical_paths: optional_u32(&object, tags::NUMBER_OF_OPTICAL_PATHS),
            total_pixel_matrix_focal_planes: optional_u32(
                &object,
                tags::TOTAL_PIXEL_MATRIX_FOCAL_PLANES,
            ),
            sparse_frame_origins,
        })
    }

    #[must_use]
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    #[must_use]
    pub fn sop_class_uid(&self) -> &str {
        &self.sop_class_uid
    }

    #[must_use]
    pub fn sop_instance_uid(&self) -> &str {
        &self.sop_instance_uid
    }

    #[must_use]
    pub fn series_instance_uid(&self) -> &str {
        &self.series_instance_uid
    }

    #[must_use]
    pub fn study_instance_uid(&self) -> &str {
        &self.study_instance_uid
    }

    #[must_use]
    pub fn frame_of_reference_uid(&self) -> Option<&str> {
        self.frame_of_reference_uid.as_deref()
    }

    #[must_use]
    pub const fn total_pixel_matrix_dimensions(&self) -> (u32, u32) {
        (
            self.total_pixel_matrix_columns,
            self.total_pixel_matrix_rows,
        )
    }

    #[must_use]
    pub const fn tile_dimensions(&self) -> (u16, u16) {
        (self.tile_columns, self.tile_rows)
    }

    #[must_use]
    pub const fn pixel_spacing(&self) -> Option<[f64; 2]> {
        self.pixel_spacing
    }

    #[must_use]
    pub const fn slice_thickness(&self) -> Option<f64> {
        self.slice_thickness
    }

    #[must_use]
    pub const fn image_orientation_slide(&self) -> Option<[f64; 6]> {
        self.image_orientation_slide
    }

    #[must_use]
    pub const fn total_pixel_matrix_origin(&self) -> Option<[f64; 3]> {
        self.total_pixel_matrix_origin
    }

    pub fn slide_coordinate_to_pixel(&self, x: f64, y: f64) -> Result<Point2> {
        let z = self.total_pixel_matrix_origin.ok_or_else(|| {
            Error::InvalidInput("WSI source has no Total Pixel Matrix Origin".into())
        })?[2];
        self.slide_coordinate_to_pixel3(x, y, z)
    }

    pub fn slide_coordinate_to_pixel3(&self, x: f64, y: f64, z: f64) -> Result<Point2> {
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Err(Error::InvalidInput(
                "slide annotation coordinates must be finite".into(),
            ));
        }
        let origin = self.total_pixel_matrix_origin.ok_or_else(|| {
            Error::InvalidInput("WSI source has no Total Pixel Matrix Origin".into())
        })?;
        let orientation = self.image_orientation_slide.ok_or_else(|| {
            Error::InvalidInput("WSI source has no Image Orientation (Slide)".into())
        })?;
        let spacing = self
            .pixel_spacing
            .ok_or_else(|| Error::InvalidInput("WSI source has no Pixel Spacing".into()))?;
        let column = [orientation[0], orientation[1], orientation[2]];
        let row = [orientation[3], orientation[4], orientation[5]];
        let delta = [x - origin[0], y - origin[1], z - origin[2]];
        let column_norm = dot(column, column);
        let row_norm = dot(row, row);
        let cross_term = dot(column, row);
        let determinant = column_norm * row_norm - cross_term * cross_term;
        if determinant.abs() <= f64::EPSILON {
            return Err(Error::InvalidInput(
                "WSI slide orientation cannot be inverted".into(),
            ));
        }
        let projected_column = dot(delta, column);
        let projected_row = dot(delta, row);
        let column_distance =
            (projected_column * row_norm - projected_row * cross_term) / determinant;
        let row_distance =
            (projected_row * column_norm - projected_column * cross_term) / determinant;
        Ok(Point2::new(
            column_distance / spacing[1],
            row_distance / spacing[0],
        ))
    }

    pub fn pixel_to_slide_coordinate(&self, x: f64, y: f64) -> Result<Point3> {
        if !x.is_finite() || !y.is_finite() {
            return Err(Error::InvalidInput(
                "pixel annotation coordinates must be finite".into(),
            ));
        }
        self.validate_point(x, y)?;
        let origin = self.total_pixel_matrix_origin.ok_or_else(|| {
            Error::InvalidInput("WSI source has no Total Pixel Matrix Origin".into())
        })?;
        let orientation = self.image_orientation_slide.ok_or_else(|| {
            Error::InvalidInput("WSI source has no Image Orientation (Slide)".into())
        })?;
        let spacing = self
            .pixel_spacing
            .ok_or_else(|| Error::InvalidInput("WSI source has no Pixel Spacing".into()))?;
        let column_distance = x * spacing[1];
        let row_distance = y * spacing[0];
        Ok(Point3::new(
            origin[0] + column_distance * orientation[0] + row_distance * orientation[3],
            origin[1] + column_distance * orientation[1] + row_distance * orientation[4],
            origin[2] + column_distance * orientation[2] + row_distance * orientation[5],
        ))
    }

    pub fn frame_coordinate_to_total_pixel(
        &self,
        frame_number: u32,
        x: f64,
        y: f64,
    ) -> Result<Point2> {
        if frame_number == 0
            || !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x > f64::from(self.tile_columns)
            || y > f64::from(self.tile_rows)
        {
            return Err(Error::InvalidInput(
                "FRAME-relative coordinate or frame number is outside its tile".into(),
            ));
        }
        let frame_count = self.number_of_frames.ok_or_else(|| {
            Error::InvalidInput("FRAME-relative source has no Number of Frames".into())
        })?;
        if frame_number > frame_count {
            return Err(Error::InvalidInput(format!(
                "referenced frame {frame_number} exceeds the source frame count"
            )));
        }
        let origin = match self.dimension_organization_type.as_deref() {
            Some("TILED_SPARSE") => self
                .sparse_frame_origins
                .as_ref()
                .and_then(|origins| origins.get((frame_number - 1) as usize))
                .copied()
                .ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "source has no plane position for referenced frame {frame_number}"
                    ))
                })?,
            Some("TILED_FULL") => self.tiled_full_frame_origin(frame_number)?,
            value => {
                return Err(Error::Unsupported(format!(
                    "FRAME-relative projection requires TILED_FULL or TILED_SPARSE source geometry, got {value:?}"
                )))
            }
        };
        let total = Point2::new(
            f64::from(origin.column - 1) + x,
            f64::from(origin.row - 1) + y,
        );
        self.validate_point(total.x, total.y)?;
        Ok(total)
    }

    fn tiled_full_frame_origin(&self, frame_number: u32) -> Result<FrameOrigin> {
        if self.number_of_optical_paths != Some(1)
            || self.total_pixel_matrix_focal_planes != Some(1)
        {
            return Err(Error::Unsupported(
                "TILED_FULL FRAME projection requires exactly one optical path and one focal plane"
                    .into(),
            ));
        }
        let tiles_across = self
            .total_pixel_matrix_columns
            .div_ceil(u32::from(self.tile_columns));
        let tiles_down = self
            .total_pixel_matrix_rows
            .div_ceil(u32::from(self.tile_rows));
        if self.number_of_frames != tiles_across.checked_mul(tiles_down) {
            return Err(Error::InvalidInput(
                "single-path TILED_FULL frame count does not match Total Pixel Matrix geometry"
                    .into(),
            ));
        }
        let frame_index = frame_number - 1;
        Ok(FrameOrigin {
            column: (frame_index % tiles_across) * u32::from(self.tile_columns) + 1,
            row: (frame_index / tiles_across) * u32::from(self.tile_rows) + 1,
        })
    }

    pub(crate) fn source_metadata(&self) -> Result<DefaultDicomObject> {
        open_metadata_object(&self.source_path)
    }

    pub(crate) fn require_shared_slide_frame(&self, other: &Self) -> Result<()> {
        if self.study_instance_uid != other.study_instance_uid {
            return Err(Error::InvalidInput(
                "source and canonical source do not share a Study Instance UID".into(),
            ));
        }
        match (
            self.frame_of_reference_uid.as_deref(),
            other.frame_of_reference_uid.as_deref(),
        ) {
            (Some(left), Some(right)) if left == right => Ok(()),
            _ => Err(Error::InvalidInput(
                "source and canonical source do not share a Frame of Reference UID".into(),
            )),
        }
    }

    pub(crate) fn validate_point(&self, x: f64, y: f64) -> Result<()> {
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x > f64::from(self.total_pixel_matrix_columns)
            || y > f64::from(self.total_pixel_matrix_rows)
        {
            return Err(Error::InvalidInput(format!(
                "annotation coordinate ({x}, {y}) is outside the {} × {} Total Pixel Matrix",
                self.total_pixel_matrix_columns, self.total_pixel_matrix_rows
            )));
        }
        Ok(())
    }
}

fn optional_pair(object: &DefaultDicomObject, tag: Tag) -> Option<[f64; 2]> {
    let values = object.get(tag)?.to_multi_float64().ok()?;
    (values.len() == 2 && values.iter().all(|value| value.is_finite() && *value > 0.0))
        .then(|| [values[0], values[1]])
}

fn optional_positive_float(object: &DefaultDicomObject, tag: Tag) -> Option<f64> {
    object
        .get(tag)?
        .to_float64()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn shared_pixel_measures_pair(object: &DefaultDicomObject, tag: Tag) -> Option<[f64; 2]> {
    let values = shared_pixel_measures(object)?
        .get(tag)?
        .to_multi_float64()
        .ok()?;
    (values.len() == 2 && values.iter().all(|value| value.is_finite() && *value > 0.0))
        .then(|| [values[0], values[1]])
}

fn shared_pixel_measures_float(object: &DefaultDicomObject, tag: Tag) -> Option<f64> {
    shared_pixel_measures(object)?
        .get(tag)?
        .to_float64()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn shared_pixel_measures(object: &DefaultDicomObject) -> Option<&dicom_object::InMemDicomObject> {
    object
        .get(tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE)?
        .items()?
        .first()?
        .get(tags::PIXEL_MEASURES_SEQUENCE)?
        .items()?
        .first()
}

fn optional_six(object: &DefaultDicomObject, tag: Tag) -> Option<[f64; 6]> {
    let values = object.get(tag)?.to_multi_float64().ok()?;
    (values.len() == 6 && values.iter().all(|value| value.is_finite())).then(|| {
        [
            values[0], values[1], values[2], values[3], values[4], values[5],
        ]
    })
}

fn optional_origin(object: &DefaultDicomObject) -> Option<[f64; 3]> {
    let item = object
        .get(tags::TOTAL_PIXEL_MATRIX_ORIGIN_SEQUENCE)?
        .items()?
        .first()?;
    let x = item
        .get(tags::X_OFFSET_IN_SLIDE_COORDINATE_SYSTEM)?
        .to_float64()
        .ok()?;
    let y = item
        .get(tags::Y_OFFSET_IN_SLIDE_COORDINATE_SYSTEM)?
        .to_float64()
        .ok()?;
    let z = item
        .get(tags::Z_OFFSET_IN_SLIDE_COORDINATE_SYSTEM)
        .and_then(|element| element.to_float64().ok())
        .unwrap_or(0.0);
    [x, y, z]
        .iter()
        .all(|value| value.is_finite())
        .then_some([x, y, z])
}

fn read_sparse_frame_origins(
    object: &DefaultDicomObject,
    dimension_organization_type: Option<&str>,
) -> Result<Option<Vec<FrameOrigin>>> {
    if dimension_organization_type != Some("TILED_SPARSE") {
        return Ok(None);
    }
    let items = object
        .get(tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE)
        .and_then(|element| element.items())
        .ok_or_else(|| {
            Error::InvalidInput(
                "TILED_SPARSE source has no Per-frame Functional Groups Sequence".into(),
            )
        })?;
    if optional_u32(object, tags::NUMBER_OF_FRAMES) != u32::try_from(items.len()).ok() {
        return Err(Error::InvalidInput(
            "TILED_SPARSE source frame count does not match its per-frame metadata".into(),
        ));
    }
    items
        .iter()
        .map(|item| {
            let planes = item
                .get(tags::PLANE_POSITION_SLIDE_SEQUENCE)
                .and_then(|element| element.items())
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "TILED_SPARSE source frame has no Plane Position (Slide)".into(),
                    )
                })?;
            if planes.len() != 1 {
                return Err(Error::InvalidInput(
                    "source frame must contain exactly one Plane Position (Slide) item".into(),
                ));
            }
            let column = planes[0]
                .get(tags::COLUMN_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX)
                .and_then(|element| element.to_int::<u32>().ok())
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "source frame has no valid Total Pixel Matrix column position".into(),
                    )
                })?;
            let row = planes[0]
                .get(tags::ROW_POSITION_IN_TOTAL_IMAGE_PIXEL_MATRIX)
                .and_then(|element| element.to_int::<u32>().ok())
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "source frame has no valid Total Pixel Matrix row position".into(),
                    )
                })?;
            if column == 0 || row == 0 {
                return Err(Error::InvalidInput(
                    "source frame Total Pixel Matrix positions are one-based".into(),
                ));
            }
            Ok(FrameOrigin { column, row })
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
