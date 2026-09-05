//! Deterministic bounded projection of one Point Frame into a terminal-neutral raster.

use std::{error::Error as StdError, fmt, mem::size_of, sync::Arc};

use crate::core::{
    ByteBound, Determinism, ElementCountRequirement, Error as CoreError, ErrorCategory,
    FieldRequirement, FieldSelector, InputCapabilities, LossPolicy, Materialization,
    MetadataEffect, OperatorBehavior, OperatorContract, OperatorInput, OperatorOutput, Ordering,
    OutputRepresentation, OutputSchema, PipelineMemoryRequirements, Planner, PointCountEffect,
    PointRepresentation, PrimitiveRequirement, ScratchMemory, ValidatedOperatorPipeline,
    ValueEffect,
    point::{
        AccessError, PointBatch, PointColumn, PointDimensions, PointField, PointFieldSemantic,
        PointFieldView, PointFrameMetadata, PointSchema, PointValue, PointView, PrimitiveType,
    },
};

const COORDINATES: [PointFieldSemantic; 3] = [
    PointFieldSemantic::X,
    PointFieldSemantic::Y,
    PointFieldSemantic::Z,
];
/// Depths within this many scaled machine epsilons collide as a tie.
pub const DEPTH_TOLERANCE_ULPS: f64 = 16.0;

/// Source coordinate axis used by the orthographic camera.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoordinateAxis {
    X,
    Y,
    Z,
}

impl CoordinateAxis {
    const fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }
}

/// Direction in which a source axis increases in camera space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AxisDirection {
    Positive,
    Negative,
}

/// One signed source axis in the camera basis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignedAxis {
    axis: CoordinateAxis,
    direction: AxisDirection,
}

impl SignedAxis {
    pub const fn positive(axis: CoordinateAxis) -> Self {
        Self {
            axis,
            direction: AxisDirection::Positive,
        }
    }

    pub const fn negative(axis: CoordinateAxis) -> Self {
        Self {
            axis,
            direction: AxisDirection::Negative,
        }
    }

    pub const fn axis(self) -> CoordinateAxis {
        self.axis
    }

    pub const fn direction(self) -> AxisDirection {
        self.direction
    }

    fn value(self, coordinates: [f64; 3]) -> f64 {
        let value = coordinates[self.axis.index()];
        match self.direction {
            AxisDirection::Positive => value,
            AxisDirection::Negative => -value,
        }
    }
}

/// Axis-aligned orthographic camera basis.
///
/// `right` increases toward larger raster columns, `up` toward smaller raster
/// rows, and `away` is the depth coordinate used by [`DepthPolicy`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrthographicView {
    right: SignedAxis,
    up: SignedAxis,
    away: SignedAxis,
}

impl OrthographicView {
    pub fn new(
        right: SignedAxis,
        up: SignedAxis,
        away: SignedAxis,
    ) -> Result<Self, ProjectionError> {
        if right.axis == up.axis || right.axis == away.axis || up.axis == away.axis {
            return Err(ProjectionError::RepeatedCameraAxis);
        }
        Ok(Self { right, up, away })
    }

    /// X right, Y up, and increasing Z away from the camera.
    pub const fn xy() -> Self {
        Self {
            right: SignedAxis::positive(CoordinateAxis::X),
            up: SignedAxis::positive(CoordinateAxis::Y),
            away: SignedAxis::positive(CoordinateAxis::Z),
        }
    }

    pub const fn right(self) -> SignedAxis {
        self.right
    }

    pub const fn up(self) -> SignedAxis {
        self.up
    }

    pub const fn away(self) -> SignedAxis {
        self.away
    }
}

/// Which depth wins when multiple points address one cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepthPolicy {
    /// The smallest camera-space `away` coordinate wins.
    Nearest,
}

/// Treatment of a point with a NaN or infinity in X, Y, or Z.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidProjectionCoordinatePolicy {
    Drop,
    Reject,
}

/// An output RGB color independent of any terminal escape protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb8(pub [u8; 3]);

/// Finite, strictly increasing intensity range mapped to grayscale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntensityRange {
    minimum: f64,
    maximum: f64,
}

impl IntensityRange {
    pub fn new(minimum: f64, maximum: f64) -> Result<Self, ProjectionError> {
        if !minimum.is_finite() || !maximum.is_finite() || minimum >= maximum {
            return Err(ProjectionError::InvalidIntensityRange);
        }
        Ok(Self { minimum, maximum })
    }

    pub const fn minimum(self) -> f64 {
        self.minimum
    }

    pub const fn maximum(self) -> f64 {
        self.maximum
    }
}

/// Color assigned to an occupied cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorPolicy {
    Uniform(Rgb8),
    /// Scalar Intensity is clamped to `range` and linearly mapped to grayscale.
    /// NaN or infinity receives `invalid`.
    Intensity {
        range: IntensityRange,
        invalid: Rgb8,
    },
}

/// Fully specified deterministic projection policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    dimensions: RasterDimensions,
    view: OrthographicView,
    depth: DepthPolicy,
    invalid_coordinates: InvalidProjectionCoordinatePolicy,
    color: ColorPolicy,
}

impl Projection {
    pub const fn new(
        dimensions: RasterDimensions,
        view: OrthographicView,
        depth: DepthPolicy,
        invalid_coordinates: InvalidProjectionCoordinatePolicy,
        color: ColorPolicy,
    ) -> Self {
        Self {
            dimensions,
            view,
            depth,
            invalid_coordinates,
            color,
        }
    }

    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }

    pub const fn view(self) -> OrthographicView {
        self.view
    }

    pub const fn depth_policy(self) -> DepthPolicy {
        self.depth
    }

    pub const fn invalid_coordinate_policy(self) -> InvalidProjectionCoordinatePolicy {
        self.invalid_coordinates
    }

    pub const fn color_policy(self) -> ColorPolicy {
        self.color
    }

    /// Declares coordinate/color materialization and exact raster scratch.
    pub fn contract(self) -> OperatorContract {
        let mut requirements: Vec<FieldRequirement> = COORDINATES
            .into_iter()
            .map(|semantic| {
                FieldRequirement::new(
                    FieldSelector::semantic(semantic),
                    PrimitiveRequirement::one_of([PrimitiveType::F32, PrimitiveType::F64]),
                    ElementCountRequirement::Exactly(1),
                )
            })
            .collect();
        let mut materialization: Vec<FieldSelector> = COORDINATES
            .into_iter()
            .map(FieldSelector::semantic)
            .collect();
        if matches!(self.color, ColorPolicy::Intensity { .. }) {
            requirements.push(FieldRequirement::new(
                FieldSelector::semantic(PointFieldSemantic::Intensity),
                PrimitiveRequirement::Numeric,
                ElementCountRequirement::Exactly(1),
            ));
            materialization.push(FieldSelector::semantic(PointFieldSemantic::Intensity));
        }
        OperatorContract::new(
            "cpu-projection",
            OperatorInput::new(
                InputCapabilities::ViewOrColumns,
                requirements,
                Materialization::fields(materialization),
            ),
            OperatorOutput::new(
                OutputSchema::Preserve,
                OutputRepresentation::Preserve,
                PointCountEffect::Preserve,
                MetadataEffect::Preserve,
                ValueEffect::Preserve,
            ),
            OperatorBehavior::new(
                [],
                Ordering::Preserve,
                Determinism::Deterministic,
                ScratchMemory::fixed(
                    self.dimensions
                        .storage_bytes()
                        .expect("validated raster dimensions have a representable storage size"),
                ),
            ),
        )
    }

    /// Preflights the projection against the shared operator contract.
    pub fn plan(
        self,
        input_schema: Arc<PointSchema>,
        source_dimensions: PointDimensions,
        input_representation: PointRepresentation,
        loss_policy: &LossPolicy,
    ) -> crate::core::Result<ProjectionPlan> {
        let pipeline = Planner::new().validate_operators(
            input_schema,
            source_dimensions,
            input_representation,
            &[self.contract()],
            loss_policy,
        )?;
        Ok(ProjectionPlan {
            projection: self,
            pipeline,
            input_representation,
            source_dimensions,
        })
    }
}

/// A projection whose schema, representation, materialization, and scratch passed preflight.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionPlan {
    projection: Projection,
    pipeline: ValidatedOperatorPipeline,
    input_representation: PointRepresentation,
    source_dimensions: PointDimensions,
}

impl ProjectionPlan {
    pub const fn pipeline(&self) -> &ValidatedOperatorPipeline {
        &self.pipeline
    }

    pub fn execute_view(&self, input: &PointView) -> Result<Raster, ProjectionError> {
        self.validate_input(
            input.schema(),
            input.layout().dimensions(),
            PointRepresentation::View,
        )?;
        self.projection.project_view(input)
    }

    /// Executes this plan against an already materialized Point Frame.
    pub fn execute_batch(&self, input: &PointBatch) -> Result<Raster, ProjectionError> {
        self.validate_input(
            input.schema(),
            input.dimensions(),
            PointRepresentation::Columns,
        )?;
        self.projection.project_batch(input)
    }

    /// Combines the operator declaration with retained input and downstream buffers.
    pub fn memory_requirements_for_view(
        &self,
        input: &PointView,
        encoder_buffer: ByteBound,
        output_buffer: ByteBound,
        queued_output: ByteBound,
    ) -> crate::core::Result<PipelineMemoryRequirements> {
        self.validate_input(
            input.schema(),
            input.layout().dimensions(),
            PointRepresentation::View,
        )
        .map_err(|error| CoreError::new(ErrorCategory::Unsupported, error.to_string()))?;
        PipelineMemoryRequirements::for_operator_pipeline(
            input,
            &self.pipeline,
            encoder_buffer,
            output_buffer,
            queued_output,
        )
    }

    fn validate_input(
        &self,
        schema: &PointSchema,
        dimensions: PointDimensions,
        representation: PointRepresentation,
    ) -> Result<(), ProjectionError> {
        let stage = self
            .pipeline
            .stages()
            .first()
            .expect("a projection plan always contains one stage");
        if schema != stage.input_schema()
            || dimensions != self.source_dimensions
            || representation != self.input_representation
        {
            return Err(ProjectionError::InputDoesNotMatchPlan);
        }
        Ok(())
    }
}

/// Camera-space bounds fitted into the raster while preserving aspect ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectionBounds {
    minimum: [f64; 3],
    maximum: [f64; 3],
}

impl ProjectionBounds {
    pub const fn minimum(self) -> [f64; 3] {
        self.minimum
    }

    pub const fn maximum(self) -> [f64; 3] {
        self.maximum
    }
}

/// Terminal-neutral z-buffer raster for one Point Frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    dimensions: RasterDimensions,
    pixels: Box<[Option<RasterPixel>]>,
    bounds: Option<ProjectionBounds>,
    schema: Arc<PointSchema>,
    metadata: Arc<PointFrameMetadata>,
    source_dimensions: PointDimensions,
}

impl Raster {
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    pub fn pixels(&self) -> &[Option<RasterPixel>] {
        &self.pixels
    }

    pub fn pixel(&self, row: usize, column: usize) -> Option<&RasterPixel> {
        if row >= self.dimensions.height || column >= self.dimensions.width {
            return None;
        }
        self.pixels[row * self.dimensions.width + column].as_ref()
    }

    pub const fn bounds(&self) -> Option<ProjectionBounds> {
        self.bounds
    }

    pub fn schema(&self) -> &PointSchema {
        &self.schema
    }

    pub fn metadata(&self) -> &PointFrameMetadata {
        &self.metadata
    }

    pub const fn source_dimensions(&self) -> PointDimensions {
        self.source_dimensions
    }

    pub fn occupied_pixel_count(&self) -> usize {
        self.pixels.iter().filter(|pixel| pixel.is_some()).count()
    }

    #[cfg(test)]
    fn golden_cells(&self) -> String {
        let mut output = String::new();
        for row in 0..self.dimensions.height {
            for column in 0..self.dimensions.width {
                match self.pixel(row, column) {
                    Some(pixel) => {
                        output.push('#');
                        output.push(char::from(b'0' + pixel.source_point_index as u8));
                    }
                    None => output.push('.'),
                }
            }
            output.push('\n');
        }
        output
    }
}

impl RasterPixel {
    pub const fn source_point_index(self) -> usize {
        self.source_point_index
    }

    pub const fn depth(self) -> f64 {
        self.depth
    }

    pub const fn color(self) -> Rgb8 {
        Rgb8(self.color)
    }
}

impl Projection {
    fn project_view(self, input: &PointView) -> Result<Raster, ProjectionError> {
        let coordinate_fields = coordinate_fields(input.schema())?.map(|field| {
            input
                .field(field.name())
                .expect("coordinate belongs to the validated view")
        });
        let intensity = match self.color {
            ColorPolicy::Uniform(_) => None,
            ColorPolicy::Intensity { .. } => {
                let field = semantic_field(input.schema(), PointFieldSemantic::Intensity)?;
                Some(
                    input
                        .field(field.name())
                        .expect("intensity belongs to the validated view"),
                )
            }
        };
        let mut bounds = None;
        for point in 0..input.layout().dimensions().point_count() {
            let Some(camera) = self.camera_point(coordinate_fields, point)? else {
                continue;
            };
            bounds = Some(expand_bounds(bounds, camera));
        }

        let mut pixels = allocate_pixels(self.dimensions)?;

        if let Some(fitted) = bounds {
            for point in 0..input.layout().dimensions().point_count() {
                let Some(camera) = self.camera_point(coordinate_fields, point)? else {
                    continue;
                };
                let (row, column) = map_to_pixel(camera, fitted, self.dimensions);
                let color = self.point_color(intensity, point)?;
                let candidate = RasterPixel {
                    source_point_index: point,
                    depth: canonical_zero(camera[2]),
                    color: color.0,
                };
                let cell = &mut pixels[row * self.dimensions.width + column];
                if cell
                    .as_ref()
                    .is_none_or(|current| self.depth_precedes(candidate.depth, current.depth))
                {
                    *cell = Some(candidate);
                }
            }
        }

        Ok(Raster {
            dimensions: self.dimensions,
            pixels: pixels.into_boxed_slice(),
            bounds,
            schema: input.shared_schema(),
            metadata: input.shared_metadata(),
            source_dimensions: input.layout().dimensions(),
        })
    }

    fn project_batch(self, input: &PointBatch) -> Result<Raster, ProjectionError> {
        let coordinate_indices = coordinate_fields(input.schema())?.map(|field| {
            input
                .schema()
                .fields()
                .iter()
                .position(|candidate| candidate.name() == field.name())
                .expect("coordinate belongs to the validated batch")
        });
        let intensity_index = match self.color {
            ColorPolicy::Uniform(_) => None,
            ColorPolicy::Intensity { .. } => {
                let field = semantic_field(input.schema(), PointFieldSemantic::Intensity)?;
                Some(
                    input
                        .schema()
                        .fields()
                        .iter()
                        .position(|candidate| candidate.name() == field.name())
                        .expect("intensity belongs to the validated batch"),
                )
            }
        };
        let mut bounds = None;
        for point in 0..input.dimensions().point_count() {
            let Some(camera) = self.camera_point_from_columns(input, coordinate_indices, point)?
            else {
                continue;
            };
            bounds = Some(expand_bounds(bounds, camera));
        }

        let mut pixels = allocate_pixels(self.dimensions)?;
        if let Some(fitted) = bounds {
            for point in 0..input.dimensions().point_count() {
                let Some(camera) =
                    self.camera_point_from_columns(input, coordinate_indices, point)?
                else {
                    continue;
                };
                let (row, column) = map_to_pixel(camera, fitted, self.dimensions);
                let color = match self.color {
                    ColorPolicy::Uniform(color) => color,
                    ColorPolicy::Intensity { range, invalid } => intensity_color(
                        column_numeric_value(
                            &input.columns()[intensity_index.expect("required by color policy")],
                            point,
                        ),
                        range,
                        invalid,
                    ),
                };
                let candidate = RasterPixel {
                    source_point_index: point,
                    depth: canonical_zero(camera[2]),
                    color: color.0,
                };
                let cell = &mut pixels[row * self.dimensions.width + column];
                if cell
                    .as_ref()
                    .is_none_or(|current| self.depth_precedes(candidate.depth, current.depth))
                {
                    *cell = Some(candidate);
                }
            }
        }

        Ok(Raster {
            dimensions: self.dimensions,
            pixels: pixels.into_boxed_slice(),
            bounds,
            schema: input.shared_schema(),
            metadata: input.metadata_handle(),
            source_dimensions: input.dimensions(),
        })
    }

    fn camera_point_from_columns(
        self,
        input: &PointBatch,
        columns: [usize; 3],
        point: usize,
    ) -> Result<Option<[f64; 3]>, ProjectionError> {
        let coordinates =
            columns.map(|column| column_floating_value(&input.columns()[column], point));
        if coordinates.into_iter().any(|value| !value.is_finite()) {
            return match self.invalid_coordinates {
                InvalidProjectionCoordinatePolicy::Drop => Ok(None),
                InvalidProjectionCoordinatePolicy::Reject => {
                    Err(ProjectionError::NonFiniteCoordinate { point })
                }
            };
        }
        Ok(Some([
            canonical_zero(self.view.right.value(coordinates)),
            canonical_zero(self.view.up.value(coordinates)),
            canonical_zero(self.view.away.value(coordinates)),
        ]))
    }

    fn camera_point(
        self,
        fields: [PointFieldView<'_>; 3],
        point: usize,
    ) -> Result<Option<[f64; 3]>, ProjectionError> {
        let coordinates = [
            floating_value(fields[0], point)?,
            floating_value(fields[1], point)?,
            floating_value(fields[2], point)?,
        ];
        if coordinates.into_iter().any(|value| !value.is_finite()) {
            return match self.invalid_coordinates {
                InvalidProjectionCoordinatePolicy::Drop => Ok(None),
                InvalidProjectionCoordinatePolicy::Reject => {
                    Err(ProjectionError::NonFiniteCoordinate { point })
                }
            };
        }
        Ok(Some([
            canonical_zero(self.view.right.value(coordinates)),
            canonical_zero(self.view.up.value(coordinates)),
            canonical_zero(self.view.away.value(coordinates)),
        ]))
    }

    fn point_color(
        self,
        intensity: Option<PointFieldView<'_>>,
        point: usize,
    ) -> Result<Rgb8, ProjectionError> {
        match self.color {
            ColorPolicy::Uniform(color) => Ok(color),
            ColorPolicy::Intensity { range, invalid } => {
                let value = numeric_value(intensity.expect("required by intensity policy"), point)?;
                Ok(intensity_color(value, range, invalid))
            }
        }
    }

    fn depth_precedes(self, candidate: f64, current: f64) -> bool {
        match self.depth {
            DepthPolicy::Nearest => {
                let scale = candidate.abs().max(current.abs()).max(1.0);
                let tolerance = DEPTH_TOLERANCE_ULPS * f64::EPSILON * scale;
                candidate < current - tolerance
            }
        }
    }
}

fn allocate_pixels(
    dimensions: RasterDimensions,
) -> Result<Vec<Option<RasterPixel>>, ProjectionError> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(dimensions.pixel_count)
        .map_err(|_| ProjectionError::AllocationFailed {
            requested: dimensions.storage_bytes().unwrap_or(u64::MAX),
        })?;
    pixels.resize(dimensions.pixel_count, None);
    Ok(pixels)
}

fn column_floating_value(column: &PointColumn, point: usize) -> f64 {
    match column {
        PointColumn::F32(values) => f64::from(values[point]),
        PointColumn::F64(values) => values[point],
        _ => unreachable!("coordinate column primitive was validated by the plan"),
    }
}

fn column_numeric_value(column: &PointColumn, point: usize) -> f64 {
    match column {
        PointColumn::I8(values) => f64::from(values[point]),
        PointColumn::U8(values) => f64::from(values[point]),
        PointColumn::I16(values) => f64::from(values[point]),
        PointColumn::U16(values) => f64::from(values[point]),
        PointColumn::I32(values) => f64::from(values[point]),
        PointColumn::U32(values) => f64::from(values[point]),
        PointColumn::I64(values) => values[point] as f64,
        PointColumn::U64(values) => values[point] as f64,
        PointColumn::F32(values) => f64::from(values[point]),
        PointColumn::F64(values) => values[point],
    }
}

fn intensity_color(value: f64, range: IntensityRange, invalid: Rgb8) -> Rgb8 {
    if !value.is_finite() {
        return invalid;
    }
    let normalized = (scaled_difference(value, range.minimum)
        / scaled_difference(range.maximum, range.minimum))
    .clamp(0.0, 1.0);
    let gray = (normalized * 255.0).round() as u8;
    Rgb8([gray; 3])
}

fn coordinate_fields(schema: &PointSchema) -> Result<[&PointField; 3], ProjectionError> {
    Ok([
        semantic_field(schema, PointFieldSemantic::X)?,
        semantic_field(schema, PointFieldSemantic::Y)?,
        semantic_field(schema, PointFieldSemantic::Z)?,
    ])
}

fn semantic_field(
    schema: &PointSchema,
    semantic: PointFieldSemantic,
) -> Result<&PointField, ProjectionError> {
    let mut matches = schema
        .fields()
        .iter()
        .filter(|field| field.semantic() == Some(semantic));
    let field = matches
        .next()
        .ok_or(ProjectionError::MissingField { semantic })?;
    if matches.next().is_some() {
        return Err(ProjectionError::AmbiguousField { semantic });
    }
    Ok(field)
}

fn floating_value(field: PointFieldView<'_>, point: usize) -> Result<f64, ProjectionError> {
    match field.value(point, 0).map_err(ProjectionError::Access)? {
        PointValue::F32(value) => Ok(f64::from(value)),
        PointValue::F64(value) => Ok(value),
        _ => Err(ProjectionError::UnsupportedFieldType {
            name: field.field().name().to_owned(),
            primitive: field.field().primitive(),
        }),
    }
}

fn numeric_value(field: PointFieldView<'_>, point: usize) -> Result<f64, ProjectionError> {
    Ok(
        match field.value(point, 0).map_err(ProjectionError::Access)? {
            PointValue::I8(value) => f64::from(value),
            PointValue::U8(value) => f64::from(value),
            PointValue::I16(value) => f64::from(value),
            PointValue::U16(value) => f64::from(value),
            PointValue::I32(value) => f64::from(value),
            PointValue::U32(value) => f64::from(value),
            PointValue::I64(value) => value as f64,
            PointValue::U64(value) => value as f64,
            PointValue::F32(value) => f64::from(value),
            PointValue::F64(value) => value,
        },
    )
}

fn expand_bounds(bounds: Option<ProjectionBounds>, point: [f64; 3]) -> ProjectionBounds {
    match bounds {
        Some(bounds) => ProjectionBounds {
            minimum: std::array::from_fn(|axis| bounds.minimum[axis].min(point[axis])),
            maximum: std::array::from_fn(|axis| bounds.maximum[axis].max(point[axis])),
        },
        None => ProjectionBounds {
            minimum: point,
            maximum: point,
        },
    }
}

fn map_to_pixel(
    point: [f64; 3],
    bounds: ProjectionBounds,
    dimensions: RasterDimensions,
) -> (usize, usize) {
    let horizontal_span = scaled_difference(bounds.maximum[0], bounds.minimum[0]);
    let vertical_span = scaled_difference(bounds.maximum[1], bounds.minimum[1]);
    let horizontal_pixels = (dimensions.width - 1) as f64;
    let vertical_pixels = (dimensions.height - 1) as f64;
    let (used_width, used_height) = if horizontal_pixels == 0.0 && vertical_pixels == 0.0 {
        (0.0, 0.0)
    } else if horizontal_pixels == 0.0 {
        (0.0, vertical_pixels)
    } else if vertical_pixels == 0.0 {
        (horizontal_pixels, 0.0)
    } else {
        match (horizontal_span > 0.0, vertical_span > 0.0) {
            (true, true) => {
                let largest_span = horizontal_span.max(vertical_span);
                let horizontal_ratio = horizontal_span / largest_span;
                let vertical_ratio = vertical_span / largest_span;
                let pixel_scale =
                    (horizontal_pixels / horizontal_ratio).min(vertical_pixels / vertical_ratio);
                (horizontal_ratio * pixel_scale, vertical_ratio * pixel_scale)
            }
            (true, false) => (horizontal_pixels, 0.0),
            (false, true) => (0.0, vertical_pixels),
            (false, false) => (0.0, 0.0),
        }
    };
    let left = (horizontal_pixels - used_width) * 0.5;
    let bottom = (vertical_pixels - used_height) * 0.5;
    let column = if horizontal_span == 0.0 {
        (horizontal_pixels * 0.5).round()
    } else {
        (left + scaled_difference(point[0], bounds.minimum[0]) / horizontal_span * used_width)
            .round()
    }
    .clamp(0.0, horizontal_pixels) as usize;
    let from_bottom = if vertical_span == 0.0 {
        (vertical_pixels * 0.5).round()
    } else {
        (bottom + scaled_difference(point[1], bounds.minimum[1]) / vertical_span * used_height)
            .round()
    }
    .clamp(0.0, vertical_pixels) as usize;
    (dimensions.height - 1 - from_bottom, column)
}

fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

fn scaled_difference(left: f64, right: f64) -> f64 {
    let direct = left - right;
    if direct.is_finite() {
        direct
    } else {
        left * 0.5 - right * 0.5
    }
}

/// Non-zero pixel dimensions for a projected raster.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RasterDimensions {
    width: usize,
    height: usize,
    pixel_count: usize,
}

impl RasterDimensions {
    pub fn new(width: usize, height: usize) -> Result<Self, ProjectionError> {
        if width == 0 || height == 0 {
            return Err(ProjectionError::ZeroRasterDimension);
        }
        let pixel_count = width
            .checked_mul(height)
            .ok_or(ProjectionError::RasterSizeOverflow)?;
        let dimensions = Self {
            width,
            height,
            pixel_count,
        };
        dimensions.storage_bytes()?;
        Ok(dimensions)
    }

    pub const fn width(self) -> usize {
        self.width
    }

    pub const fn height(self) -> usize {
        self.height
    }

    pub const fn pixel_count(self) -> usize {
        self.pixel_count
    }

    /// Exact allocation used by the raster pixel array on this target.
    pub fn storage_bytes(self) -> Result<u64, ProjectionError> {
        let bytes = self
            .pixel_count
            .checked_mul(size_of::<Option<RasterPixel>>())
            .ok_or(ProjectionError::RasterSizeOverflow)?;
        u64::try_from(bytes).map_err(|_| ProjectionError::RasterSizeOverflow)
    }
}

/// One occupied raster cell. Empty cells are represented by `None`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterPixel {
    source_point_index: usize,
    depth: f64,
    color: [u8; 3],
}

/// A deterministic projection validation or execution failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    ZeroRasterDimension,
    RasterSizeOverflow,
    RepeatedCameraAxis,
    InvalidIntensityRange,
    InputDoesNotMatchPlan,
    MissingField {
        semantic: PointFieldSemantic,
    },
    AmbiguousField {
        semantic: PointFieldSemantic,
    },
    UnsupportedFieldType {
        name: String,
        primitive: PrimitiveType,
    },
    NonFiniteCoordinate {
        point: usize,
    },
    AllocationFailed {
        requested: u64,
    },
    Access(AccessError),
}

impl ProjectionError {
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::ZeroRasterDimension | Self::RepeatedCameraAxis | Self::InvalidIntensityRange => {
                ErrorCategory::Usage
            }
            Self::RasterSizeOverflow | Self::AllocationFailed { .. } => ErrorCategory::Resource,
            Self::InputDoesNotMatchPlan
            | Self::MissingField { .. }
            | Self::AmbiguousField { .. }
            | Self::UnsupportedFieldType { .. } => ErrorCategory::Unsupported,
            Self::NonFiniteCoordinate { .. } => ErrorCategory::InvalidData,
            Self::Access(_) => ErrorCategory::Internal,
        }
    }
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroRasterDimension => {
                formatter.write_str("projection raster width and height must be non-zero")
            }
            Self::RasterSizeOverflow => formatter.write_str("projection raster size overflowed"),
            Self::RepeatedCameraAxis => formatter.write_str(
                "projection camera right, up, and away directions must use distinct axes",
            ),
            Self::InvalidIntensityRange => formatter.write_str(
                "projection intensity range must be finite and strictly increasing",
            ),
            Self::InputDoesNotMatchPlan => formatter.write_str(
                "projection input does not match its validated Point Schema, dimensions, and representation",
            ),
            Self::MissingField { semantic } => {
                write!(formatter, "projection field semantic {semantic:?} is missing")
            }
            Self::AmbiguousField { semantic } => {
                write!(formatter, "projection field semantic {semantic:?} is ambiguous")
            }
            Self::UnsupportedFieldType { name, primitive } => write!(
                formatter,
                "projection field {name:?} has unsupported primitive {primitive:?}",
            ),
            Self::NonFiniteCoordinate { point } => {
                write!(formatter, "projection point {point} has a non-finite coordinate")
            }
            Self::AllocationFailed { requested } => write!(
                formatter,
                "projection raster allocation of {requested} bytes failed within its planned bound",
            ),
            Self::Access(error) => write!(formatter, "projection field access failed: {error}"),
        }
    }
}

impl StdError for ProjectionError {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;

    use super::*;
    use crate::core::point::{
        Endianness, MaterializationBudget, PointDimensions, PointField, PointFieldSemantic,
        PointFrameMetadata, PointLayout, PointSchema, PointView, PrimitiveType, Timestamp,
    };
    use crate::core::{
        ByteBound, Destination, FrameSelector, JobSpec, LossPolicy, Planner, PointRepresentation,
        SourceSpec,
    };

    fn synthetic_view() -> PointView {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F64, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F64, 1, Some(PointFieldSemantic::Y)).unwrap(),
                PointField::new("z", PrimitiveType::F64, 1, Some(PointFieldSemantic::Z)).unwrap(),
                PointField::new(
                    "intensity",
                    PrimitiveType::F32,
                    1,
                    Some(PointFieldSemantic::Intensity),
                )
                .unwrap(),
                PointField::new("id", PrimitiveType::U16, 1, None).unwrap(),
            ])
            .unwrap(),
        );
        let points: [(f64, f64, f64, f32, u16); 5] = [
            (-1.0, -1.0, 5.0, 0.0_f32, 10_u16),
            (1.0, 1.0, 4.0, 100.0, 11),
            (0.0, 0.0, 10.0, 50.0, 12),
            (0.0, 0.0, 2.0, 75.0, 13),
            (0.5, -0.5, 3.0, f32::NAN, 14),
        ];
        let mut source = Vec::new();
        for (x, y, z, intensity, id) in points {
            source.extend_from_slice(&x.to_le_bytes());
            source.extend_from_slice(&y.to_le_bytes());
            source.extend_from_slice(&z.to_le_bytes());
            source.extend_from_slice(&intensity.to_le_bytes());
            source.extend_from_slice(&id.to_le_bytes());
        }
        let dimensions = PointDimensions::new(points.len(), 1).unwrap();
        let layout = PointLayout::new(
            schema,
            dimensions,
            vec![0, 8, 16, 24, 28],
            30,
            30 * points.len(),
            0,
            Endianness::Little,
        )
        .unwrap();
        PointView::new(
            Arc::from(source),
            Arc::new(
                PointFrameMetadata::new(Timestamp::new(7, 8).unwrap(), "lidar", false)
                    .with_container_times(9, 10),
            ),
            layout,
        )
        .unwrap()
    }

    fn xyz_view(points: &[[f64; 3]]) -> PointView {
        let schema = Arc::new(
            PointSchema::new(vec![
                PointField::new("x", PrimitiveType::F64, 1, Some(PointFieldSemantic::X)).unwrap(),
                PointField::new("y", PrimitiveType::F64, 1, Some(PointFieldSemantic::Y)).unwrap(),
                PointField::new("z", PrimitiveType::F64, 1, Some(PointFieldSemantic::Z)).unwrap(),
            ])
            .unwrap(),
        );
        let mut source = Vec::new();
        for coordinates in points {
            for value in coordinates {
                source.extend_from_slice(&value.to_le_bytes());
            }
        }
        let dimensions = PointDimensions::new(points.len(), 1).unwrap();
        let layout = PointLayout::new(
            schema,
            dimensions,
            vec![0, 8, 16],
            24,
            24 * points.len(),
            0,
            Endianness::Little,
        )
        .unwrap();
        PointView::new(
            Arc::from(source),
            Arc::new(PointFrameMetadata::new(
                Timestamp::new(1, 2).unwrap(),
                "map",
                false,
            )),
            layout,
        )
        .unwrap()
    }

    fn uniform_projection(
        dimensions: RasterDimensions,
        invalid_coordinates: InvalidProjectionCoordinatePolicy,
    ) -> Projection {
        Projection::new(
            dimensions,
            OrthographicView::xy(),
            DepthPolicy::Nearest,
            invalid_coordinates,
            ColorPolicy::Uniform(Rgb8([1, 2, 3])),
        )
    }

    fn project_xyz(input: &PointView, projection: Projection) -> Result<Raster, ProjectionError> {
        projection
            .plan(
                input.shared_schema(),
                input.layout().dimensions(),
                PointRepresentation::View,
                &LossPolicy::lossless(),
            )
            .unwrap()
            .execute_view(input)
    }

    #[test]
    fn raster_dimensions_reject_zero_and_preflight_exact_pixel_storage() {
        assert!(matches!(
            RasterDimensions::new(0, 4),
            Err(ProjectionError::ZeroRasterDimension)
        ));
        assert!(matches!(
            RasterDimensions::new(usize::MAX, 1),
            Err(ProjectionError::RasterSizeOverflow)
        ));

        let dimensions = RasterDimensions::new(7, 3).unwrap();
        assert_eq!(dimensions.width(), 7);
        assert_eq!(dimensions.height(), 3);
        assert_eq!(dimensions.pixel_count(), 21);
        assert_eq!(
            dimensions.storage_bytes().unwrap(),
            21 * std::mem::size_of::<Option<RasterPixel>>() as u64
        );
    }

    #[test]
    fn camera_axes_and_intensity_range_are_validated_before_planning() {
        assert!(matches!(
            OrthographicView::new(
                SignedAxis::positive(CoordinateAxis::X),
                SignedAxis::negative(CoordinateAxis::X),
                SignedAxis::positive(CoordinateAxis::Z),
            ),
            Err(ProjectionError::RepeatedCameraAxis)
        ));
        assert!(matches!(
            IntensityRange::new(1.0, 1.0),
            Err(ProjectionError::InvalidIntensityRange)
        ));
        assert_eq!(OrthographicView::xy().up().axis(), CoordinateAxis::Y);
    }

    #[test]
    fn synthetic_projection_matches_reviewed_golden_and_preserves_frame_identity() {
        let input = synthetic_view();
        let projection = Projection::new(
            RasterDimensions::new(5, 3).unwrap(),
            OrthographicView::xy(),
            DepthPolicy::Nearest,
            InvalidProjectionCoordinatePolicy::Drop,
            ColorPolicy::Intensity {
                range: IntensityRange::new(0.0, 100.0).unwrap(),
                invalid: Rgb8([255, 0, 255]),
            },
        );
        let plan = projection
            .plan(
                input.shared_schema(),
                input.layout().dimensions(),
                PointRepresentation::View,
                &LossPolicy::lossless(),
            )
            .unwrap();
        let raster = plan.execute_view(&input).unwrap();

        assert_eq!(raster.schema(), input.schema());
        assert_eq!(raster.metadata(), input.metadata());
        assert_eq!(raster.source_dimensions(), input.layout().dimensions());
        assert_eq!(raster.occupied_pixel_count(), 4);
        assert_eq!(raster.pixel(1, 2).unwrap().source_point_index(), 3);
        assert_eq!(raster.pixel(1, 2).unwrap().color(), Rgb8([191, 191, 191]));
        assert_eq!(raster.pixel(1, 3).unwrap().color(), Rgb8([255, 0, 255]));
        assert_eq!(
            raster.golden_cells(),
            include_str!("../../tests/golden/projection_cells.txt")
        );
    }

    #[test]
    fn view_and_materialized_batch_produce_identical_rasters() {
        let input = synthetic_view();
        let batch = input
            .materialize(MaterializationBudget::new(usize::MAX))
            .unwrap();
        let projection = Projection::new(
            RasterDimensions::new(5, 3).unwrap(),
            OrthographicView::xy(),
            DepthPolicy::Nearest,
            InvalidProjectionCoordinatePolicy::Drop,
            ColorPolicy::Uniform(Rgb8([4, 5, 6])),
        );
        let view_plan = projection
            .plan(
                input.shared_schema(),
                input.layout().dimensions(),
                PointRepresentation::View,
                &LossPolicy::lossless(),
            )
            .unwrap();
        let batch_plan = projection
            .plan(
                batch.shared_schema(),
                batch.dimensions(),
                PointRepresentation::Columns,
                &LossPolicy::lossless(),
            )
            .unwrap();

        assert_eq!(
            batch_plan.execute_batch(&batch).unwrap(),
            view_plan.execute_view(&input).unwrap()
        );
    }

    #[test]
    fn raster_materialization_and_scratch_feed_the_managed_memory_planner() {
        let input = synthetic_view();
        let projection = Projection::new(
            RasterDimensions::new(80, 24).unwrap(),
            OrthographicView::xy(),
            DepthPolicy::Nearest,
            InvalidProjectionCoordinatePolicy::Drop,
            ColorPolicy::Intensity {
                range: IntensityRange::new(0.0, 100.0).unwrap(),
                invalid: Rgb8([0, 0, 0]),
            },
        );
        let plan = projection
            .plan(
                input.shared_schema(),
                input.layout().dimensions(),
                PointRepresentation::View,
                &LossPolicy::lossless(),
            )
            .unwrap();
        assert_eq!(
            plan.pipeline().stages()[0].materialized_fields(),
            &["x", "y", "z", "intensity"]
        );
        assert_eq!(
            plan.pipeline().peak_scratch_bytes(),
            projection.dimensions().storage_bytes().unwrap()
        );

        let requirements = plan
            .memory_requirements_for_view(
                &input,
                ByteBound::bounded(256),
                ByteBound::bounded(128),
                ByteBound::bounded(64),
            )
            .unwrap();
        let job = JobSpec::extract(
            SourceSpec::file("frame.mcap").unwrap(),
            "/points",
            FrameSelector::Index(0),
            Destination::stdout(),
        )
        .unwrap();
        let initial = Planner::new()
            .plan(job.clone(), requirements, u64::MAX)
            .unwrap();
        let peak = initial.memory().peak_bytes();
        let breakdown = initial.memory().breakdown();
        assert_eq!(
            breakdown.operator_scratch_bytes(),
            projection.dimensions().storage_bytes().unwrap()
        );
        assert!(breakdown.materialization_bytes() > 0);
        assert!(Planner::new().plan(job, requirements, peak - 1).is_err());
    }

    #[test]
    fn degenerate_bounds_center_points_and_all_invalid_frames_stay_empty() {
        let one_point = xyz_view(&[[f64::MAX, -f64::MAX, -0.0]]);
        let raster = project_xyz(
            &one_point,
            uniform_projection(
                RasterDimensions::new(4, 4).unwrap(),
                InvalidProjectionCoordinatePolicy::Drop,
            ),
        )
        .unwrap();
        assert_eq!(raster.pixel(1, 2).unwrap().source_point_index(), 0);
        assert_eq!(
            raster.pixel(1, 2).unwrap().depth().to_bits(),
            0.0_f64.to_bits()
        );

        let invalid = xyz_view(&[[f64::NAN, 0.0, 0.0], [0.0, f64::INFINITY, 0.0]]);
        let empty = project_xyz(
            &invalid,
            uniform_projection(
                RasterDimensions::new(4, 4).unwrap(),
                InvalidProjectionCoordinatePolicy::Drop,
            ),
        )
        .unwrap();
        assert_eq!(empty.bounds(), None);
        assert_eq!(empty.occupied_pixel_count(), 0);
    }

    #[test]
    fn a_single_pixel_axis_does_not_collapse_the_other_axis() {
        let input = xyz_view(&[[-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]]);
        let raster = project_xyz(
            &input,
            uniform_projection(
                RasterDimensions::new(1, 5).unwrap(),
                InvalidProjectionCoordinatePolicy::Drop,
            ),
        )
        .unwrap();

        assert_eq!(raster.pixel(0, 0).unwrap().source_point_index(), 1);
        assert_eq!(raster.pixel(4, 0).unwrap().source_point_index(), 0);
    }

    #[test]
    fn invalid_rejection_and_depth_tolerance_are_input_order_stable() {
        let invalid = xyz_view(&[[0.0, 0.0, f64::NAN]]);
        assert!(matches!(
            project_xyz(
                &invalid,
                uniform_projection(
                    RasterDimensions::new(1, 1).unwrap(),
                    InvalidProjectionCoordinatePolicy::Reject,
                ),
            ),
            Err(ProjectionError::NonFiniteCoordinate { point: 0 })
        ));

        let tied = xyz_view(&[[0.0, 0.0, 1.0], [0.0, 0.0, 1.0 - 8.0 * f64::EPSILON]]);
        let tied_raster = project_xyz(
            &tied,
            uniform_projection(
                RasterDimensions::new(1, 1).unwrap(),
                InvalidProjectionCoordinatePolicy::Drop,
            ),
        )
        .unwrap();
        assert_eq!(tied_raster.pixel(0, 0).unwrap().source_point_index(), 0);

        let nearer = xyz_view(&[[0.0, 0.0, 1.0], [0.0, 0.0, 1.0 - 32.0 * f64::EPSILON]]);
        let nearer_raster = project_xyz(
            &nearer,
            uniform_projection(
                RasterDimensions::new(1, 1).unwrap(),
                InvalidProjectionCoordinatePolicy::Drop,
            ),
        )
        .unwrap();
        assert_eq!(nearer_raster.pixel(0, 0).unwrap().source_point_index(), 1);
    }

    #[test]
    fn signed_camera_axes_control_orientation_and_near_direction() {
        let input = xyz_view(&[
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 2.0],
        ]);
        let camera = OrthographicView::new(
            SignedAxis::negative(CoordinateAxis::X),
            SignedAxis::positive(CoordinateAxis::Y),
            SignedAxis::negative(CoordinateAxis::Z),
        )
        .unwrap();
        let projection = Projection::new(
            RasterDimensions::new(3, 1).unwrap(),
            camera,
            DepthPolicy::Nearest,
            InvalidProjectionCoordinatePolicy::Drop,
            ColorPolicy::Uniform(Rgb8([1, 2, 3])),
        );
        let raster = project_xyz(&input, projection).unwrap();

        assert_eq!(raster.pixel(0, 0).unwrap().source_point_index(), 1);
        assert_eq!(raster.pixel(0, 1).unwrap().source_point_index(), 3);
        assert_eq!(raster.pixel(0, 2).unwrap().source_point_index(), 0);
    }

    #[test]
    fn projection_satisfies_the_shared_frame_local_contract() {
        use crate::ops::contract_tests::{ContractTestCase, assert_frame_local_contract};

        let input = synthetic_view();
        let projection = uniform_projection(
            RasterDimensions::new(9, 5).unwrap(),
            InvalidProjectionCoordinatePolicy::Drop,
        );
        assert_frame_local_contract(ContractTestCase {
            contract: projection.contract(),
            accepted_schema: input.shared_schema(),
            rejected_schema: Some(Arc::new(PointSchema::new(vec![]).unwrap())),
            dimensions: input.layout().dimensions(),
            input_representation: PointRepresentation::View,
            authorized_losses: &[],
            expected_output_fields: &["x", "y", "z", "intensity", "id"],
            expected_materialized_fields: &["x", "y", "z"],
            expected_scratch_bytes: projection.dimensions().storage_bytes().unwrap(),
            expected_output_representation: PointRepresentation::View,
            expected_point_count: PointCountEffect::Preserve,
            expected_ordering: Ordering::Preserve,
        });
    }

    proptest! {
        #[test]
        fn projection_is_repeatable_bounded_and_never_invents_source_indices(
            points in prop::collection::vec(
                (any::<f64>(), any::<f64>(), any::<f64>()),
                0..64,
            )
        ) {
            let points: Vec<[f64; 3]> = points.into_iter().map(|(x, y, z)| [x, y, z]).collect();
            let input = xyz_view(&points);
            let projection = uniform_projection(
                RasterDimensions::new(17, 9).unwrap(),
                InvalidProjectionCoordinatePolicy::Drop,
            );
            let first = project_xyz(&input, projection).unwrap();
            let second = project_xyz(&input, projection).unwrap();

            prop_assert_eq!(&first, &second);
            prop_assert_eq!(first.pixels().len(), 17 * 9);
            prop_assert!(first.occupied_pixel_count() <= points.len());
            for pixel in first.pixels().iter().flatten() {
                prop_assert!(pixel.source_point_index() < points.len());
                prop_assert!(pixel.depth().is_finite());
            }
        }
    }
}
