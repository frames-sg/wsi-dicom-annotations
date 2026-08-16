use dicom_dictionary_std::uids;
use sha2::{Digest, Sha256};

use crate::annotations::dicom_dataset::new_dicom_uid;
use crate::annotations::dicom_file::encoded_dicom_prefix_length;
use crate::{Error, Result};

use super::document::ParametricMapDocument;
use super::frame::{encode_frame, frame_byte_length, FrameKey};
use super::plan::{
    collect_keys, ParametricMapPartPlan, ParametricMapPlan, PartDraft, PlanIdentifiers,
};
use super::writer::{build_object, ConcatenationSpec, InstanceSpec};

const MAX_FRAMES_PER_INSTANCE: u32 = 65_535;
const EXPLICIT_LONG_HEADER_BYTES: u64 = 12;

#[derive(Clone, Copy)]
struct InstanceLimits {
    maximum_bytes: u64,
    frame_bytes: u64,
}

#[derive(Clone, Copy)]
struct PartPosition {
    number: u16,
    total: u16,
    concatenated: bool,
}

impl ParametricMapDocument {
    pub fn plan(&self, maximum_instance_bytes: u64) -> Result<ParametricMapPlan> {
        let frame_bytes = frame_byte_length(self.descriptor, self.precision)?;
        let frame_bytes = u64::try_from(frame_bytes)
            .map_err(|_| Error::InvalidInput("PM frame byte length does not fit u64".into()))?;
        if maximum_instance_bytes <= EXPLICIT_LONG_HEADER_BYTES + frame_bytes {
            return Err(Error::InvalidInput(format!(
                "maximum PM instance size {maximum_instance_bytes} cannot contain one encoded frame"
            )));
        }
        let identifiers = PlanIdentifiers::new();
        let limits = InstanceLimits {
            maximum_bytes: maximum_instance_bytes,
            frame_bytes,
        };
        if let Some(draft) = self.plan_single_instance(&identifiers, limits)? {
            return self.finish_plan(identifiers, vec![draft], maximum_instance_bytes, false);
        }
        let drafts = plan_concatenation_parts(self, &identifiers, limits)?;
        self.finish_plan(identifiers, drafts, maximum_instance_bytes, true)
    }

    fn plan_single_instance(
        &self,
        identifiers: &PlanIdentifiers,
        limits: InstanceLimits,
    ) -> Result<Option<PartDraft>> {
        if self.frame_count > MAX_FRAMES_PER_INSTANCE {
            return Ok(None);
        }
        let keys = collect_keys(&mut self.output_frames(), self.frame_count)?;
        let sop_instance_uid = new_dicom_uid();
        let spec = identifiers.instance_spec(&sop_instance_uid, &keys, 0, 1, 1, false);
        if encoded_length(self, &spec, limits.frame_bytes)? > limits.maximum_bytes {
            return Ok(None);
        }
        Ok(Some(PartDraft {
            sop_instance_uid,
            frame_offset: 0,
            frame_count: self.frame_count,
        }))
    }

    fn finish_plan(
        &self,
        identifiers: PlanIdentifiers,
        drafts: Vec<PartDraft>,
        maximum_instance_bytes: u64,
        concatenated: bool,
    ) -> Result<ParametricMapPlan> {
        let total = u16::try_from(drafts.len())
            .map_err(|_| Error::InvalidInput("PM concatenation exceeds 65535 instances".into()))?;
        let mut cursor = self.output_frames();
        let mut aggregate_digest = Sha256::new();
        let mut parts = Vec::with_capacity(drafts.len());
        for (index, draft) in drafts.into_iter().enumerate() {
            let keys = collect_keys(&mut cursor, draft.frame_count)?;
            let number = u16::try_from(index + 1).map_err(|_| {
                Error::InvalidInput("PM concatenation part number exceeds US".into())
            })?;
            parts.push(finalize_part(
                self,
                &identifiers,
                draft,
                &keys,
                PartPosition {
                    number,
                    total,
                    concatenated,
                },
                maximum_instance_bytes,
                &mut aggregate_digest,
            )?);
        }
        let aggregate_digest: [u8; 32] = aggregate_digest.finalize().into();
        if cursor.next()?.is_some() || aggregate_digest != self.pixel_digest {
            return Err(Error::InvalidInput(
                "PM raster changed after semantic preflight".into(),
            ));
        }
        Ok(ParametricMapPlan {
            semantic_digest: self.semantic_digest().to_string(),
            series_instance_uid: identifiers.series_instance_uid,
            dimension_organization_uid: identifiers.dimension_organization_uid,
            concatenation_uid: concatenated.then_some(identifiers.concatenation_uid),
            concatenation_source_sop_instance_uid: concatenated
                .then_some(identifiers.concatenation_source_sop_instance_uid),
            content_date: identifiers.content_date,
            content_time: identifiers.content_time,
            parts,
        })
    }
}

fn finalize_part(
    document: &ParametricMapDocument,
    identifiers: &PlanIdentifiers,
    draft: PartDraft,
    keys: &[FrameKey],
    position: PartPosition,
    maximum_instance_bytes: u64,
    aggregate_digest: &mut Sha256,
) -> Result<ParametricMapPartPlan> {
    let spec = identifiers.instance_spec(
        &draft.sop_instance_uid,
        keys,
        draft.frame_offset,
        position.number,
        position.total,
        position.concatenated,
    );
    let mut part_digest = Sha256::new();
    let mut pixel_length = 0_u64;
    for key in keys {
        let frame = encode_frame(&document.raster, document.descriptor, *key)?;
        if frame.all_missing {
            return Err(Error::InvalidInput(
                "PM raster changed while finalizing the write plan".into(),
            ));
        }
        part_digest.update(&frame.bytes);
        aggregate_digest.update(&frame.bytes);
        pixel_length = pixel_length
            .checked_add(
                u64::try_from(frame.bytes.len())
                    .map_err(|_| Error::InvalidInput("PM frame length does not fit u64".into()))?,
            )
            .ok_or_else(|| Error::InvalidInput("PM pixel value length overflows u64".into()))?;
    }
    let pixel_value_length = u32::try_from(pixel_length)
        .map_err(|_| Error::InvalidInput("PM pixel value exceeds DICOM VL".into()))?;
    let object = build_object(document, &spec)?;
    let encoded_file_length =
        encoded_dicom_prefix_length(object, uids::PARAMETRIC_MAP_STORAGE, spec.sop_instance_uid)?
            .checked_add(EXPLICIT_LONG_HEADER_BYTES)
            .and_then(|value| value.checked_add(pixel_length))
            .ok_or_else(|| Error::InvalidInput("PM file length overflows u64".into()))?;
    if encoded_file_length > maximum_instance_bytes {
        return Err(Error::InvalidInput(format!(
            "planned PM part {number} is {encoded_file_length} bytes, exceeding the {maximum_instance_bytes}-byte instance limit"
            , number = position.number
        )));
    }
    Ok(ParametricMapPartPlan {
        sop_instance_uid: draft.sop_instance_uid,
        frame_offset: draft.frame_offset,
        frame_count: draft.frame_count,
        pixel_value_length,
        encoded_file_length,
        pixel_sha256: format!("{:x}", part_digest.finalize()),
    })
}

fn plan_concatenation_parts(
    document: &ParametricMapDocument,
    identifiers: &PlanIdentifiers,
    limits: InstanceLimits,
) -> Result<Vec<PartDraft>> {
    let value_limit = u64::from(u32::MAX) / limits.frame_bytes;
    let configured_limit = limits.maximum_bytes / limits.frame_bytes;
    let candidate_limit = u64::from(MAX_FRAMES_PER_INSTANCE)
        .min(value_limit)
        .min(configured_limit);
    let candidate_limit = u32::try_from(candidate_limit)
        .map_err(|_| Error::InvalidInput("PM candidate frame count does not fit u32".into()))?;
    if candidate_limit == 0 {
        return Err(Error::InvalidInput(
            "maximum PM instance size cannot contain one pixel frame".into(),
        ));
    }
    build_part_drafts(document, identifiers, limits, candidate_limit)
}

fn build_part_drafts(
    document: &ParametricMapDocument,
    identifiers: &PlanIdentifiers,
    limits: InstanceLimits,
    candidate_limit: u32,
) -> Result<Vec<PartDraft>> {
    let mut cursor = document.output_frames();
    let mut pending = Vec::new();
    let mut drafts = Vec::new();
    let mut frame_offset = 0_u32;
    while frame_offset < document.frame_count {
        let wanted = candidate_limit.min(document.frame_count - frame_offset);
        let wanted = usize::try_from(wanted).map_err(|_| {
            Error::InvalidInput("PM candidate frame count exceeds addressable memory".into())
        })?;
        while pending.len() < wanted {
            let frame = cursor.next()?.ok_or_else(|| {
                Error::InvalidInput("PM raster ended during size planning".into())
            })?;
            pending.push(frame.key);
        }
        let number = u16::try_from(drafts.len() + 1)
            .map_err(|_| Error::InvalidInput("PM concatenation exceeds 65535 instances".into()))?;
        let fitting = largest_fitting_prefix(
            document,
            identifiers,
            &pending,
            frame_offset,
            number,
            limits,
        )?;
        drafts.push(PartDraft {
            sop_instance_uid: new_dicom_uid(),
            frame_offset,
            frame_count: fitting,
        });
        let fitting_usize = usize::try_from(fitting).map_err(|_| {
            Error::InvalidInput("PM planned frame count exceeds addressable memory".into())
        })?;
        pending.drain(..fitting_usize);
        frame_offset = frame_offset.checked_add(fitting).ok_or_else(|| {
            Error::InvalidInput("PM concatenation frame offset overflows UL".into())
        })?;
    }
    Ok(drafts)
}

fn largest_fitting_prefix(
    document: &ParametricMapDocument,
    identifiers: &PlanIdentifiers,
    keys: &[FrameKey],
    frame_offset: u32,
    number: u16,
    limits: InstanceLimits,
) -> Result<u32> {
    let mut low = 1_usize;
    let mut high = keys.len();
    let mut best = 0_usize;
    while low <= high {
        let middle = low + (high - low) / 2;
        let spec = InstanceSpec {
            sop_instance_uid: "2.25.340282366920938463463374607431768211455",
            series_instance_uid: &identifiers.series_instance_uid,
            dimension_organization_uid: &identifiers.dimension_organization_uid,
            frames: &keys[..middle],
            frame_offset,
            concatenation: Some(ConcatenationSpec {
                uid: &identifiers.concatenation_uid,
                source_sop_instance_uid: &identifiers.concatenation_source_sop_instance_uid,
                number,
                total: u16::MAX,
            }),
            content_date: &identifiers.content_date,
            content_time: &identifiers.content_time,
        };
        if encoded_length(document, &spec, limits.frame_bytes)? <= limits.maximum_bytes {
            best = middle;
            low = middle + 1;
        } else {
            high = middle - 1;
        }
    }
    let best = u32::try_from(best)
        .map_err(|_| Error::InvalidInput("PM planned frame count does not fit u32".into()))?;
    if best == 0 {
        return Err(Error::InvalidInput(format!(
            "maximum PM instance size {} cannot contain one frame and required metadata",
            limits.maximum_bytes
        )));
    }
    Ok(best)
}

fn encoded_length(
    document: &ParametricMapDocument,
    spec: &InstanceSpec<'_>,
    frame_bytes: u64,
) -> Result<u64> {
    let frame_count = u64::try_from(spec.frames.len())
        .map_err(|_| Error::InvalidInput("PM instance frame count does not fit u64".into()))?;
    let pixel_length = frame_count
        .checked_mul(frame_bytes)
        .ok_or_else(|| Error::InvalidInput("PM pixel value length overflows".into()))?;
    if pixel_length > u64::from(u32::MAX) {
        return Ok(u64::MAX);
    }
    let object = build_object(document, spec)?;
    encoded_dicom_prefix_length(object, uids::PARAMETRIC_MAP_STORAGE, spec.sop_instance_uid)?
        .checked_add(EXPLICIT_LONG_HEADER_BYTES)
        .and_then(|value| value.checked_add(pixel_length))
        .ok_or_else(|| Error::InvalidInput("PM file length overflows u64".into()))
}
