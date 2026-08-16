use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use crate::{Error, Result};

use super::super::model::{
    clockwise_polygon, validate_polygon, AnnotationGeometry, AnnotationGraphicType, Point2,
};

pub(super) fn encoded_geometry(
    geometry: &AnnotationGeometry,
) -> Result<(Vec<f64>, Option<Vec<u32>>)> {
    match geometry {
        AnnotationGeometry::Points(points) => Ok((
            points.iter().flat_map(|point| [point.x, point.y]).collect(),
            None,
        )),
        AnnotationGeometry::Polygons(polygons) => {
            let mut coordinates = Vec::new();
            let mut indices = Vec::with_capacity(polygons.len());
            for polygon in polygons {
                indices.push(u32::try_from(coordinates.len() + 1).map_err(|_| {
                    Error::InvalidInput("ANN coordinate index exceeds DICOM OL range".into())
                })?);
                coordinates.extend(clockwise_polygon(polygon).flat_map(|point| [point.x, point.y]));
            }
            Ok((coordinates, Some(indices)))
        }
        AnnotationGeometry::ReadOnly {
            coordinates,
            primitive_point_indices,
            ..
        } => Ok((
            coordinates.clone(),
            (!primitive_point_indices.is_empty()).then(|| primitive_point_indices.clone()),
        )),
    }
}

pub(super) fn validate_encoded_geometry(
    item: &InMemDicomObject,
    graphic_type: AnnotationGraphicType,
    coordinates: &[f64],
    indices: &[u32],
    dimensions: usize,
) -> Result<()> {
    let point_count = coordinates.len() / dimensions;
    let annotation_count = match graphic_type {
        AnnotationGraphicType::Point => {
            if !indices.is_empty() {
                return Err(Error::InvalidInput(
                    "ANN point groups cannot contain primitive indices".into(),
                ));
            }
            point_count
        }
        AnnotationGraphicType::Polygon | AnnotationGraphicType::Polyline => {
            if indices.is_empty()
                || indices[0] != 1
                || indices.windows(2).any(|pair| pair[0] >= pair[1])
                || indices
                    .iter()
                    .any(|index| primitive_point_offset(*index, dimensions).is_none())
            {
                return Err(Error::InvalidInput(
                    "ANN primitive indices must start at one, align to coordinate tuples, and increase strictly"
                        .into(),
                ));
            }
            for (position, index) in indices.iter().enumerate() {
                let start = primitive_point_offset(*index, dimensions)
                    .ok_or_else(|| Error::InvalidInput("invalid ANN primitive index".into()))?;
                let end = indices.get(position + 1).map_or(point_count, |next| {
                    primitive_point_offset(*next, dimensions).unwrap_or(point_count)
                });
                let minimum = if graphic_type == AnnotationGraphicType::Polygon {
                    3
                } else {
                    2
                };
                if start >= end || end > point_count || end - start < minimum {
                    return Err(Error::InvalidInput(format!(
                        "ANN {} primitive has fewer than {minimum} points or exceeds coordinate data",
                        graphic_type.dicom_value()
                    )));
                }
            }
            indices.len()
        }
        AnnotationGraphicType::Ellipse | AnnotationGraphicType::Rectangle => {
            if !indices.is_empty() || !point_count.is_multiple_of(4) {
                return Err(Error::InvalidInput(format!(
                    "ANN {} groups require four points per annotation and no primitive indices",
                    graphic_type.dicom_value()
                )));
            }
            point_count / 4
        }
    };
    let declared = item
        .get(tags::NUMBER_OF_ANNOTATIONS)
        .and_then(|element| element.to_int::<usize>().ok())
        .ok_or_else(|| {
            Error::InvalidInput("ANN group has no valid Number of Annotations".into())
        })?;
    if annotation_count == 0 || declared != annotation_count {
        return Err(Error::InvalidInput(format!(
            "ANN declares {declared} annotations but its geometry encodes {annotation_count}"
        )));
    }
    Ok(())
}

pub(super) fn primitive_point_offset(index: u32, dimensions: usize) -> Option<usize> {
    let value_offset = usize::try_from(index).ok()?.checked_sub(1)?;
    value_offset
        .is_multiple_of(dimensions)
        .then_some(value_offset / dimensions)
}

pub(super) fn decode_polygons(coordinates: &[f64], indices: &[u32]) -> Result<Vec<Vec<Point2>>> {
    if indices.is_empty() {
        return Err(Error::InvalidInput(
            "ANN polygon group has no primitive point index list".into(),
        ));
    }
    let mut polygons = Vec::with_capacity(indices.len());
    for (position, &index) in indices.iter().enumerate() {
        let start = usize::try_from(index.saturating_sub(1)).unwrap_or(usize::MAX);
        let end = indices.get(position + 1).map_or(coordinates.len(), |next| {
            usize::try_from(next.saturating_sub(1)).unwrap_or(usize::MAX)
        });
        if index == 0
            || start >= end
            || end > coordinates.len()
            || !start.is_multiple_of(2)
            || !end.is_multiple_of(2)
        {
            return Err(Error::InvalidInput(
                "ANN polygon primitive indices are invalid".into(),
            ));
        }
        let polygon = coordinates[start..end]
            .chunks_exact(2)
            .map(|point| Point2::new(point[0], point[1]))
            .collect::<Vec<_>>();
        validate_polygon(&polygon)?;
        polygons.push(polygon);
    }
    Ok(polygons)
}
