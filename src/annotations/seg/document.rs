use super::*;

impl SegmentationDocument {
    pub fn binary(
        source: DicomAnnotationContext,
        segments: Vec<SegmentationSegment>,
    ) -> Result<Self> {
        if segments.is_empty() || segments.len() > usize::from(u16::MAX) {
            return Err(Error::InvalidInput(
                "a binary SEG document must contain 1..=65535 segments".into(),
            ));
        }
        for segment in &segments {
            for point in segment
                .outer_polygons()
                .iter()
                .chain(segment.exclusion_polygons())
                .flatten()
            {
                source.validate_point(point.x, point.y)?;
            }
        }
        Ok(Self {
            source,
            kind: SegmentationKind::Binary,
            sop_instance_uid: new_dicom_uid(),
            series_instance_uid: new_dicom_uid(),
            content_label: "WSI_SEGMENTATION".into(),
            content_description: "WSI binary segmentation".into(),
            content_creator_name: None,
            series_equipment: SeriesEquipmentMetadata::viewer("9201"),
            segments,
            imported_binary_frames: None,
            imported_fractional_frames: None,
            diagnostics: Vec::new(),
        })
    }

    pub fn read_seg(path: impl AsRef<Path>, source: &DicomAnnotationContext) -> Result<Self> {
        let path = path.as_ref();
        enforce_file_limit(path, MAX_SEG_FILE_BYTES, "SEG")?;
        let object = dicom_object::open_file(path).map_err(|error| Error::DicomRead {
            path: path.to_path_buf(),
            source: Box::new(error),
        })?;
        let sop_class = object.meta().media_storage_sop_class_uid();
        if sop_class != uids::SEGMENTATION_STORAGE
            && sop_class != uids::LABEL_MAP_SEGMENTATION_STORAGE
        {
            return Err(Error::Unsupported(format!(
                "{} is not a DICOM Segmentation instance",
                path.display()
            )));
        }
        validate_source_reference(&object, source)?;
        let segmentation_type = required_string(&object, tags::SEGMENTATION_TYPE)?;
        let kind = match segmentation_type.as_str() {
            "BINARY" => SegmentationKind::Binary,
            "LABELMAP" => SegmentationKind::LabelMap,
            "FRACTIONAL" => SegmentationKind::Fractional,
            other => {
                return Err(Error::Unsupported(format!(
                    "SEG type {other:?} is not recognized"
                )))
            }
        };
        if !matches!(
            object.meta().transfer_syntax(),
            uids::IMPLICIT_VR_LITTLE_ENDIAN | uids::EXPLICIT_VR_LITTLE_ENDIAN
        ) {
            return Err(Error::Unsupported(
                "compressed or big-endian SEG Pixel Data is not supported".into(),
            ));
        }
        let mut diagnostics = Vec::new();
        let segments = read_segments(
            &object,
            &mut diagnostics,
            kind == SegmentationKind::LabelMap,
        )?;
        let (imported_binary_frames, imported_fractional_frames) = match kind {
            SegmentationKind::Binary => {
                (Some(read_binary_frames(&object, source, &segments)?), None)
            }
            SegmentationKind::LabelMap => (
                Some(read_labelmap_frames(&object, source, &segments)?),
                None,
            ),
            SegmentationKind::Fractional => (
                None,
                Some(read_fractional_frames(&object, source, &segments)?),
            ),
        };
        Ok(Self {
            source: source.clone(),
            kind,
            sop_instance_uid: required_string(&object, tags::SOP_INSTANCE_UID)?,
            series_instance_uid: required_string(&object, tags::SERIES_INSTANCE_UID)?,
            content_label: required_string(&object, tags::CONTENT_LABEL)?,
            content_description: optional_string(&object, tags::CONTENT_DESCRIPTION)
                .unwrap_or_default(),
            content_creator_name: optional_string(&object, tags::CONTENT_CREATOR_NAME),
            series_equipment: SeriesEquipmentMetadata::read(&object),
            segments,
            imported_binary_frames,
            imported_fractional_frames,
            diagnostics,
        })
    }

    pub fn rasterized_frames(&self) -> Result<Vec<BinarySegmentationFrame>> {
        if let Some(frames) = &self.imported_binary_frames {
            return Ok(frames.clone());
        }
        if self.kind != SegmentationKind::Binary {
            return Err(Error::Unsupported(
                "fractional SEG is a read-only raster overlay and cannot be vectorized as binary"
                    .into(),
            ));
        }
        rasterize_segments(&self.source, &self.segments)
    }

    pub fn binary_runs(&self) -> Result<Vec<BinaryMaskRun>> {
        if self.kind == SegmentationKind::Fractional {
            return Err(Error::Unsupported(
                "fractional SEG does not contain binary mask runs".into(),
            ));
        }
        let mut runs = Vec::new();
        for frame in self.rasterized_frames()? {
            let base_column = frame
                .tile_col
                .checked_mul(u32::from(frame.width))
                .ok_or_else(|| Error::InvalidInput("SEG column position overflows".into()))?;
            let base_row = frame
                .tile_row
                .checked_mul(u32::from(frame.height))
                .ok_or_else(|| Error::InvalidInput("SEG row position overflows".into()))?;
            for row in 0..usize::from(frame.height) {
                let row_start = row * usize::from(frame.width);
                let values = &frame.mask[row_start..row_start + usize::from(frame.width)];
                let mut column = 0_usize;
                while column < values.len() {
                    if !values[column] {
                        column += 1;
                        continue;
                    }
                    let start = column;
                    while column < values.len() && values[column] {
                        column += 1;
                    }
                    if runs.len() >= MAX_VECTORIZED_RUNS {
                        return Err(Error::InvalidInput(format!(
                            "SEG normalization exceeds the {MAX_VECTORIZED_RUNS}-run resource limit"
                        )));
                    }
                    let row = u32::try_from(row).map_err(|_| {
                        Error::InvalidInput("SEG row index exceeds DICOM range".into())
                    })?;
                    let start = u32::try_from(start).map_err(|_| {
                        Error::InvalidInput("SEG column index exceeds DICOM range".into())
                    })?;
                    let length = u32::try_from(column)
                        .ok()
                        .and_then(|end| end.checked_sub(start))
                        .ok_or_else(|| {
                            Error::InvalidInput("SEG run length exceeds DICOM range".into())
                        })?;
                    runs.push(BinaryMaskRun {
                        segment_number: frame.segment_number,
                        row: base_row + row,
                        column_start: base_column + start,
                        length,
                    });
                }
            }
        }
        runs.sort_by_key(|run| (run.segment_number, run.row, run.column_start));
        merge_binary_runs(runs)
    }

    pub fn fractional_runs(&self) -> Result<Vec<FractionalMaskRun>> {
        let frames = self.fractional_frames().ok_or_else(|| {
            Error::Unsupported("SEG does not contain fractional mask values".into())
        })?;
        let mut runs = Vec::new();
        for frame in frames {
            let base_column = frame
                .tile_col
                .checked_mul(u32::from(frame.width))
                .ok_or_else(|| Error::InvalidInput("SEG column position overflows".into()))?;
            let base_row = frame
                .tile_row
                .checked_mul(u32::from(frame.height))
                .ok_or_else(|| Error::InvalidInput("SEG row position overflows".into()))?;
            for row in 0..usize::from(frame.height) {
                let row_start = row * usize::from(frame.width);
                let values = &frame.values[row_start..row_start + usize::from(frame.width)];
                let mut column = 0_usize;
                while column < values.len() {
                    if values[column] == 0 {
                        column += 1;
                        continue;
                    }
                    let start = column;
                    while column < values.len() && values[column] != 0 {
                        column += 1;
                    }
                    if runs.len() >= MAX_VECTORIZED_RUNS {
                        return Err(Error::InvalidInput(format!(
                            "SEG normalization exceeds the {MAX_VECTORIZED_RUNS}-run resource limit"
                        )));
                    }
                    let row_u32 = u32::try_from(row).map_err(|_| {
                        Error::InvalidInput("SEG row index exceeds DICOM range".into())
                    })?;
                    let start_u32 = u32::try_from(start).map_err(|_| {
                        Error::InvalidInput("SEG column index exceeds DICOM range".into())
                    })?;
                    runs.push(FractionalMaskRun {
                        segment_number: frame.segment_number,
                        row: base_row + row_u32,
                        column_start: base_column + start_u32,
                        maximum_fractional_value: frame.maximum_fractional_value,
                        values: values[start..column].to_vec(),
                    });
                }
            }
        }
        runs.sort_by_key(|run| (run.segment_number, run.row, run.column_start));
        merge_fractional_runs(runs)
    }

    pub fn mask_digest(&self) -> Result<String> {
        let mut digest = Sha256::new();
        digest.update(b"dicom-viewer-seg-runs-v1\0");
        let (columns, rows) = self.source.total_pixel_matrix_dimensions();
        digest.update(columns.to_le_bytes());
        digest.update(rows.to_le_bytes());
        digest.update([match self.kind {
            SegmentationKind::Binary => 0,
            SegmentationKind::LabelMap => 1,
            SegmentationKind::Fractional => 2,
        }]);
        if self.kind == SegmentationKind::Fractional {
            for run in self.fractional_runs()? {
                digest.update(run.segment_number.to_le_bytes());
                digest.update(run.row.to_le_bytes());
                digest.update(run.column_start.to_le_bytes());
                digest.update(run.maximum_fractional_value.to_le_bytes());
                digest.update(
                    u32::try_from(run.values.len())
                        .map_err(|_| {
                            Error::InvalidInput(
                                "fractional SEG run length exceeds DICOM range".into(),
                            )
                        })?
                        .to_le_bytes(),
                );
                for value in run.values {
                    digest.update(value.to_le_bytes());
                }
            }
        } else {
            for run in self.binary_runs()? {
                digest.update(run.segment_number.to_le_bytes());
                digest.update(run.row.to_le_bytes());
                digest.update(run.column_start.to_le_bytes());
                digest.update(run.length.to_le_bytes());
            }
        }
        Ok(format!("{:x}", digest.finalize()))
    }

    #[must_use]
    pub fn fractional_frames(&self) -> Option<&[FractionalSegmentationFrame]> {
        self.imported_fractional_frames.as_deref()
    }

    pub fn write_seg(&self, path: impl AsRef<Path>) -> Result<()> {
        self.write_seg_with_loss_policy(path, false)
    }

    pub fn write_seg_with_loss_policy(
        &self,
        path: impl AsRef<Path>,
        allow_lossy: bool,
    ) -> Result<()> {
        if self.kind != SegmentationKind::Binary {
            return Err(Error::Unsupported(
                "only binary SEG export is supported".into(),
            ));
        }
        if !allow_lossy {
            let blocking = self
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.blocks_roundtrip())
                .map(InteroperabilityDiagnostic::code)
                .collect::<Vec<_>>();
            if !blocking.is_empty() {
                return Err(Error::Unsupported(format!(
                    "SEG rewrite would lose semantics ({}); pass the explicit lossy override to continue",
                    blocking.join(", ")
                )));
            }
        }
        let path = path.as_ref();
        ensure_sidecar_destination(path, self.source.source_path())?;
        let frames = self.rasterized_frames()?;
        if frames.is_empty() {
            return Err(Error::InvalidInput(
                "SEG contains no non-empty frames".into(),
            ));
        }
        let mut object = build_common_object(
            &self.source,
            uids::SEGMENTATION_STORAGE,
            &self.sop_instance_uid,
            &self.series_instance_uid,
            "SEG",
            &self.series_equipment,
        )?;
        add_segmentation_attributes(&mut object, self, &frames)?;
        add_common_instance_reference(&mut object, &self.source);
        atomic_write_dicom(
            path,
            object,
            uids::SEGMENTATION_STORAGE,
            &self.sop_instance_uid,
        )
    }

    #[must_use]
    pub fn revised(&self) -> Self {
        let mut revised = self.clone();
        revised.sop_instance_uid = new_dicom_uid();
        revised
    }

    /// Convert binary or label-map pixels into exact pixel-edge rectangle runs.
    ///
    /// Each contiguous horizontal run becomes one editable polygon. This preserves
    /// the raster exactly without requiring ANN-style polygons with holes.
    pub fn vectorized_annotation_groups(&self) -> Result<Vec<AnnotationGroup>> {
        if self.kind == SegmentationKind::Fractional {
            return Err(Error::Unsupported(
                "fractional SEG remains a read-only raster overlay".into(),
            ));
        }
        let frames = self.rasterized_frames()?;
        let mut polygons_by_segment = vec![Vec::new(); self.segments.len()];
        let mut run_count = 0_usize;
        for frame in frames {
            let segment_index = self
                .segment_index_for_number(frame.segment_number)
                .ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "SEG frame references missing segment {}",
                        frame.segment_number
                    ))
                })?;
            let Some(polygons) = polygons_by_segment.get_mut(segment_index) else {
                return Err(Error::InvalidInput(format!(
                    "SEG frame references missing segment {}",
                    frame.segment_number
                )));
            };
            let base_x = frame.tile_col * u32::from(frame.width);
            let base_y = frame.tile_row * u32::from(frame.height);
            for row in 0..usize::from(frame.height) {
                let row_start = row * usize::from(frame.width);
                let row_values = &frame.mask[row_start..row_start + usize::from(frame.width)];
                let mut column = 0_usize;
                while column < row_values.len() {
                    if !row_values[column] {
                        column += 1;
                        continue;
                    }
                    let start = column;
                    while column < row_values.len() && row_values[column] {
                        column += 1;
                    }
                    run_count = run_count.saturating_add(1);
                    if run_count > MAX_VECTORIZED_RUNS {
                        return Err(Error::InvalidInput(format!(
                            "SEG vectorization exceeds the {MAX_VECTORIZED_RUNS}-run editing budget"
                        )));
                    }
                    let x0 = f64::from(base_x) + start as f64;
                    let x1 = f64::from(base_x) + column as f64;
                    let y0 = f64::from(base_y) + row as f64;
                    let y1 = y0 + 1.0;
                    polygons.push(vec![
                        Point2::new(x0, y0),
                        Point2::new(x1, y0),
                        Point2::new(x1, y1),
                        Point2::new(x0, y1),
                    ]);
                }
            }
        }
        self.segments
            .iter()
            .zip(polygons_by_segment)
            .filter(|(_, polygons)| !polygons.is_empty())
            .map(|(segment, polygons)| {
                AnnotationGroup::polygons(
                    segment.label(),
                    segment.category().clone(),
                    segment.property_type().clone(),
                    segment.recommended_display_cielab(),
                    polygons,
                )
            })
            .collect()
    }

    #[must_use]
    pub const fn kind(&self) -> SegmentationKind {
        self.kind
    }

    #[must_use]
    pub const fn editable(&self) -> bool {
        !matches!(self.kind, SegmentationKind::Fractional)
    }

    #[must_use]
    pub fn source(&self) -> &DicomAnnotationContext {
        &self.source
    }

    #[must_use]
    pub fn sop_instance_uid(&self) -> &str {
        &self.sop_instance_uid
    }

    #[must_use]
    pub fn segments(&self) -> &[SegmentationSegment] {
        &self.segments
    }

    #[must_use]
    pub fn series_instance_uid(&self) -> &str {
        &self.series_instance_uid
    }

    #[must_use]
    pub fn content_label(&self) -> &str {
        &self.content_label
    }

    #[must_use]
    pub fn content_description(&self) -> &str {
        &self.content_description
    }

    #[must_use]
    pub fn content_creator_name(&self) -> Option<&str> {
        self.content_creator_name.as_deref()
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[InteroperabilityDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn segment_for_number(&self, number: u16) -> Option<&SegmentationSegment> {
        self.segment_index_for_number(number)
            .and_then(|index| self.segments.get(index))
    }

    fn segment_index_for_number(&self, number: u16) -> Option<usize> {
        self.segments
            .iter()
            .enumerate()
            .position(|(index, segment)| {
                segment
                    .source_segment_number
                    .unwrap_or_else(|| u16::try_from(index + 1).unwrap_or(u16::MAX))
                    == number
            })
    }
}
