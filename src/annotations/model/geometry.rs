use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const MIN_POLYGON_AREA: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

impl Point2 {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationGraphicType {
    Point,
    Polygon,
    Polyline,
    Ellipse,
    Rectangle,
}

impl AnnotationGraphicType {
    #[must_use]
    pub const fn dicom_value(self) -> &'static str {
        match self {
            Self::Point => "POINT",
            Self::Polygon => "POLYGON",
            Self::Polyline => "POLYLINE",
            Self::Ellipse => "ELLIPSE",
            Self::Rectangle => "RECTANGLE",
        }
    }

    pub(crate) fn from_dicom(value: &str) -> Result<Self> {
        match value.trim() {
            "POINT" => Ok(Self::Point),
            "POLYGON" => Ok(Self::Polygon),
            "POLYLINE" => Ok(Self::Polyline),
            "ELLIPSE" => Ok(Self::Ellipse),
            "RECTANGLE" => Ok(Self::Rectangle),
            other => Err(Error::Unsupported(format!(
                "ANN graphic type {other:?} is not recognized"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationGeometry {
    Points(Vec<Point2>),
    Polygons(Vec<Vec<Point2>>),
    ReadOnly {
        graphic_type: AnnotationGraphicType,
        coordinates: Vec<f64>,
        primitive_point_indices: Vec<u32>,
        coordinate_dimensions: usize,
    },
}

impl AnnotationGeometry {
    #[must_use]
    pub const fn graphic_type(&self) -> AnnotationGraphicType {
        match self {
            Self::Points(_) => AnnotationGraphicType::Point,
            Self::Polygons(_) => AnnotationGraphicType::Polygon,
            Self::ReadOnly { graphic_type, .. } => *graphic_type,
        }
    }

    #[must_use]
    pub const fn editable(&self) -> bool {
        matches!(self, Self::Points(_) | Self::Polygons(_))
    }

    pub(crate) fn annotation_count(&self) -> usize {
        match self {
            Self::Points(points) => points.len(),
            Self::Polygons(polygons) => polygons.len(),
            Self::ReadOnly {
                graphic_type,
                coordinates,
                primitive_point_indices,
                coordinate_dimensions,
            } => match graphic_type {
                AnnotationGraphicType::Point => coordinates.len() / coordinate_dimensions,
                AnnotationGraphicType::Polygon | AnnotationGraphicType::Polyline => {
                    primitive_point_indices.len()
                }
                AnnotationGraphicType::Ellipse | AnnotationGraphicType::Rectangle => {
                    coordinates.len() / (coordinate_dimensions * 4)
                }
            },
        }
    }
}

pub(super) fn validate_points(points: impl Iterator<Item = Point2>) -> Result<()> {
    if points
        .into_iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return Err(Error::InvalidInput(
            "annotation coordinates must be finite".into(),
        ));
    }
    Ok(())
}

/// Validates a finite, non-self-intersecting, implicitly closed polygon ring.
pub fn validate_polygon(points: &[Point2]) -> Result<()> {
    if points.len() < 3 {
        return Err(Error::InvalidInput(
            "an annotation polygon needs at least three vertices".into(),
        ));
    }
    validate_points(points.iter().copied())?;
    if points.first() == points.last() {
        return Err(Error::InvalidInput(
            "ANN polygons close implicitly and must not repeat the first vertex".into(),
        ));
    }
    if polygon_signed_area(points).abs() < MIN_POLYGON_AREA {
        return Err(Error::InvalidInput(
            "an annotation polygon has zero area".into(),
        ));
    }
    if polygon_self_intersects(points) {
        return Err(Error::InvalidInput(
            "an annotation polygon crosses itself".into(),
        ));
    }
    Ok(())
}

pub(crate) fn clockwise_polygon(points: &[Point2]) -> impl Iterator<Item = Point2> + '_ {
    let reverse = polygon_signed_area(points) < 0.0;
    (0..points.len()).map(move |index| {
        let index = if reverse {
            points.len() - 1 - index
        } else {
            index
        };
        points[index]
    })
}

/// Reports whether `point` is inside `polygon` using an even-odd ray cast.
///
/// Boundary points are not assigned special semantics. Callers that require
/// strict containment must separately reject boundary intersections.
#[must_use]
pub fn polygon_contains_point(polygon: &[Point2], point: Point2) -> bool {
    let mut inside = false;
    for (a, b) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        if (a.y > point.y) != (b.y > point.y) {
            let crossing_x = (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
            if point.x < crossing_x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Reports whether any edges from two implicitly closed polygons intersect.
#[must_use]
pub fn polygon_boundaries_intersect(first: &[Point2], second: &[Point2]) -> bool {
    first
        .iter()
        .zip(first.iter().cycle().skip(1))
        .take(first.len())
        .any(|(first_start, first_end)| {
            second
                .iter()
                .zip(second.iter().cycle().skip(1))
                .take(second.len())
                .any(|(second_start, second_end)| {
                    segments_intersect(*first_start, *first_end, *second_start, *second_end)
                })
        })
}

/// Returns the signed area of an implicitly closed polygon.
///
/// Positive values use the coordinate system's counterclockwise winding.
#[must_use]
pub fn polygon_signed_area(points: &[Point2]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        * 0.5
}

/// Reports whether non-adjacent edges of an implicitly closed polygon intersect.
#[must_use]
pub fn polygon_self_intersects(points: &[Point2]) -> bool {
    for first in 0..points.len() {
        let first_next = (first + 1) % points.len();
        for second in (first + 1)..points.len() {
            let second_next = (second + 1) % points.len();
            if first_next == second || second_next == first {
                continue;
            }
            if segments_intersect(
                points[first],
                points[first_next],
                points[second],
                points[second_next],
            ) {
                return true;
            }
        }
    }
    false
}

fn segments_intersect(a1: Point2, a2: Point2, b1: Point2, b2: Point2) -> bool {
    let o1 = orientation(a1, a2, b1);
    let o2 = orientation(a1, a2, b2);
    let o3 = orientation(b1, b2, a1);
    let o4 = orientation(b1, b2, a2);
    if ((o1 > 0.0 && o2 < 0.0) || (o1 < 0.0 && o2 > 0.0))
        && ((o3 > 0.0 && o4 < 0.0) || (o3 < 0.0 && o4 > 0.0))
    {
        return true;
    }
    (o1.abs() <= f64::EPSILON && point_on_segment(b1, a1, a2))
        || (o2.abs() <= f64::EPSILON && point_on_segment(b2, a1, a2))
        || (o3.abs() <= f64::EPSILON && point_on_segment(a1, b1, b2))
        || (o4.abs() <= f64::EPSILON && point_on_segment(a2, b1, b2))
}

fn point_on_segment(point: Point2, a: Point2, b: Point2) -> bool {
    point.x >= a.x.min(b.x)
        && point.x <= a.x.max(b.x)
        && point.y >= a.y.min(b.y)
        && point.y <= a.y.max(b.y)
}

fn orientation(a: Point2, b: Point2, c: Point2) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}
