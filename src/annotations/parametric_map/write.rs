use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;

use dicom_core::VR;
use dicom_dictionary_std::{tags, uids};
use sha2::{Digest, Sha256};

use crate::annotations::dicom_file::{
    atomic_write_new_dicom_with_streamed_value, StreamedDicomValue,
};
use crate::{Error, Result};

use super::document::ParametricMapDocument;
use super::frame::{encode_frame, FrameKey, PixelPrecision};
use super::plan::{collect_keys, ParametricMapInstance, ParametricMapPartPlan, ParametricMapPlan};
use super::verify::{verify_instance, ExpectedInstance};
use super::writer::build_object;

impl ParametricMapDocument {
    pub fn write_single(
        &self,
        path: impl AsRef<Path>,
        maximum_instance_bytes: u64,
    ) -> Result<ParametricMapInstance> {
        let plan = self.plan(maximum_instance_bytes)?;
        if plan.parts.len() != 1 {
            return Err(Error::InvalidInput(format!(
                "PM requires {} concatenation parts; use an output directory",
                plan.parts.len()
            )));
        }
        let mut instances = self.write_planned_parts(&plan, &[path.as_ref()])?;
        instances.pop().ok_or_else(|| {
            Error::InvalidInput("PM plan unexpectedly contained no output part".into())
        })
    }

    pub fn write_planned_parts(
        &self,
        plan: &ParametricMapPlan,
        paths: &[impl AsRef<Path>],
    ) -> Result<Vec<ParametricMapInstance>> {
        self.validate_plan_paths(plan, paths)?;
        let total = u16::try_from(plan.parts.len()).map_err(|_| {
            Error::InvalidInput("PM concatenation contains too many instances".into())
        })?;
        let mut cursor = self.output_frames();
        let mut instances = Vec::with_capacity(plan.parts.len());
        for (index, (part, path)) in plan.parts.iter().zip(paths).enumerate() {
            let keys = collect_keys(&mut cursor, part.frame_count)?;
            let number = u16::try_from(index + 1).map_err(|_| {
                Error::InvalidInput("PM concatenation part number exceeds US".into())
            })?;
            let path = path.as_ref();
            self.write_part(plan, part, &keys, number, total, path)?;
            instances.push(ParametricMapInstance::from_part(
                part,
                &plan.series_instance_uid,
            ));
        }
        if cursor.next()?.is_some() {
            return Err(Error::InvalidInput(
                "PM raster changed after size planning".into(),
            ));
        }
        Ok(instances)
    }

    fn write_part(
        &self,
        plan: &ParametricMapPlan,
        part: &ParametricMapPartPlan,
        keys: &[FrameKey],
        number: u16,
        total: u16,
        path: &Path,
    ) -> Result<()> {
        let spec = plan.instance_spec(part, keys, number, total)?;
        let object = build_object(self, &spec)?;
        let streamed = StreamedDicomValue::new(
            pixel_tag(self.precision),
            pixel_vr(self.precision),
            part.pixel_value_length,
        )?;
        let expected = ExpectedInstance::new(self, &spec, part);
        atomic_write_new_dicom_with_streamed_value(
            path,
            object,
            uids::PARAMETRIC_MAP_STORAGE,
            &part.sop_instance_uid,
            streamed,
            |writer| write_part_pixels(self, keys, part, path, writer),
            |temporary| verify_instance(temporary, &expected),
        )
    }

    fn validate_plan_paths(
        &self,
        plan: &ParametricMapPlan,
        paths: &[impl AsRef<Path>],
    ) -> Result<()> {
        if plan.semantic_digest != self.semantic_digest() {
            return Err(Error::InvalidInput(
                "PM plan does not belong to this raster document".into(),
            ));
        }
        if paths.len() != plan.parts.len() {
            return Err(Error::InvalidInput(format!(
                "PM plan requires {} output paths, got {}",
                plan.parts.len(),
                paths.len()
            )));
        }
        let mut unique = BTreeSet::new();
        for path in paths {
            validate_new_path(path.as_ref(), &mut unique)?;
        }
        Ok(())
    }
}

fn validate_new_path(path: &Path, unique: &mut BTreeSet<std::path::PathBuf>) -> Result<()> {
    if !unique.insert(path.to_path_buf()) {
        return Err(Error::InvalidInput(format!(
            "duplicate PM output path: {}",
            path.display()
        )));
    }
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(Error::InvalidInput(format!(
            "output path already exists: {}",
            path.display()
        ))),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn write_part_pixels(
    document: &ParametricMapDocument,
    keys: &[FrameKey],
    part: &ParametricMapPartPlan,
    output_path: &Path,
    writer: &mut impl Write,
) -> Result<()> {
    let mut digest = Sha256::new();
    let mut written = 0_u64;
    for key in keys {
        let frame = encode_frame(&document.raster, document.descriptor, *key)?;
        if frame.all_missing {
            return Err(Error::InvalidInput(
                "PM raster changed after write planning".into(),
            ));
        }
        writer.write_all(&frame.bytes).map_err(|source| Error::Io {
            path: output_path.to_path_buf(),
            source,
        })?;
        digest.update(&frame.bytes);
        written = written
            .checked_add(
                u64::try_from(frame.bytes.len())
                    .map_err(|_| Error::InvalidInput("PM frame length does not fit u64".into()))?,
            )
            .ok_or_else(|| Error::InvalidInput("PM write length overflows u64".into()))?;
    }
    if written != u64::from(part.pixel_value_length)
        || format!("{:x}", digest.finalize()) != part.pixel_sha256
    {
        return Err(Error::InvalidInput(
            "PM raster bytes changed after write planning".into(),
        ));
    }
    Ok(())
}

fn pixel_tag(precision: PixelPrecision) -> dicom_core::Tag {
    match precision {
        PixelPrecision::Float32 => tags::FLOAT_PIXEL_DATA,
        PixelPrecision::Float64 => tags::DOUBLE_FLOAT_PIXEL_DATA,
    }
}

fn pixel_vr(precision: PixelPrecision) -> VR {
    match precision {
        PixelPrecision::Float32 => VR::OF,
        PixelPrecision::Float64 => VR::OD,
    }
}
