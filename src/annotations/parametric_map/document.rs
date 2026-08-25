use std::path::Path;

use sha2::{Digest, Sha256};

use crate::{
    DerivedObjectProducer, DicomAnnotationContext, Error, InteroperabilityDiagnostic, Result,
};

use super::super::semantic_digest::{finish, update_algorithm, update_code, update_text};
use super::frame::{encode_frame, FrameKey, PixelPrecision};
use super::geometry::RasterGeometry;
use super::profile::{RasterDType, RasterOutputPrecision};
use super::{NormalizedRaster, RasterChannelSelection, RasterDescriptor, RasterProfile};

pub struct ParametricMapDocument {
    pub(super) source: DicomAnnotationContext,
    pub(super) profile: RasterProfile,
    pub(super) producer: DerivedObjectProducer,
    pub(super) raster: NormalizedRaster,
    pub(super) selected_channels: Vec<usize>,
    pub(super) descriptor: RasterDescriptor,
    pub(super) geometry: RasterGeometry,
    pub(super) precision: PixelPrecision,
    pub(super) frame_count: u32,
    pub(super) dense: bool,
    pub(super) channel_ranges: Vec<ValueRange>,
    pub(super) pixel_digest: [u8; 32],
    semantic_digest: String,
    diagnostics: Vec<InteroperabilityDiagnostic>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ValueRange {
    pub(super) minimum: f64,
    pub(super) maximum: f64,
}

impl ParametricMapDocument {
    pub fn open(
        source: DicomAnnotationContext,
        canonical_source: DicomAnnotationContext,
        profile: RasterProfile,
        raster_path: impl AsRef<Path>,
        channel_selection: RasterChannelSelection,
    ) -> Result<Self> {
        let selected_channels = profile.select_channels(channel_selection)?;
        let raster_path = raster_path.as_ref().to_path_buf();
        let raster = NormalizedRaster::open(&profile, &raster_path)?;
        let descriptor = raster.descriptor();
        validate_descriptor(descriptor)?;
        let precision = output_precision(&profile);
        let geometry = RasterGeometry::resolve(&source, &canonical_source, &profile, descriptor)?;
        let mut document = Self {
            source,
            profile,
            producer: DerivedObjectProducer::library_default(9401, "WSI parametric maps"),
            raster,
            selected_channels,
            descriptor,
            geometry,
            precision,
            frame_count: 0,
            dense: false,
            channel_ranges: Vec::new(),
            pixel_digest: [0; 32],
            semantic_digest: String::new(),
            diagnostics: Vec::new(),
        };
        document.scan()?;
        Ok(document)
    }

    /// Replaces the neutral library identity with caller-owned producer metadata.
    #[must_use]
    pub fn with_producer(mut self, producer: DerivedObjectProducer) -> Self {
        self.producer = producer;
        self
    }

    #[must_use]
    pub fn producer(&self) -> &DerivedObjectProducer {
        &self.producer
    }

    #[must_use]
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    #[must_use]
    pub fn semantic_digest(&self) -> &str {
        &self.semantic_digest
    }

    #[must_use]
    pub fn selected_channel_count(&self) -> usize {
        self.selected_channels.len()
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[InteroperabilityDiagnostic] {
        &self.diagnostics
    }

    fn scan(&mut self) -> Result<()> {
        let mut digest = Sha256::new();
        let mut ranges = vec![MutableRange::default(); self.selected_channels.len()];
        let mut frame_count = 0_u64;
        let mut missing_frame_count = 0_u64;
        let mut missing_sample_count = 0_u64;
        let mut padded_sample_count = 0_u64;
        let tiles_across = self.descriptor.width.div_ceil(self.descriptor.tile_width);
        let tiles_down = self.descriptor.height.div_ceil(self.descriptor.tile_height);
        for (channel_ordinal, channel) in self.selected_channels.iter().copied().enumerate() {
            let channel_ordinal = u32::try_from(channel_ordinal + 1)
                .map_err(|_| Error::InvalidInput("raster channel count exceeds DICOM UL".into()))?;
            for tile_row in 0..tiles_down {
                for tile_column in 0..tiles_across {
                    let key = FrameKey {
                        channel,
                        channel_ordinal,
                        tile_row,
                        tile_column,
                    };
                    let frame = encode_frame(&self.raster, self.descriptor, key)?;
                    validate_encoded_precision(self.precision, self.descriptor, frame.bytes.len())?;
                    missing_sample_count = missing_sample_count
                        .checked_add(frame.missing_sample_count)
                        .ok_or_else(|| {
                            Error::InvalidInput("missing raster sample count overflows".into())
                        })?;
                    padded_sample_count = padded_sample_count
                        .checked_add(frame.padded_sample_count)
                        .ok_or_else(|| {
                            Error::InvalidInput("padded raster sample count overflows".into())
                        })?;
                    if frame.all_missing {
                        missing_frame_count += 1;
                        continue;
                    }
                    digest.update(&frame.bytes);
                    ranges[channel_ordinal as usize - 1]
                        .observe(frame.finite_minimum, frame.finite_maximum);
                    frame_count += 1;
                }
            }
        }
        if frame_count == 0 {
            return Err(Error::InvalidInput(
                "selected raster channels contain no finite samples".into(),
            ));
        }
        self.frame_count = u32::try_from(frame_count).map_err(|_| {
            Error::InvalidInput(
                "selected raster contains more frames than Concatenation Frame Offset Number can represent"
                    .into(),
            )
        })?;
        self.dense = self.selected_channels.len() == 1 && missing_frame_count == 0;
        self.channel_ranges = ranges
            .into_iter()
            .map(MutableRange::finish)
            .collect::<Result<Vec<_>>>()?;
        self.pixel_digest = digest.finalize().into();
        self.semantic_digest = semantic_digest(self);
        self.diagnostics = build_diagnostics(
            &self.profile,
            self.raster.normalizations(),
            missing_sample_count,
            padded_sample_count,
            missing_frame_count,
        );
        Ok(())
    }

    pub(super) fn output_frames(&self) -> OutputFrameCursor<'_> {
        OutputFrameCursor {
            document: self,
            channel_ordinal: 0,
            tile_index: 0,
        }
    }
}

fn build_diagnostics(
    profile: &RasterProfile,
    source_normalizations: &[InteroperabilityDiagnostic],
    missing_samples: u64,
    padded_samples: u64,
    omitted_frames: u64,
) -> Vec<InteroperabilityDiagnostic> {
    let mut diagnostics = source_normalizations.to_vec();
    if profile.dtype.is_integer() {
        diagnostics.push(InteroperabilityDiagnostic::normalized(
            "RASTER_INTEGER_SCALED",
            "$.raster.integer_scaling",
            "integer samples were transformed with the explicitly declared slope, intercept, missing sentinel, and output precision",
        ));
    }
    if missing_samples > 0 {
        diagnostics.push(InteroperabilityDiagnostic::normalized(
            "RASTER_NAN_CANONICALIZED",
            "$.raster.values",
            format!(
                "canonicalized {missing_samples} missing samples to the selected quiet-NaN payload"
            ),
        ));
    }
    if padded_samples > 0 {
        diagnostics.push(InteroperabilityDiagnostic::normalized(
            "PM_EDGE_TILE_PADDED",
            "$.raster.edge_tiles",
            format!("padded {padded_samples} edge-tile samples with the canonical quiet NaN"),
        ));
    }
    if omitted_frames > 0 {
        diagnostics.push(InteroperabilityDiagnostic::normalized(
            "PM_ALL_MISSING_TILES_OMITTED",
            "$.raster.tiles",
            format!("omitted {omitted_frames} all-missing tiles and encoded TILED_SPARSE"),
        ));
    }
    diagnostics
}

pub(super) struct OutputFrameCursor<'a> {
    document: &'a ParametricMapDocument,
    channel_ordinal: usize,
    tile_index: u64,
}

impl OutputFrameCursor<'_> {
    pub(super) fn next(&mut self) -> Result<Option<super::frame::EncodedFrame>> {
        let descriptor = self.document.descriptor;
        let tiles_across = descriptor.width.div_ceil(descriptor.tile_width);
        let tiles_down = descriptor.height.div_ceil(descriptor.tile_height);
        let tiles_per_channel = u64::from(tiles_across) * u64::from(tiles_down);
        while self.channel_ordinal < self.document.selected_channels.len() {
            if self.tile_index == tiles_per_channel {
                self.channel_ordinal += 1;
                self.tile_index = 0;
                continue;
            }
            let key = FrameKey {
                channel: self.document.selected_channels[self.channel_ordinal],
                channel_ordinal: self.channel_ordinal as u32 + 1,
                tile_row: (self.tile_index / u64::from(tiles_across)) as u32,
                tile_column: (self.tile_index % u64::from(tiles_across)) as u32,
            };
            self.tile_index += 1;
            let frame = encode_frame(&self.document.raster, descriptor, key)?;
            if !frame.all_missing {
                return Ok(Some(frame));
            }
        }
        Ok(None)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MutableRange {
    minimum: Option<f64>,
    maximum: Option<f64>,
}

impl MutableRange {
    fn observe(&mut self, minimum: Option<f64>, maximum: Option<f64>) {
        if let Some(value) = minimum {
            self.minimum = Some(self.minimum.map_or(value, |current| current.min(value)));
        }
        if let Some(value) = maximum {
            self.maximum = Some(self.maximum.map_or(value, |current| current.max(value)));
        }
    }

    fn finish(self) -> Result<ValueRange> {
        match (self.minimum, self.maximum) {
            (Some(minimum), Some(maximum)) => Ok(ValueRange { minimum, maximum }),
            _ => Err(Error::InvalidInput(
                "a selected raster channel contains no finite samples".into(),
            )),
        }
    }
}

fn validate_descriptor(descriptor: RasterDescriptor) -> Result<()> {
    if descriptor.tile_height == 0
        || descriptor.tile_width == 0
        || descriptor.tile_height > u32::from(u16::MAX)
        || descriptor.tile_width > u32::from(u16::MAX)
    {
        return Err(Error::InvalidInput(
            "PM tile rows and columns must each fit a nonzero DICOM US value".into(),
        ));
    }
    Ok(())
}

fn output_precision(profile: &RasterProfile) -> PixelPrecision {
    match profile.dtype {
        RasterDType::Float32 => PixelPrecision::Float32,
        RasterDType::Float64 => PixelPrecision::Float64,
        _ => match profile
            .integer_scaling
            .as_ref()
            .map(|scaling| scaling.output_precision)
        {
            Some(RasterOutputPrecision::Float64) => PixelPrecision::Float64,
            Some(RasterOutputPrecision::Float32) | None => PixelPrecision::Float32,
        },
    }
}

fn validate_encoded_precision(
    precision: PixelPrecision,
    descriptor: RasterDescriptor,
    actual: usize,
) -> Result<()> {
    let expected = super::frame::frame_byte_length(descriptor, precision)?;
    if actual != expected {
        return Err(Error::InvalidInput(format!(
            "raster adapter produced {actual} bytes per frame, expected {expected} for the profile precision"
        )));
    }
    Ok(())
}

fn semantic_digest(document: &ParametricMapDocument) -> String {
    let mut digest = Sha256::new();
    digest.update(b"dicom-parametric-map-v1\0");
    update_text(&mut digest, document.source.study_instance_uid());
    update_text(&mut digest, document.source.sop_instance_uid());
    update_text(
        &mut digest,
        document.source.frame_of_reference_uid().unwrap_or(""),
    );
    for value in [
        document.descriptor.height,
        document.descriptor.width,
        document.descriptor.tile_height,
        document.descriptor.tile_width,
    ] {
        digest.update(value.to_le_bytes());
    }
    for value in document
        .geometry
        .origin
        .into_iter()
        .chain(document.geometry.orientation)
        .chain(document.geometry.pixel_spacing)
    {
        digest.update(value.to_bits().to_le_bytes());
    }
    for channel in &document.selected_channels {
        let channel = &document.profile.channels[*channel];
        update_text(&mut digest, &channel.name);
        update_code(&mut digest, &channel.quantity);
        update_code(&mut digest, &channel.unit);
    }
    update_algorithm(&mut digest, &document.profile.algorithm);
    digest.update(document.pixel_digest);
    finish(digest)
}
