use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

use super::{
    default_tile_length, normalize_tile, validate_region, NpySource, PixelTile, RasterDType,
    RasterDescriptor, RasterProfile, RasterSource, RawTile, SourceNormalization, TiffSource,
};
use crate::annotations::json::validate_unique_object_keys;
use crate::annotations::parametric_map::profile::RasterOutputPrecision;

const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MANIFEST_TILES: usize = 100_000;

pub(super) struct TiledManifestSource {
    descriptor: RasterDescriptor,
    tiles: Vec<ManifestTile>,
    overlap_policy: OverlapPolicy,
    scaling: Option<super::IntegerScaling>,
    precision: RasterOutputPrecision,
    normalizations: Vec<SourceNormalization>,
}

struct ManifestTile {
    source: Box<dyn RasterSource>,
    origin_y: u32,
    origin_x: u32,
    effective: Region,
}

#[derive(Debug, Clone, Copy)]
struct Region {
    y: u32,
    x: u32,
    height: u32,
    width: u32,
}

impl Region {
    fn bottom(self) -> u32 {
        self.y + self.height
    }

    fn right(self) -> u32 {
        self.x + self.width
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let y = self.y.max(other.y);
        let x = self.x.max(other.x);
        let bottom = self.bottom().min(other.bottom());
        let right = self.right().min(other.right());
        (y < bottom && x < right).then_some(Self {
            y,
            x,
            height: bottom - y,
            width: right - x,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TileFormat {
    Tiff,
    Npy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum OverlapPolicy {
    Reject,
    ValidRegionCrop,
    Mean,
    Max,
    ManifestOrderLastWrite,
}

fn default_overlap_policy() -> OverlapPolicy {
    OverlapPolicy::Reject
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    schema_version: u32,
    #[serde(default = "default_overlap_policy")]
    overlap_policy: OverlapPolicy,
    tiles: Vec<RawTileEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTileEntry {
    path: String,
    format: TileFormat,
    sample_grid_origin: SampleGridOrigin,
    #[serde(default)]
    valid_region: Option<ValidRegion>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct SampleGridOrigin {
    y: u32,
    x: u32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidRegion {
    y: u32,
    x: u32,
    height: u32,
    width: u32,
}

impl TiledManifestSource {
    pub(super) fn open(profile: &RasterProfile, path: &Path) -> Result<Self> {
        let manifest = read_manifest(path)?;
        if manifest.schema_version != 1 {
            return Err(Error::Unsupported(format!(
                "tiled manifest schema version {} is not supported",
                manifest.schema_version
            )));
        }
        if manifest.tiles.is_empty() || manifest.tiles.len() > MAX_MANIFEST_TILES {
            return Err(Error::InvalidInput(format!(
                "tiled manifest must contain 1..={MAX_MANIFEST_TILES} tiles"
            )));
        }
        let manifest_parent = path
            .parent()
            .ok_or_else(|| Error::InvalidInput("tiled manifest has no parent directory".into()))?;
        let canonical_parent =
            std::fs::canonicalize(manifest_parent).map_err(|source| Error::Io {
                path: manifest_parent.to_path_buf(),
                source,
            })?;
        let mut tiles = Vec::with_capacity(manifest.tiles.len());
        let mut height = 0_u32;
        let mut width = 0_u32;
        let mut tile_height = u32::MAX;
        let mut tile_width = u32::MAX;
        let mut normalizations = BTreeSet::new();
        for raw in manifest.tiles {
            let tile_path = resolve_tile_path(&canonical_parent, &raw.path)?;
            let source: Box<dyn RasterSource> = match raw.format {
                TileFormat::Tiff => Box::new(TiffSource::open(profile, &tile_path)?),
                TileFormat::Npy => Box::new(NpySource::open(profile, &tile_path)?),
            };
            let descriptor = source.descriptor();
            normalizations.extend(source.normalizations().iter().copied());
            if descriptor.dtype != profile.dtype || descriptor.channels != profile.channels.len() {
                return Err(Error::InvalidInput(format!(
                    "tile {} dtype or channel count disagrees with the raster profile",
                    raw.path
                )));
            }
            let effective = effective_region(
                raw.sample_grid_origin,
                raw.valid_region,
                descriptor,
                manifest.overlap_policy,
            )?;
            height = height.max(effective.bottom());
            width = width.max(effective.right());
            tile_height = tile_height.min(descriptor.tile_height);
            tile_width = tile_width.min(descriptor.tile_width);
            tiles.push(ManifestTile {
                source,
                origin_y: raw.sample_grid_origin.y,
                origin_x: raw.sample_grid_origin.x,
                effective,
            });
        }
        let precision = match profile.dtype {
            RasterDType::Float32 => RasterOutputPrecision::Float32,
            RasterDType::Float64 => RasterOutputPrecision::Float64,
            _ => profile
                .integer_scaling
                .as_ref()
                .map(|scaling| scaling.output_precision)
                .ok_or_else(|| {
                    Error::InvalidInput("integer raster has no scaling declaration".into())
                })?,
        };
        if let Some(normalization) = manifest.overlap_policy.normalization() {
            normalizations.insert(normalization);
        }
        Ok(Self {
            descriptor: RasterDescriptor {
                height,
                width,
                channels: profile.channels.len(),
                dtype: profile.dtype,
                tile_height: tile_height.min(default_tile_length(height)),
                tile_width: tile_width.min(default_tile_length(width)),
            },
            tiles,
            overlap_policy: manifest.overlap_policy,
            scaling: profile.integer_scaling.clone(),
            precision,
            normalizations: normalizations.into_iter().collect(),
        })
    }
}

impl RasterSource for TiledManifestSource {
    fn descriptor(&self) -> RasterDescriptor {
        self.descriptor
    }

    fn read_region(
        &self,
        channel: usize,
        origin_y: u32,
        origin_x: u32,
        height: u32,
        width: u32,
    ) -> Result<RawTile> {
        validate_region(self.descriptor, channel, origin_y, origin_x, height, width)?;
        let requested = Region {
            y: origin_y,
            x: origin_x,
            height,
            width,
        };
        let length = usize::try_from(u64::from(height) * u64::from(width)).map_err(|_| {
            Error::InvalidInput("requested manifest tile is too large for this platform".into())
        })?;
        let mut composite = CompositeTile::new(self.precision, length);
        for tile in &self.tiles {
            let Some(intersection) = requested.intersection(tile.effective) else {
                continue;
            };
            let raw = tile.source.read_region(
                channel,
                intersection.y - tile.origin_y,
                intersection.x - tile.origin_x,
                intersection.height,
                intersection.width,
            )?;
            let values = normalize_tile(raw, self.scaling.as_ref())?;
            composite.merge(values, intersection, requested, self.overlap_policy)?;
        }
        composite.finish(self.overlap_policy)
    }

    fn normalizations(&self) -> &[SourceNormalization] {
        &self.normalizations
    }
}

impl OverlapPolicy {
    fn normalization(self) -> Option<SourceNormalization> {
        match self {
            Self::Reject => None,
            Self::ValidRegionCrop => Some(SourceNormalization::ManifestValidRegionCrop),
            Self::Mean => Some(SourceNormalization::ManifestOverlapMean),
            Self::Max => Some(SourceNormalization::ManifestOverlapMax),
            Self::ManifestOrderLastWrite => Some(SourceNormalization::ManifestOrderLastWrite),
        }
    }
}

enum CompositeTile {
    F32 { values: Vec<f32>, counts: Vec<u32> },
    F64 { values: Vec<f64>, counts: Vec<u32> },
}

impl CompositeTile {
    fn new(precision: RasterOutputPrecision, length: usize) -> Self {
        match precision {
            RasterOutputPrecision::Float32 => Self::F32 {
                values: vec![f32::NAN; length],
                counts: vec![0; length],
            },
            RasterOutputPrecision::Float64 => Self::F64 {
                values: vec![f64::NAN; length],
                counts: vec![0; length],
            },
        }
    }

    fn merge(
        &mut self,
        source: PixelTile,
        intersection: Region,
        destination: Region,
        policy: OverlapPolicy,
    ) -> Result<()> {
        match (self, source) {
            (Self::F32 { values, counts }, PixelTile::F32(source)) => {
                merge_values(values, counts, &source, intersection, destination, policy)
            }
            (Self::F64 { values, counts }, PixelTile::F64(source)) => {
                merge_values(values, counts, &source, intersection, destination, policy)
            }
            _ => Err(Error::InvalidInput(
                "manifest tiles disagree on normalized output precision".into(),
            )),
        }
    }

    fn finish(mut self, policy: OverlapPolicy) -> Result<RawTile> {
        match &mut self {
            Self::F32 { values, counts } if policy == OverlapPolicy::Mean => {
                finish_mean(values, counts)?;
            }
            Self::F64 { values, counts } if policy == OverlapPolicy::Mean => {
                finish_mean(values, counts)?;
            }
            _ => {}
        }
        Ok(match self {
            Self::F32 { values, .. } => RawTile::F32(values),
            Self::F64 { values, .. } => RawTile::F64(values),
        })
    }
}

trait FloatSample: Copy + PartialOrd + std::ops::AddAssign + std::ops::DivAssign {
    fn is_nan(self) -> bool;
    fn from_count(count: u32) -> Self;
}

impl FloatSample for f32 {
    fn is_nan(self) -> bool {
        self.is_nan()
    }

    fn from_count(count: u32) -> Self {
        count as Self
    }
}

impl FloatSample for f64 {
    fn is_nan(self) -> bool {
        self.is_nan()
    }

    fn from_count(count: u32) -> Self {
        f64::from(count)
    }
}

fn merge_values<T: FloatSample>(
    destination: &mut [T],
    counts: &mut [u32],
    source: &[T],
    intersection: Region,
    requested: Region,
    policy: OverlapPolicy,
) -> Result<()> {
    for row in 0..intersection.height {
        for column in 0..intersection.width {
            let source_index = linear_index(row, column, intersection.width)?;
            let destination_index = linear_index(
                intersection.y - requested.y + row,
                intersection.x - requested.x + column,
                requested.width,
            )?;
            let sample = *source
                .get(source_index)
                .ok_or_else(|| Error::InvalidInput("manifest tile data is truncated".into()))?;
            let value = destination
                .get_mut(destination_index)
                .ok_or_else(|| Error::InvalidInput("manifest output index overflows".into()))?;
            let count = counts
                .get_mut(destination_index)
                .ok_or_else(|| Error::InvalidInput("manifest overlap index overflows".into()))?;
            match policy {
                OverlapPolicy::Reject | OverlapPolicy::ValidRegionCrop => {
                    if *count != 0 {
                        return Err(Error::InvalidInput(
                            "tiled manifest contains overlapping sample-grid regions".into(),
                        ));
                    }
                    *value = sample;
                    *count = 1;
                }
                OverlapPolicy::ManifestOrderLastWrite => {
                    *value = sample;
                    *count = count.checked_add(1).ok_or_else(overlap_count_error)?;
                }
                OverlapPolicy::Max if !sample.is_nan() => {
                    if *count == 0 || sample > *value {
                        *value = sample;
                    }
                    *count = count.checked_add(1).ok_or_else(overlap_count_error)?;
                }
                OverlapPolicy::Mean if !sample.is_nan() => {
                    if *count == 0 {
                        *value = sample;
                    } else {
                        *value += sample;
                    }
                    *count = count.checked_add(1).ok_or_else(|| {
                        Error::InvalidInput("too many overlapping manifest tiles".into())
                    })?;
                }
                OverlapPolicy::Mean | OverlapPolicy::Max => {}
            }
        }
    }
    Ok(())
}

fn overlap_count_error() -> Error {
    Error::InvalidInput("too many overlapping manifest tiles".into())
}

fn finish_mean<T: FloatSample>(values: &mut [T], counts: &[u32]) -> Result<()> {
    for (value, count) in values.iter_mut().zip(counts) {
        if *count > 1 {
            *value /= T::from_count(*count);
            if value.is_nan() {
                return Err(Error::InvalidInput(
                    "manifest overlap mean produced a nonfinite value".into(),
                ));
            }
        }
    }
    Ok(())
}

fn linear_index(y: u32, x: u32, width: u32) -> Result<usize> {
    usize::try_from(u64::from(y) * u64::from(width) + u64::from(x))
        .map_err(|_| Error::InvalidInput("manifest tile index overflows".into()))
}

fn read_manifest(path: &Path) -> Result<RawManifest> {
    let metadata = std::fs::metadata(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return Err(Error::InvalidInput(format!(
            "tiled manifest must be a regular file no larger than {MAX_MANIFEST_BYTES} bytes"
        )));
    }
    let bytes = std::fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    validate_unique_object_keys(&bytes, "tiled manifest")?;
    serde_json::from_slice(&bytes)
        .map_err(|error| Error::InvalidInput(format!("tiled manifest is not valid JSON: {error}")))
}

fn resolve_tile_path(parent: &Path, declared: &str) -> Result<PathBuf> {
    let declared = Path::new(declared);
    if declared.as_os_str().is_empty()
        || declared.is_absolute()
        || declared
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::InvalidInput(
            "manifest tile paths must be nonempty relative paths without traversal".into(),
        ));
    }
    let candidate = parent.join(declared);
    let canonical = std::fs::canonicalize(&candidate).map_err(|source| Error::Io {
        path: candidate,
        source,
    })?;
    if !canonical.starts_with(parent) {
        return Err(Error::InvalidInput(
            "manifest tile path escapes the manifest directory".into(),
        ));
    }
    Ok(canonical)
}

fn effective_region(
    origin: SampleGridOrigin,
    valid: Option<ValidRegion>,
    descriptor: RasterDescriptor,
    policy: OverlapPolicy,
) -> Result<Region> {
    if policy == OverlapPolicy::ValidRegionCrop && valid.is_none() {
        return Err(Error::InvalidInput(
            "valid-region-crop requires valid_region on every tile".into(),
        ));
    }
    let valid = valid.unwrap_or(ValidRegion {
        y: 0,
        x: 0,
        height: descriptor.height,
        width: descriptor.width,
    });
    if valid.height == 0
        || valid.width == 0
        || valid
            .y
            .checked_add(valid.height)
            .is_none_or(|end| end > descriptor.height)
        || valid
            .x
            .checked_add(valid.width)
            .is_none_or(|end| end > descriptor.width)
    {
        return Err(Error::InvalidInput(
            "manifest valid_region is empty or outside its tile".into(),
        ));
    }
    let y = origin
        .y
        .checked_add(valid.y)
        .ok_or_else(|| Error::InvalidInput("manifest tile y origin overflows".into()))?;
    let x = origin
        .x
        .checked_add(valid.x)
        .ok_or_else(|| Error::InvalidInput("manifest tile x origin overflows".into()))?;
    y.checked_add(valid.height)
        .ok_or_else(|| Error::InvalidInput("manifest tile y extent overflows".into()))?;
    x.checked_add(valid.width)
        .ok_or_else(|| Error::InvalidInput("manifest tile x extent overflows".into()))?;
    Ok(Region {
        y,
        x,
        height: valid.height,
        width: valid.width,
    })
}

#[cfg(test)]
#[path = "tiled_manifest_unit_tests.rs"]
mod tests;
