use crate::annotations::dicom_dataset::{dicom_now, new_dicom_uid};
use crate::{Error, Result};

use super::document::OutputFrameCursor;
use super::frame::FrameKey;
use super::writer::{ConcatenationSpec, InstanceSpec};

#[derive(Debug, Clone)]
pub struct ParametricMapPlan {
    pub(super) semantic_digest: String,
    pub(super) series_instance_uid: String,
    pub(super) dimension_organization_uid: String,
    pub(super) concatenation_uid: Option<String>,
    pub(super) concatenation_source_sop_instance_uid: Option<String>,
    pub(super) content_date: String,
    pub(super) content_time: String,
    pub(super) parts: Vec<ParametricMapPartPlan>,
}

#[derive(Debug, Clone)]
pub struct ParametricMapPartPlan {
    pub(super) sop_instance_uid: String,
    pub(super) frame_offset: u32,
    pub(super) frame_count: u32,
    pub(super) pixel_value_length: u32,
    pub(super) encoded_file_length: u64,
    pub(super) pixel_sha256: String,
}

#[derive(Debug, Clone)]
pub struct ParametricMapInstance {
    sop_instance_uid: String,
    series_instance_uid: String,
    frame_offset: u32,
    frame_count: u32,
    pixel_value_length: u32,
    encoded_file_length: u64,
    pixel_sha256: String,
}

impl ParametricMapPlan {
    #[must_use]
    pub fn parts(&self) -> &[ParametricMapPartPlan] {
        &self.parts
    }

    #[must_use]
    pub fn series_instance_uid(&self) -> &str {
        &self.series_instance_uid
    }

    #[must_use]
    pub fn concatenation_uid(&self) -> Option<&str> {
        self.concatenation_uid.as_deref()
    }

    #[must_use]
    pub fn concatenation_source_sop_instance_uid(&self) -> Option<&str> {
        self.concatenation_source_sop_instance_uid.as_deref()
    }

    pub(super) fn instance_spec<'a>(
        &'a self,
        part: &'a ParametricMapPartPlan,
        frames: &'a [FrameKey],
        number: u16,
        total: u16,
    ) -> Result<InstanceSpec<'a>> {
        let concatenation = match (
            self.concatenation_uid.as_deref(),
            self.concatenation_source_sop_instance_uid.as_deref(),
        ) {
            (Some(uid), Some(source_sop_instance_uid)) => Some(ConcatenationSpec {
                uid,
                source_sop_instance_uid,
                number,
                total,
            }),
            (None, None) => None,
            _ => {
                return Err(Error::InvalidInput(
                    "PM plan has incomplete concatenation identity".into(),
                ))
            }
        };
        Ok(InstanceSpec {
            sop_instance_uid: &part.sop_instance_uid,
            series_instance_uid: &self.series_instance_uid,
            dimension_organization_uid: &self.dimension_organization_uid,
            frames,
            frame_offset: part.frame_offset,
            concatenation,
            content_date: &self.content_date,
            content_time: &self.content_time,
        })
    }
}

impl ParametricMapPartPlan {
    #[must_use]
    pub fn sop_instance_uid(&self) -> &str {
        &self.sop_instance_uid
    }

    #[must_use]
    pub const fn frame_offset(&self) -> u32 {
        self.frame_offset
    }

    #[must_use]
    pub const fn frame_count(&self) -> u32 {
        self.frame_count
    }

    #[must_use]
    pub const fn pixel_value_length(&self) -> u32 {
        self.pixel_value_length
    }

    #[must_use]
    pub const fn encoded_file_length(&self) -> u64 {
        self.encoded_file_length
    }

    #[must_use]
    pub fn pixel_sha256(&self) -> &str {
        &self.pixel_sha256
    }
}

impl ParametricMapInstance {
    pub(super) fn from_part(part: &ParametricMapPartPlan, series_instance_uid: &str) -> Self {
        Self {
            sop_instance_uid: part.sop_instance_uid.clone(),
            series_instance_uid: series_instance_uid.to_string(),
            frame_offset: part.frame_offset,
            frame_count: part.frame_count,
            pixel_value_length: part.pixel_value_length,
            encoded_file_length: part.encoded_file_length,
            pixel_sha256: part.pixel_sha256.clone(),
        }
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
    pub const fn frame_offset(&self) -> u32 {
        self.frame_offset
    }

    #[must_use]
    pub const fn frame_count(&self) -> u32 {
        self.frame_count
    }

    #[must_use]
    pub const fn pixel_value_length(&self) -> u32 {
        self.pixel_value_length
    }

    #[must_use]
    pub const fn encoded_file_length(&self) -> u64 {
        self.encoded_file_length
    }

    #[must_use]
    pub fn pixel_sha256(&self) -> &str {
        &self.pixel_sha256
    }
}

pub(super) struct PlanIdentifiers {
    pub(super) series_instance_uid: String,
    pub(super) dimension_organization_uid: String,
    pub(super) concatenation_uid: String,
    pub(super) concatenation_source_sop_instance_uid: String,
    pub(super) content_date: String,
    pub(super) content_time: String,
}

impl PlanIdentifiers {
    pub(super) fn new() -> Self {
        let (content_date, content_time) = dicom_now();
        Self {
            series_instance_uid: new_dicom_uid(),
            dimension_organization_uid: new_dicom_uid(),
            concatenation_uid: new_dicom_uid(),
            concatenation_source_sop_instance_uid: new_dicom_uid(),
            content_date,
            content_time,
        }
    }

    pub(super) fn instance_spec<'a>(
        &'a self,
        sop_instance_uid: &'a str,
        frames: &'a [FrameKey],
        frame_offset: u32,
        number: u16,
        total: u16,
        concatenated: bool,
    ) -> InstanceSpec<'a> {
        InstanceSpec {
            sop_instance_uid,
            series_instance_uid: &self.series_instance_uid,
            dimension_organization_uid: &self.dimension_organization_uid,
            frames,
            frame_offset,
            concatenation: concatenated.then_some(ConcatenationSpec {
                uid: &self.concatenation_uid,
                source_sop_instance_uid: &self.concatenation_source_sop_instance_uid,
                number,
                total,
            }),
            content_date: &self.content_date,
            content_time: &self.content_time,
        }
    }
}

#[derive(Debug)]
pub(super) struct PartDraft {
    pub(super) sop_instance_uid: String,
    pub(super) frame_offset: u32,
    pub(super) frame_count: u32,
}

pub(super) fn collect_keys(
    cursor: &mut OutputFrameCursor<'_>,
    count: u32,
) -> Result<Vec<FrameKey>> {
    let capacity = usize::try_from(count)
        .map_err(|_| Error::InvalidInput("PM frame count exceeds addressable memory".into()))?;
    let mut keys = Vec::with_capacity(capacity);
    while keys.len() < capacity {
        let frame = cursor.next()?.ok_or_else(|| {
            Error::InvalidInput("PM raster ended before its declared frame count".into())
        })?;
        keys.push(frame.key);
    }
    Ok(keys)
}
