mod functional_groups;
mod image;
mod quantity;

use dicom_core::VR;
use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;

use crate::annotations::derived_object::{add_common_instance_reference, build_common_object};
use crate::annotations::dicom_dataset::{put_text, sequence};
use crate::Result;

use super::document::ParametricMapDocument;
use super::frame::FrameKey;
use functional_groups::{per_frame_functional_groups, shared_functional_groups};
use image::{
    add_dimensions, add_general_image_attributes, add_pixel_attributes, add_slide_geometry,
};
use quantity::add_algorithm_provenance;

pub(super) struct InstanceSpec<'a> {
    pub(super) sop_instance_uid: &'a str,
    pub(super) series_instance_uid: &'a str,
    pub(super) dimension_organization_uid: &'a str,
    pub(super) frames: &'a [FrameKey],
    pub(super) frame_offset: u32,
    pub(super) concatenation: Option<ConcatenationSpec<'a>>,
    pub(super) content_date: &'a str,
    pub(super) content_time: &'a str,
}

#[derive(Clone, Copy)]
pub(super) struct ConcatenationSpec<'a> {
    pub(super) uid: &'a str,
    pub(super) source_sop_instance_uid: &'a str,
    pub(super) number: u16,
    pub(super) total: u16,
}

pub(super) fn build_object(
    document: &ParametricMapDocument,
    spec: &InstanceSpec<'_>,
) -> Result<InMemDicomObject> {
    let mut object = build_common_object(
        &document.source,
        uids::PARAMETRIC_MAP_STORAGE,
        spec.sop_instance_uid,
        spec.series_instance_uid,
        "SM",
        &document.producer,
    )?;
    put_text(
        &mut object,
        tags::INSTANCE_CREATION_DATE,
        VR::DA,
        spec.content_date,
    );
    put_text(
        &mut object,
        tags::INSTANCE_CREATION_TIME,
        VR::TM,
        spec.content_time,
    );
    add_general_image_attributes(document, spec, &mut object)?;
    add_slide_geometry(document, &mut object)?;
    add_dimensions(document, spec, &mut object);
    add_pixel_attributes(document, &mut object)?;
    add_algorithm_provenance(document, &mut object)?;
    object.put(sequence(tags::ACQUISITION_CONTEXT_SEQUENCE, Vec::new()));
    object.put(sequence(
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        vec![shared_functional_groups(document)?],
    ));
    object.put(sequence(
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
        spec.frames
            .iter()
            .map(|frame| per_frame_functional_groups(document, *frame))
            .collect::<Result<Vec<_>>>()?,
    ));
    add_common_instance_reference(&mut object, &document.source);
    Ok(object)
}
