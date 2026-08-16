use super::feature::ProfiledFeature;
use super::geometry::ProfiledGeometry;
use crate::Point2;

#[derive(Debug, Clone)]
pub struct PathologyPreview {
    features: Vec<PathologyPreviewFeature>,
}

impl PathologyPreview {
    pub(super) fn from_owned_features(features: Vec<ProfiledFeature>) -> Self {
        Self {
            features: features
                .into_iter()
                .map(PathologyPreviewFeature::from)
                .collect(),
        }
    }

    #[must_use]
    pub fn features(&self) -> &[PathologyPreviewFeature] {
        &self.features
    }
}

#[derive(Debug, Clone)]
pub struct PathologyPreviewFeature {
    tracking_id: String,
    label: String,
    classification: String,
    recommended_display_cielab: [u16; 3],
    bounds: [f64; 4],
    geometry: PathologyPreviewGeometry,
}

impl PathologyPreviewFeature {
    #[must_use]
    pub fn tracking_id(&self) -> &str {
        &self.tracking_id
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn classification(&self) -> &str {
        &self.classification
    }

    #[must_use]
    pub const fn recommended_display_cielab(&self) -> [u16; 3] {
        self.recommended_display_cielab
    }

    #[must_use]
    pub const fn bounds(&self) -> [f64; 4] {
        self.bounds
    }

    #[must_use]
    pub const fn geometry(&self) -> &PathologyPreviewGeometry {
        &self.geometry
    }
}

impl From<ProfiledFeature> for PathologyPreviewFeature {
    fn from(feature: ProfiledFeature) -> Self {
        let bounds = geometry_bounds(&feature.geometry);
        let geometry = match feature.geometry {
            ProfiledGeometry::Points(points) => PathologyPreviewGeometry::Points(points),
            ProfiledGeometry::Lines(lines) => PathologyPreviewGeometry::Lines(lines),
            ProfiledGeometry::Polygons(polygons) => PathologyPreviewGeometry::Polygons(
                polygons
                    .into_iter()
                    .map(|polygon| PathologyPreviewPolygon {
                        exterior: polygon.outer,
                        holes: polygon.holes,
                    })
                    .collect(),
            ),
        };
        Self {
            tracking_id: feature.tracking_id,
            label: feature.name,
            classification: feature.classification,
            recommended_display_cielab: feature.semantics.color,
            bounds,
            geometry,
        }
    }
}

fn geometry_bounds(geometry: &ProfiledGeometry) -> [f64; 4] {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut observe = |point: &Point2| {
        bounds[0] = bounds[0].min(point.x);
        bounds[1] = bounds[1].min(point.y);
        bounds[2] = bounds[2].max(point.x);
        bounds[3] = bounds[3].max(point.y);
    };
    match geometry {
        ProfiledGeometry::Points(points) => points.iter().for_each(&mut observe),
        ProfiledGeometry::Lines(lines) => lines.iter().flatten().for_each(&mut observe),
        ProfiledGeometry::Polygons(polygons) => polygons.iter().for_each(|polygon| {
            polygon.outer.iter().for_each(&mut observe);
            polygon.holes.iter().flatten().for_each(&mut observe);
        }),
    }
    bounds
}

#[derive(Debug, Clone)]
pub enum PathologyPreviewGeometry {
    Points(Vec<Point2>),
    Lines(Vec<Vec<Point2>>),
    Polygons(Vec<PathologyPreviewPolygon>),
}

#[derive(Debug, Clone)]
pub struct PathologyPreviewPolygon {
    exterior: Vec<Point2>,
    holes: Vec<Vec<Point2>>,
}

impl PathologyPreviewPolygon {
    #[must_use]
    pub fn exterior(&self) -> &[Point2] {
        &self.exterior
    }

    #[must_use]
    pub fn holes(&self) -> &[Vec<Point2>] {
        &self.holes
    }
}
