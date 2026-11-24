// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{collections::BTreeMap, path::Path};

use dp2_core::{DepthError, DepthEstimator, DepthMap, Dimensions, VideoFrame};
use half::f16;
use ndarray::{Array1, ArrayD, IxDyn};
use ort::{
    environment::Environment,
    session::{Session, SessionInputValue, builder::GraphOptimizationLevel},
    value::{DynValue, Tensor, TensorElementType, ValueType},
};
use tracing::{debug, info, info_span};

use crate::{
    DepthPro2Config, FocalLengthPolicy, default_ort_dylib_name, focal_length_from_fov,
    frame_to_nchw_f32, resize_depth_bilinear, scale_focal_length,
};

/// Element type of a streaming temporal state tensor carried between frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateDtype {
    /// 32-bit IEEE floating point.
    F32,
    /// 16-bit IEEE floating point.
    F16,
    /// 64-bit signed integer.
    I64,
    /// Boolean flag tensor.
    Bool,
}

/// Description of one streaming state tensor pair (`past_*` input ->
/// `present_*` output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTensorSpec {
    /// Name of the input tensor carrying previous-frame state.
    pub past_name: String,
    /// Name of the output tensor returning updated state.
    pub present_name: String,
    /// Concrete shape of the tensor.
    pub shape: Vec<usize>,
    /// Element type of the tensor.
    pub dtype: StateDtype,
}

/// Parsed input/output contract of a Depth Pro 2 ONNX graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphSpec {
    /// Input spatial dimensions expected by `pixel_values`.
    pub input_dimensions: Dimensions,
    /// Whether the graph expects an explicit `f_px` scalar input.
    pub has_f_px_input: bool,
    /// Streaming temporal state tensors carried between calls.
    pub state_specs: Vec<StateTensorSpec>,
    /// Output tensor name containing metric depth in metres.
    pub depth_output_name: String,
    /// Optional output tensor name containing estimated focal length in pixels.
    pub focal_output_name: Option<String>,
}

impl GraphSpec {
    /// Inspects raw input and output descriptions to derive a [`GraphSpec`].
    ///
    /// # Errors
    ///
    /// Returns [`DepthError::Inference`] if `pixel_values` is missing, has
    /// invalid spatial dimensions, or if a streaming state tensor has a
    /// dynamic or unsupported shape/type.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::Dimensions;
    /// use dp2_depth::{GraphSpec, StateDtype};
    ///
    /// let inputs = vec![
    ///     (
    ///         "pixel_values".to_string(),
    ///         vec![1, 3, 480, 640],
    ///         Some(StateDtype::F32),
    ///     ),
    ///     ("f_px".to_string(), vec![1], Some(StateDtype::F32)),
    /// ];
    /// let outputs = vec!["depth_meters".to_string()];
    /// let spec = GraphSpec::from_signatures(&inputs, &outputs)?;
    /// assert_eq!(spec.input_dimensions, Dimensions::new(640, 480)?);
    /// assert!(spec.has_f_px_input);
    /// assert!(spec.state_specs.is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn from_signatures(
        inputs: &[(String, Vec<i64>, Option<StateDtype>)],
        outputs: &[String],
    ) -> Result<Self, DepthError> {
        let (_, pixel_shape, _) = inputs
            .iter()
            .find(|(name, _, _)| name == "pixel_values")
            .ok_or_else(|| {
                DepthError::Inference("ONNX graph is missing required 'pixel_values' input".into())
            })?;

        if pixel_shape.len() != 4 {
            return Err(DepthError::Inference(format!(
                "expected 4D pixel_values input [T, 3, H, W], got shape {pixel_shape:?}"
            )));
        }

        let height = u32::try_from(pixel_shape[2]).map_err(|_| {
            DepthError::Inference(format!(
                "pixel_values height must be positive, got {}",
                pixel_shape[2]
            ))
        })?;
        let width = u32::try_from(pixel_shape[3]).map_err(|_| {
            DepthError::Inference(format!(
                "pixel_values width must be positive, got {}",
                pixel_shape[3]
            ))
        })?;
        let input_dimensions = Dimensions::new(width, height)
            .map_err(|e| DepthError::Inference(format!("invalid model input dimensions: {e}")))?;

        let has_f_px_input = inputs.iter().any(|(name, _, _)| name == "f_px");

        let mut state_specs = Vec::new();
        for (name, shape, dtype) in inputs {
            let Some(suffix) = name.strip_prefix("past_") else {
                continue;
            };
            let present_name = format!("present_{suffix}");
            if !outputs.iter().any(|o| o == &present_name) {
                return Err(DepthError::Inference(format!(
                    "streaming input '{name}' has no matching output '{present_name}'"
                )));
            }
            let concrete_shape = shape
                .iter()
                .map(|&d| {
                    usize::try_from(d).map_err(|_| {
                        DepthError::Inference(format!(
                            "streaming state input '{name}' has dynamic/negative dimension {d}"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let dtype = dtype.ok_or_else(|| {
                DepthError::Inference(format!(
                    "streaming state input '{name}' has unsupported element type"
                ))
            })?;
            state_specs.push(StateTensorSpec {
                past_name: name.clone(),
                present_name,
                shape: concrete_shape,
                dtype,
            });
        }

        let depth_output_name = outputs
            .iter()
            .find(|name| name.as_str() == "depth_meters" || name.as_str() == "depth")
            .or_else(|| outputs.first())
            .cloned()
            .ok_or_else(|| DepthError::Inference("ONNX graph declares no outputs".into()))?;

        let focal_output_name = outputs
            .iter()
            .find(|name| name.as_str() == "f_px" || name.as_str() == "focallength_px")
            .cloned();

        Ok(Self {
            input_dimensions,
            has_f_px_input,
            state_specs,
            depth_output_name,
            focal_output_name,
        })
    }

    /// Returns `true` if this graph maintains temporal state tensors across
    /// frames.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_depth::{GraphSpec, StateDtype};
    ///
    /// let inputs = vec![
    ///     (
    ///         "pixel_values".to_string(),
    ///         vec![1, 3, 480, 640],
    ///         Some(StateDtype::F32),
    ///     ),
    ///     ("f_px".to_string(), vec![1], Some(StateDtype::F32)),
    ///     ("past_k".to_string(), vec![1, 4, 8], Some(StateDtype::F16)),
    /// ];
    /// let outputs = vec!["depth_meters".to_string(), "present_k".to_string()];
    /// let spec = GraphSpec::from_signatures(&inputs, &outputs)?;
    /// assert!(spec.is_streaming());
    /// # Ok::<(), dp2_core::DepthError>(())
    /// ```
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        !self.state_specs.is_empty()
    }
}

struct FocalSession {
    session: Session,
    input_dimensions: Dimensions,
    output_name: String,
}

/// Depth Pro 2 estimator backed by ONNX Runtime (`ort`) dynamic loading.
pub struct OnnxDepthPro2Estimator {
    _env: Environment,
    session: Session,
    spec: GraphSpec,
    focal_policy: FocalLengthPolicy,
    focal_session: Option<FocalSession>,
    cached_focal_px: Option<f32>,
    state: BTreeMap<String, DynValue>,
}

impl OnnxDepthPro2Estimator {
    /// Loads the ONNX Runtime library and Depth Pro 2 model specified by
    /// `config`.
    ///
    /// # Errors
    ///
    /// Returns [`DepthError::Inference`] if the ONNX Runtime shared library or
    /// model file cannot be loaded, or if the model's input/output
    /// signature is incompatible.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_depth::{DepthPro2Config, OnnxDepthPro2Estimator};
    ///
    /// let config = DepthPro2Config::new("models/model_stream4.onnx");
    /// let _estimator = OnnxDepthPro2Estimator::from_config(&config)?;
    /// # Ok::<(), dp2_core::DepthError>(())
    /// ```
    pub fn from_config(config: &DepthPro2Config) -> Result<Self, DepthError> {
        let dylib_path = config
            .ort_dylib_path
            .as_deref()
            .unwrap_or_else(|| Path::new(default_ort_dylib_name()));

        let env = ort::init_from(dylib_path)
            .map_err(|e| {
                DepthError::Inference(format!(
                    "failed to load ONNX Runtime library '{}': {e}",
                    dylib_path.display()
                ))
            })?
            .with_name("dp2")
            .build()
            .map_err(|e| DepthError::Inference(format!("failed to build ORT environment: {e}")))?;

        let session = build_session(&env, &config.model_path, config.intra_threads)?;
        let spec = parse_session_spec(&session)?;

        let focal_session = match &config.focal_length {
            FocalLengthPolicy::EstimateWithModel(path) => {
                Some(load_focal_session(&env, path, config.intra_threads)?)
            }
            FocalLengthPolicy::FixedPx(_) | FocalLengthPolicy::HorizontalFovDegrees(_) => None,
        };

        let state = zero_state_map(&spec.state_specs)?;

        info!(
            model = %config.model_path.display(),
            model_width = spec.input_dimensions.width(),
            model_height = spec.input_dimensions.height(),
            streaming = spec.is_streaming(),
            state_tensors = spec.state_specs.len(),
            "loaded Depth Pro 2 ONNX model"
        );

        Ok(Self {
            _env: env,
            session,
            spec,
            focal_policy: config.focal_length.clone(),
            focal_session,
            cached_focal_px: None,
            state,
        })
    }

    /// Returns the parsed ONNX graph specification for the loaded model.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_depth::{DepthPro2Config, OnnxDepthPro2Estimator};
    ///
    /// let estimator = OnnxDepthPro2Estimator::from_config(&DepthPro2Config::new("model.onnx"))?;
    /// let _dims = estimator.graph_spec().input_dimensions;
    /// # Ok::<(), dp2_core::DepthError>(())
    /// ```
    #[must_use]
    pub const fn graph_spec(&self) -> &GraphSpec {
        &self.spec
    }

    fn resolve_focal_length_for_input(&mut self, frame: &VideoFrame) -> Result<f32, DepthError> {
        let input_width = frame.dimensions().width();
        match &self.focal_policy {
            FocalLengthPolicy::FixedPx(f_px) => {
                if !f_px.is_finite() || *f_px <= 0.0 {
                    return Err(DepthError::InvalidFocalLength {
                        focal_length_px: *f_px,
                    });
                }
                Ok(*f_px)
            }
            FocalLengthPolicy::HorizontalFovDegrees(fov) => {
                focal_length_from_fov(input_width, *fov)
            }
            FocalLengthPolicy::EstimateWithModel(_) => {
                if let Some(cached) = self.cached_focal_px {
                    return Ok(cached);
                }

                let focal = self.focal_session.as_mut().ok_or_else(|| {
                    DepthError::Inference("focal length session not initialised".into())
                })?;

                let tensor = frame_to_nchw_f32(frame, focal.input_dimensions);
                let pixel_val = Tensor::from_array(tensor).map_err(|e| {
                    DepthError::Inference(format!("failed to create focal input tensor: {e}"))
                })?;

                let outputs = focal
                    .session
                    .run(vec![(
                        "pixel_values".to_string(),
                        SessionInputValue::from(pixel_val),
                    )])
                    .map_err(|e| {
                        DepthError::Inference(format!("focal length inference failed: {e}"))
                    })?;

                let out_val = outputs.get(focal.output_name.as_str()).ok_or_else(|| {
                    DepthError::Inference(format!(
                        "focal model missing output '{}'",
                        focal.output_name
                    ))
                })?;
                let model_f_px = extract_scalar_f32(out_val)?;
                let input_f_px =
                    scale_focal_length(model_f_px, focal.input_dimensions.width(), input_width);
                self.cached_focal_px = Some(input_f_px);
                debug!(
                    focal_length_px = input_f_px,
                    "estimated camera focal length"
                );
                Ok(input_f_px)
            }
        }
    }
}

impl DepthEstimator for OnnxDepthPro2Estimator {
    fn estimate(&mut self, frame: &VideoFrame) -> Result<DepthMap, DepthError> {
        let _span = info_span!(
            "depth_pro_2_infer",
            input_width = frame.dimensions().width(),
            input_height = frame.dimensions().height(),
            model_width = self.spec.input_dimensions.width(),
            model_height = self.spec.input_dimensions.height()
        )
        .entered();

        let input_dims = frame.dimensions();
        let model_dims = self.spec.input_dimensions;
        let pixel_array = frame_to_nchw_f32(frame, model_dims);
        let pixel_tensor = Tensor::from_array(pixel_array)
            .map_err(|e| DepthError::Inference(format!("failed to build pixel_values: {e}")))?;

        let mut inputs: Vec<(String, SessionInputValue<'_>)> =
            Vec::with_capacity(2 + self.spec.state_specs.len());
        inputs.push((
            "pixel_values".to_string(),
            SessionInputValue::from(pixel_tensor),
        ));

        let input_f_px = if self.spec.has_f_px_input {
            let f_px_input = self.resolve_focal_length_for_input(frame)?;
            let f_px_model = scale_focal_length(f_px_input, input_dims.width(), model_dims.width());
            let f_tensor = Tensor::from_array(Array1::from_vec(vec![f_px_model]))
                .map_err(|e| DepthError::Inference(format!("failed to build f_px tensor: {e}")))?;
            inputs.push(("f_px".to_string(), SessionInputValue::from(f_tensor)));
            Some(f_px_input)
        } else {
            None
        };

        for spec in &self.spec.state_specs {
            let state_val = self.state.get(&spec.past_name).ok_or_else(|| {
                DepthError::Inference(format!("missing streaming state '{}'", spec.past_name))
            })?;
            inputs.push((spec.past_name.clone(), SessionInputValue::from(state_val)));
        }

        let mut outputs = self
            .session
            .run(inputs)
            .map_err(|e| DepthError::Inference(format!("ONNX Runtime execution failed: {e}")))?;

        let depth_val = outputs
            .get(self.spec.depth_output_name.as_str())
            .ok_or_else(|| {
                DepthError::Inference(format!(
                    "model output '{}' not returned",
                    self.spec.depth_output_name
                ))
            })?;
        let raw_depth = extract_depth_vec(depth_val)?;

        let resolved_focal_px = if let Some(f_px) = input_f_px {
            f_px
        } else if let Some(ref focal_name) = self.spec.focal_output_name {
            let focal_val = outputs.get(focal_name.as_str()).ok_or_else(|| {
                DepthError::Inference(format!("model output '{focal_name}' not returned"))
            })?;
            let model_f_px = extract_scalar_f32(focal_val)?;
            scale_focal_length(model_f_px, model_dims.width(), input_dims.width())
        } else {
            focal_length_from_fov(input_dims.width(), 60.0)?
        };

        let mut next_state = BTreeMap::new();
        for spec in &self.spec.state_specs {
            let updated = outputs.remove(&spec.present_name).ok_or_else(|| {
                DepthError::Inference(format!(
                    "streaming output '{}' missing from inference result",
                    spec.present_name
                ))
            })?;
            next_state.insert(spec.past_name.clone(), updated);
        }
        if !self.spec.state_specs.is_empty() {
            self.state = next_state;
        }

        if raw_depth.len() != model_dims.pixel_count() {
            return Err(DepthError::LengthMismatch {
                actual: raw_depth.len(),
                expected: model_dims.pixel_count(),
            });
        }

        let resized = resize_depth_bilinear(&raw_depth, model_dims, input_dims);
        DepthMap::new(input_dims, resized, resolved_focal_px)
    }

    fn reset(&mut self) {
        self.cached_focal_px = None;
        if let Ok(fresh) = zero_state_map(&self.spec.state_specs) {
            self.state = fresh;
        }
    }
}

fn build_session(
    env: &Environment,
    model_path: &Path,
    intra_threads: Option<usize>,
) -> Result<Session, DepthError> {
    let mut builder = Session::builder(env)
        .map_err(|e| DepthError::Inference(format!("failed to create session builder: {e}")))?
        .with_optimization_level(GraphOptimizationLevel::All)
        .map_err(|e| DepthError::Inference(format!("failed to set optimisation level: {e}")))?
        .with_execution_providers([ort::ep::CPU::default().with_arena_allocator(true).build()])
        .map_err(|e| DepthError::Inference(format!("failed to configure CPU provider: {e}")))?;

    if let Some(threads) = intra_threads {
        builder = builder
            .with_intra_threads(threads)
            .map_err(|e| DepthError::Inference(format!("failed to set intra threads: {e}")))?;
    }

    builder.commit_from_file(model_path).map_err(|e| {
        DepthError::Inference(format!(
            "failed to load ONNX model '{}': {e}",
            model_path.display()
        ))
    })
}

fn parse_session_spec(session: &Session) -> Result<GraphSpec, DepthError> {
    let inputs: Vec<(String, Vec<i64>, Option<StateDtype>)> = session
        .inputs()
        .iter()
        .map(|outlet| {
            let (shape, dtype) = match outlet.dtype() {
                ValueType::Tensor { ty, shape, .. } => {
                    (shape.iter().copied().collect(), map_element_type(*ty))
                }
                _ => (Vec::new(), None),
            };
            (outlet.name().to_string(), shape, dtype)
        })
        .collect();

    let outputs: Vec<String> = session
        .outputs()
        .iter()
        .map(|o| o.name().to_string())
        .collect();

    GraphSpec::from_signatures(&inputs, &outputs)
}

fn load_focal_session(
    env: &Environment,
    path: &Path,
    intra_threads: Option<usize>,
) -> Result<FocalSession, DepthError> {
    let session = build_session(env, path, intra_threads)?;
    let spec = parse_session_spec(&session)?;
    let output_name = spec.focal_output_name.unwrap_or(spec.depth_output_name);

    Ok(FocalSession {
        session,
        input_dimensions: spec.input_dimensions,
        output_name,
    })
}

const fn map_element_type(ty: TensorElementType) -> Option<StateDtype> {
    match ty {
        TensorElementType::Float32 => Some(StateDtype::F32),
        TensorElementType::Float16 => Some(StateDtype::F16),
        TensorElementType::Int64 => Some(StateDtype::I64),
        TensorElementType::Bool => Some(StateDtype::Bool),
        _ => None,
    }
}

fn zero_state_map(specs: &[StateTensorSpec]) -> Result<BTreeMap<String, DynValue>, DepthError> {
    let mut map = BTreeMap::new();
    for spec in specs {
        let value: DynValue = match spec.dtype {
            StateDtype::F32 => Tensor::from_array(ArrayD::<f32>::zeros(IxDyn(&spec.shape)))
                .map_err(|e| DepthError::Inference(format!("failed to allocate f32 state: {e}")))?
                .into_dyn(),
            StateDtype::F16 => {
                Tensor::from_array(ArrayD::<f16>::from_elem(IxDyn(&spec.shape), f16::ZERO))
                    .map_err(|e| {
                        DepthError::Inference(format!("failed to allocate f16 state: {e}"))
                    })?
                    .into_dyn()
            }
            StateDtype::I64 => Tensor::from_array(ArrayD::<i64>::zeros(IxDyn(&spec.shape)))
                .map_err(|e| DepthError::Inference(format!("failed to allocate i64 state: {e}")))?
                .into_dyn(),
            StateDtype::Bool => {
                Tensor::from_array(ArrayD::<bool>::from_elem(IxDyn(&spec.shape), false))
                    .map_err(|e| {
                        DepthError::Inference(format!("failed to allocate bool state: {e}"))
                    })?
                    .into_dyn()
            }
        };
        map.insert(spec.past_name.clone(), value);
    }
    Ok(map)
}

fn extract_depth_vec(value: &DynValue) -> Result<Vec<f32>, DepthError> {
    if let Ok(arr) = value.try_extract_array::<f32>() {
        return Ok(arr.iter().copied().collect());
    }
    if let Ok(arr) = value.try_extract_array::<f16>() {
        return Ok(arr.iter().map(|v| v.to_f32()).collect());
    }
    Err(DepthError::Inference(
        "depth output tensor is neither float32 nor float16".into(),
    ))
}

fn extract_scalar_f32(value: &DynValue) -> Result<f32, DepthError> {
    if let Ok(arr) = value.try_extract_array::<f32>()
        && let Some(&first) = arr.iter().next()
    {
        return Ok(first);
    }
    if let Ok(arr) = value.try_extract_array::<f16>()
        && let Some(&first) = arr.iter().next()
    {
        return Ok(first.to_f32());
    }
    Err(DepthError::Inference(
        "focal output tensor did not contain a float32/float16 scalar".into(),
    ))
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn graph_spec_parses_streaming_fp16_signature() -> Result<(), Box<dyn std::error::Error>> {
        let inputs = vec![
            (
                "pixel_values".to_string(),
                vec![1, 3, 480, 640],
                Some(StateDtype::F32),
            ),
            ("f_px".to_string(), vec![1], Some(StateDtype::F32)),
            ("past_filled".to_string(), vec![4], Some(StateDtype::Bool)),
            (
                "past_k_0".to_string(),
                vec![4, 6, 1200, 64],
                Some(StateDtype::F16),
            ),
        ];
        let outputs = vec![
            "depth_meters".to_string(),
            "present_filled".to_string(),
            "present_k_0".to_string(),
        ];

        let spec = GraphSpec::from_signatures(&inputs, &outputs)?;
        assert_eq!(
            spec,
            GraphSpec {
                input_dimensions: Dimensions::new(640, 480)?,
                has_f_px_input: true,
                state_specs: vec![
                    StateTensorSpec {
                        past_name: "past_filled".into(),
                        present_name: "present_filled".into(),
                        shape: vec![4],
                        dtype: StateDtype::Bool,
                    },
                    StateTensorSpec {
                        past_name: "past_k_0".into(),
                        present_name: "present_k_0".into(),
                        shape: vec![4, 6, 1200, 64],
                        dtype: StateDtype::F16,
                    },
                ],
                depth_output_name: "depth_meters".into(),
                focal_output_name: None,
            }
        );
        assert!(spec.is_streaming());
        Ok(())
    }

    #[test]
    fn graph_spec_parses_auto_focal_signature() -> Result<(), Box<dyn std::error::Error>> {
        let inputs = vec![(
            "pixel_values".to_string(),
            vec![1, 3, 384, 672],
            Some(StateDtype::F32),
        )];
        let outputs = vec!["depth_meters".to_string(), "f_px".to_string()];

        let spec = GraphSpec::from_signatures(&inputs, &outputs)?;
        assert_eq!(
            spec,
            GraphSpec {
                input_dimensions: Dimensions::new(672, 384)?,
                has_f_px_input: false,
                state_specs: vec![],
                depth_output_name: "depth_meters".into(),
                focal_output_name: Some("f_px".into()),
            }
        );
        assert!(!spec.is_streaming());
        Ok(())
    }

    #[test]
    fn graph_spec_rejects_missing_present_output() {
        let inputs = vec![
            (
                "pixel_values".to_string(),
                vec![1, 3, 480, 640],
                Some(StateDtype::F32),
            ),
            ("past_k_0".to_string(), vec![4], Some(StateDtype::F32)),
        ];
        let outputs = vec!["depth_meters".to_string()];

        assert_matches!(
            GraphSpec::from_signatures(&inputs, &outputs),
            Err(DepthError::Inference(_))
        );
    }
}
