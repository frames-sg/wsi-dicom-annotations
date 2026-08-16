use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

use super::{PathologyCoordinateSpace, PathologyGeometryKind};
use crate::annotations::context::DicomAnnotationContext;
use crate::annotations::model::{
    clockwise_polygon, polygon_boundaries_intersect, polygon_contains_point, validate_polygon,
    InteroperabilityDiagnostic, Point2,
};

const MAX_COORDINATE_PAIRS: usize = 32_000_000;

#[derive(Debug)]
pub(super) enum ProfiledGeometry {
    Points(Vec<Point2>),
    Lines(Vec<Vec<Point2>>),
    Polygons(Vec<ProfiledPolygon>),
}

#[derive(Debug)]
pub(super) struct ProfiledPolygon {
    pub(super) outer: Vec<Point2>,
    pub(super) holes: Vec<Vec<Point2>>,
}

impl ProfiledGeometry {
    pub(super) fn update_digest(&self, digest: &mut Sha256) {
        match self {
            Self::Points(points) => update_points_digest(digest, points),
            Self::Lines(lines) => {
                digest.update((lines.len() as u64).to_le_bytes());
                for line in lines {
                    update_points_digest(digest, line);
                }
            }
            Self::Polygons(polygons) => {
                digest.update((polygons.len() as u64).to_le_bytes());
                for polygon in polygons {
                    update_points_digest(digest, &polygon.outer);
                    digest.update((polygon.holes.len() as u64).to_le_bytes());
                    for hole in &polygon.holes {
                        update_points_digest(digest, hole);
                    }
                }
            }
        }
    }
}

fn update_points_digest(digest: &mut Sha256, points: &[Point2]) {
    digest.update((points.len() as u64).to_le_bytes());
    for point in points {
        digest.update(point.x.to_bits().to_le_bytes());
        digest.update(point.y.to_bits().to_le_bytes());
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn profile_geometry(
    raw: RawGeometry,
    path: &str,
    source: &DicomAnnotationContext,
    canonical_source: &DicomAnnotationContext,
    coordinate_space: PathologyCoordinateSpace,
    coordinate_pairs: &mut usize,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<(PathologyGeometryKind, ProfiledGeometry)> {
    let mut transform = |position, position_path: &str| {
        transform_position(
            position,
            position_path,
            source,
            canonical_source,
            coordinate_space,
            coordinate_pairs,
        )
    };
    match raw {
        RawGeometry::Point { coordinates } => Ok((
            PathologyGeometryKind::Point,
            ProfiledGeometry::Points(vec![transform(coordinates, path)?]),
        )),
        RawGeometry::MultiPoint { coordinates } => {
            if coordinates.is_empty() {
                return Err(empty_geometry(path));
            }
            let points = coordinates
                .into_iter()
                .enumerate()
                .map(|(index, position)| transform(position, &format!("{path}[{index}]")))
                .collect::<Result<_>>()?;
            Ok((
                PathologyGeometryKind::MultiPoint,
                ProfiledGeometry::Points(points),
            ))
        }
        RawGeometry::LineString { coordinates } => Ok((
            PathologyGeometryKind::LineString,
            ProfiledGeometry::Lines(vec![profile_line(coordinates, path, &mut transform)?]),
        )),
        RawGeometry::MultiLineString { coordinates } => {
            if coordinates.is_empty() {
                return Err(empty_geometry(path));
            }
            let mut lines = Vec::with_capacity(coordinates.len());
            for (index, line) in coordinates.into_iter().enumerate() {
                lines.push(profile_line(
                    line,
                    &format!("{path}[{index}]"),
                    &mut transform,
                )?);
            }
            Ok((
                PathologyGeometryKind::MultiLineString,
                ProfiledGeometry::Lines(lines),
            ))
        }
        RawGeometry::Polygon { coordinates } => Ok((
            PathologyGeometryKind::Polygon,
            ProfiledGeometry::Polygons(vec![profile_polygon(
                coordinates,
                path,
                &mut transform,
                diagnostics,
            )?]),
        )),
        RawGeometry::MultiPolygon { coordinates } => {
            if coordinates.is_empty() {
                return Err(empty_geometry(path));
            }
            let mut polygons = Vec::with_capacity(coordinates.len());
            for (index, polygon) in coordinates.into_iter().enumerate() {
                polygons.push(profile_polygon(
                    polygon,
                    &format!("{path}[{index}]"),
                    &mut transform,
                    diagnostics,
                )?);
            }
            validate_multipolygon(&polygons, path)?;
            Ok((
                PathologyGeometryKind::MultiPolygon,
                ProfiledGeometry::Polygons(polygons),
            ))
        }
        RawGeometry::GeometryCollection { .. } => Err(Error::Unsupported(format!(
            "{path} uses GeometryCollection, which pathology-geojson-v1 does not support"
        ))),
    }
}

fn profile_line(
    raw: Vec<Vec<f64>>,
    path: &str,
    transform: &mut impl FnMut(Vec<f64>, &str) -> Result<Point2>,
) -> Result<Vec<Point2>> {
    if raw.len() < 2 {
        return Err(Error::InvalidInput(format!(
            "{path} line needs at least two positions"
        )));
    }
    let points = raw
        .into_iter()
        .enumerate()
        .map(|(index, position)| transform(position, &format!("{path}[{index}]")))
        .collect::<Result<Vec<_>>>()?;
    if points.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(Error::InvalidInput(format!(
            "{path} line has consecutive duplicate positions"
        )));
    }
    Ok(points)
}

fn profile_polygon(
    raw: Vec<Vec<Vec<f64>>>,
    path: &str,
    transform: &mut impl FnMut(Vec<f64>, &str) -> Result<Point2>,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<ProfiledPolygon> {
    if raw.is_empty() {
        return Err(empty_geometry(path));
    }
    let mut rings = raw
        .into_iter()
        .enumerate()
        .map(|(index, ring)| {
            profile_ring(
                ring,
                &format!("{path}[{index}]"),
                index > 0,
                transform,
                diagnostics,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let outer = rings.remove(0);
    let polygon = ProfiledPolygon {
        outer,
        holes: rings,
    };
    validate_polygon_holes(&polygon, path)?;
    Ok(polygon)
}

fn validate_polygon_holes(polygon: &ProfiledPolygon, path: &str) -> Result<()> {
    for (index, hole) in polygon.holes.iter().enumerate() {
        if polygon_boundaries_intersect(&polygon.outer, hole)
            || !polygon_contains_point(&polygon.outer, hole[0])
        {
            return Err(Error::InvalidInput(format!(
                "{path}[{}] has invalid polygon hole topology",
                index + 1
            )));
        }
        for previous in &polygon.holes[..index] {
            if polygon_boundaries_intersect(previous, hole)
                || polygon_contains_point(previous, hole[0])
                || polygon_contains_point(hole, previous[0])
            {
                return Err(Error::InvalidInput(format!(
                    "{path}[{}] has invalid polygon hole topology",
                    index + 1
                )));
            }
        }
    }
    Ok(())
}

fn validate_multipolygon(polygons: &[ProfiledPolygon], path: &str) -> Result<()> {
    for (index, polygon) in polygons.iter().enumerate() {
        for previous in &polygons[..index] {
            if polygon_regions_overlap(previous, polygon) {
                return Err(Error::InvalidInput(format!(
                    "{path}[{index}] has invalid MultiPolygon component topology"
                )));
            }
        }
    }
    Ok(())
}

fn polygon_regions_overlap(first: &ProfiledPolygon, second: &ProfiledPolygon) -> bool {
    let first_rings =
        std::iter::once(first.outer.as_slice()).chain(first.holes.iter().map(Vec::as_slice));
    let second_rings =
        || std::iter::once(second.outer.as_slice()).chain(second.holes.iter().map(Vec::as_slice));
    first_rings.into_iter().any(|first_ring| {
        second_rings().any(|second_ring| polygon_boundaries_intersect(first_ring, second_ring))
    }) || polygon_region_contains(first, second.outer[0])
        || polygon_region_contains(second, first.outer[0])
}

fn polygon_region_contains(polygon: &ProfiledPolygon, point: Point2) -> bool {
    polygon_contains_point(&polygon.outer, point)
        && !polygon
            .holes
            .iter()
            .any(|hole| polygon_contains_point(hole, point))
}

fn profile_ring(
    mut raw: Vec<Vec<f64>>,
    path: &str,
    is_hole: bool,
    transform: &mut impl FnMut(Vec<f64>, &str) -> Result<Point2>,
    diagnostics: &mut Vec<InteroperabilityDiagnostic>,
) -> Result<Vec<Point2>> {
    if raw.is_empty() || raw.first() != raw.last() {
        return Err(Error::InvalidInput(format!(
            "{path} GeoJSON linear ring must repeat its first position at the end"
        )));
    }
    raw.pop();
    diagnostics.push(InteroperabilityDiagnostic::normalized(
        "GEOJSON_RING_CLOSURE_REMOVED",
        path,
        "removed GeoJSON's repeated closing position for DICOM implicit closure",
    ));
    let points = raw
        .into_iter()
        .enumerate()
        .map(|(index, position)| transform(position, &format!("{path}[{index}]")))
        .collect::<Result<Vec<_>>>()?;
    validate_polygon(&points).map_err(|error| {
        Error::InvalidInput(format!("{path} is not a valid polygon ring: {error}"))
    })?;
    let mut normalized_points = clockwise_polygon(&points).collect::<Vec<_>>();
    if is_hole {
        normalized_points.reverse();
    }
    if normalized_points != points {
        diagnostics.push(InteroperabilityDiagnostic::normalized(
            "GEOJSON_WINDING_NORMALIZED",
            path,
            if is_hole {
                "normalized polygon hole to counterclockwise winding"
            } else {
                "normalized polygon exterior to DICOM clockwise winding"
            },
        ));
    }
    Ok(normalized_points)
}

fn transform_position(
    raw: Vec<f64>,
    path: &str,
    source: &DicomAnnotationContext,
    canonical_source: &DicomAnnotationContext,
    coordinate_space: PathologyCoordinateSpace,
    coordinate_pairs: &mut usize,
) -> Result<Point2> {
    if raw.len() != 2 || raw.iter().any(|coordinate| !coordinate.is_finite()) {
        return Err(Error::InvalidInput(format!(
            "{path} must contain exactly two finite coordinates in [x,y] order"
        )));
    }
    *coordinate_pairs = coordinate_pairs
        .checked_add(1)
        .ok_or_else(|| Error::InvalidInput("GeoJSON coordinate count overflows".into()))?;
    if *coordinate_pairs > MAX_COORDINATE_PAIRS {
        return Err(Error::InvalidInput(format!(
            "GeoJSON exceeds the {MAX_COORDINATE_PAIRS}-coordinate-pair limit"
        )));
    }
    let point = match coordinate_space {
        PathologyCoordinateSpace::SourcePixels => Point2::new(raw[0], raw[1]),
        PathologyCoordinateSpace::SlideMillimeters => {
            source.slide_coordinate_to_pixel(raw[0], raw[1])?
        }
        PathologyCoordinateSpace::Level0Pixels
            if source.sop_instance_uid() == canonical_source.sop_instance_uid() =>
        {
            Point2::new(raw[0], raw[1])
        }
        PathologyCoordinateSpace::Level0Pixels => {
            let slide = canonical_source.pixel_to_slide_coordinate(raw[0], raw[1])?;
            source.slide_coordinate_to_pixel3(slide.x, slide.y, slide.z)?
        }
    };
    source
        .validate_point(point.x, point.y)
        .map_err(|error| Error::InvalidInput(format!("{path}: {error}")))?;
    Ok(point)
}

fn empty_geometry(path: &str) -> Error {
    Error::InvalidInput(format!("{path} must not be empty"))
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(super) enum RawGeometry {
    Point {
        coordinates: Vec<f64>,
    },
    MultiPoint {
        coordinates: Vec<Vec<f64>>,
    },
    LineString {
        coordinates: Vec<Vec<f64>>,
    },
    MultiLineString {
        coordinates: Vec<Vec<Vec<f64>>>,
    },
    Polygon {
        coordinates: Vec<Vec<Vec<f64>>>,
    },
    MultiPolygon {
        coordinates: Vec<Vec<Vec<Vec<f64>>>>,
    },
    GeometryCollection {
        #[serde(rename = "geometries")]
        _geometries: Vec<Value>,
    },
}
