// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;

use derive_more::{Display, From, Into};
use serde::{Deserialize, Serialize};

use crate::{DepthMap, DepthRange, EffectError, VideoFrame};

/// Stable string identifier for a frame effect (for example `"depth-map"`).
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Display, From, Into, Serialize, Deserialize,
)]
pub struct EffectId(String);

impl AsRef<str> for EffectId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl EffectId {
    /// Creates an effect identifier from any string-like value.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::EffectId;
    ///
    /// let id = EffectId::new("depth-map");
    /// assert_eq!(id.as_str(), "depth-map");
    /// ```
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the identifier as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::EffectId;
    ///
    /// let id = EffectId::new("passthrough");
    /// assert_eq!(id.as_str(), "passthrough");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Colour palette used by [`DepthMapEffect`] to map normalised inverse depth
/// into RGB.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Colormap {
    /// Perceptually vivid rainbow-like palette matching Depth Pro 2's
    /// visualiser (blue far, red near).
    #[default]
    Turbo,
    /// Perceptually uniform purple-to-yellow palette.
    Viridis,
    /// Perceptually uniform black-purple-orange-white palette.
    Magma,
    /// Linear 8-bit greyscale (black far, white near).
    Grayscale,
}

impl Colormap {
    /// Samples the colour palette at a normalised position `t` in `[0.0, 1.0]`.
    ///
    /// Values outside `[0.0, 1.0]` are clamped.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::Colormap;
    ///
    /// assert_eq!(Colormap::Grayscale.sample(0.0), [0, 0, 0]);
    /// assert_eq!(Colormap::Grayscale.sample(1.0), [255, 255, 255]);
    /// ```
    #[must_use]
    pub fn sample(self, t: f32) -> [u8; 3] {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };

        match self {
            Self::Grayscale => {
                let v = float_to_u8(t);
                [v, v, v]
            }
            Self::Turbo => sample_lut(&TURBO_STOPS, t),
            Self::Viridis => sample_lut(&VIRIDIS_STOPS, t),
            Self::Magma => sample_lut(&MAGMA_STOPS, t),
        }
    }
}

/// Configuration for [`DepthMapEffect`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepthMapConfig {
    /// Colour palette applied to normalised inverse depth.
    pub colormap: Colormap,
    /// Optional fixed near bound in metres.
    pub min_depth_meters: Option<f32>,
    /// Optional fixed far bound in metres.
    pub max_depth_meters: Option<f32>,
    /// When `true`, near and far colours are reversed.
    pub invert: bool,
}

impl Default for DepthMapConfig {
    fn default() -> Self {
        Self {
            colormap: Colormap::Turbo,
            min_depth_meters: None,
            max_depth_meters: None,
            invert: false,
        }
    }
}

impl DepthMapConfig {
    /// Validates optional depth range bounds and returns the resolved
    /// [`DepthRange`].
    ///
    /// # Errors
    ///
    /// Returns [`EffectError::InvalidParameter`] if only one of
    /// `min_depth_meters` or `max_depth_meters` is set, or
    /// [`EffectError::Depth`] if the pair is not a valid [`DepthRange`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Colormap, DepthMapConfig, DepthRange};
    ///
    /// let cfg = DepthMapConfig {
    ///     colormap: Colormap::Viridis,
    ///     min_depth_meters: Some(0.5),
    ///     max_depth_meters: Some(8.0),
    ///     invert: false,
    /// };
    /// assert_eq!(cfg.depth_range()?, Some(DepthRange::new(0.5, 8.0)?));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn depth_range(&self) -> Result<Option<DepthRange>, EffectError> {
        match (self.min_depth_meters, self.max_depth_meters) {
            (None, None) => Ok(None),
            (Some(min_m), Some(max_m)) => Ok(Some(DepthRange::new(min_m, max_m)?)),
            _ => Err(EffectError::InvalidParameter(
                "min_depth_meters and max_depth_meters must both be set or both omitted".into(),
            )),
        }
    }
}

/// Runtime-selectable effect configuration.
///
/// Built-in effects have strongly typed variants; future or external effects
/// can also be addressed via [`EffectConfig::Custom`] without changing the
/// control protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum EffectConfig {
    /// Visualise the Depth Pro 2 depth map as a colour or greyscale video feed.
    DepthMap(DepthMapConfig),
    /// Forward input frames without depth estimation or modification.
    Passthrough,
    /// Invoke a custom registered effect by identifier and key-value
    /// parameters.
    Custom {
        /// Identifier registered with [`EffectRegistry`].
        id: EffectId,
        /// Effect-specific string parameters.
        parameters: BTreeMap<String, String>,
    },
}

impl EffectConfig {
    /// Returns the effect identifier corresponding to this configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthMapConfig, EffectConfig, EffectId};
    ///
    /// assert_eq!(
    ///     EffectConfig::DepthMap(DepthMapConfig::default()).id(),
    ///     EffectId::new("depth-map")
    /// );
    /// assert_eq!(EffectConfig::Passthrough.id(), EffectId::new("passthrough"));
    /// ```
    #[must_use]
    pub fn id(&self) -> EffectId {
        match self {
            Self::DepthMap(_) => EffectId::new(DepthMapEffect::ID),
            Self::Passthrough => EffectId::new(PassthroughEffect::ID),
            Self::Custom { id, .. } => id.clone(),
        }
    }
}

impl Default for EffectConfig {
    fn default() -> Self {
        Self::DepthMap(DepthMapConfig::default())
    }
}

/// Static metadata describing a registered frame effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectDescriptor {
    /// Stable machine-readable identifier.
    pub id: EffectId,
    /// Human-readable name.
    pub name: String,
    /// Concise summary of what the effect outputs.
    pub description: String,
    /// Whether the pipeline must run depth estimation before applying this
    /// effect.
    pub requires_depth: bool,
}

/// A frame transformation that can be swapped at runtime inside the pipeline.
pub trait FrameEffect: Send + Sync {
    /// Returns metadata describing this effect.
    fn descriptor(&self) -> EffectDescriptor;

    /// Returns whether this effect requires a [`DepthMap`] for each input
    /// frame.
    fn requires_depth(&self) -> bool {
        self.descriptor().requires_depth
    }

    /// Produces an output video frame from the input frame and optional depth
    /// map.
    ///
    /// # Errors
    ///
    /// Returns an [`EffectError`] if required depth data is absent or
    /// dimensions do not match.
    fn apply(
        &self,
        input: &VideoFrame,
        depth: Option<&DepthMap>,
    ) -> Result<VideoFrame, EffectError>;
}

/// Constructs configured [`FrameEffect`] instances from an [`EffectConfig`].
pub trait EffectFactory: Send + Sync {
    /// Returns the descriptor of the effect that this factory builds.
    fn descriptor(&self) -> EffectDescriptor;

    /// Validates `config` and constructs an executable effect instance.
    ///
    /// # Errors
    ///
    /// Returns an [`EffectError`] if `config` does not match this effect or
    /// contains invalid parameters.
    fn build(&self, config: &EffectConfig) -> Result<Box<dyn FrameEffect>, EffectError>;
}

/// Registry of available frame effects that can be instantiated and swapped at
/// runtime.
pub struct EffectRegistry {
    factories: BTreeMap<EffectId, Box<dyn EffectFactory>>,
}

impl Default for EffectRegistry {
    fn default() -> Self {
        Self::with_builtin_effects()
    }
}

impl EffectRegistry {
    /// Creates an empty effect registry.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::EffectRegistry;
    ///
    /// let registry = EffectRegistry::empty();
    /// assert!(registry.descriptors().is_empty());
    /// ```
    #[must_use]
    pub fn empty() -> Self {
        Self {
            factories: BTreeMap::new(),
        }
    }

    /// Creates a registry pre-populated with the built-in `depth-map` and
    /// `passthrough` effects.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectId, EffectRegistry};
    ///
    /// let registry = EffectRegistry::with_builtin_effects();
    /// let ids: Vec<EffectId> = registry.descriptors().into_iter().map(|d| d.id).collect();
    /// assert_eq!(
    ///     ids,
    ///     vec![EffectId::new("depth-map"), EffectId::new("passthrough")]
    /// );
    /// ```
    #[must_use]
    pub fn with_builtin_effects() -> Self {
        let mut registry = Self::empty();
        registry.register(Box::new(DepthMapEffectFactory));
        registry.register(Box::new(PassthroughEffectFactory));
        registry
    }

    /// Registers a new effect factory, replacing any existing factory with the
    /// same identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthMapConfig, EffectConfig, EffectRegistry};
    ///
    /// let registry = EffectRegistry::with_builtin_effects();
    /// let effect = registry.build(&EffectConfig::DepthMap(DepthMapConfig::default()))?;
    /// assert!(effect.requires_depth());
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    pub fn register(&mut self, factory: Box<dyn EffectFactory>) {
        let id = factory.descriptor().id;
        self.factories.insert(id, factory);
    }

    /// Returns descriptors for all registered effects in deterministic
    /// identifier order.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::EffectRegistry;
    ///
    /// let registry = EffectRegistry::with_builtin_effects();
    /// assert_eq!(registry.descriptors().len(), 2);
    /// ```
    #[must_use]
    pub fn descriptors(&self) -> Vec<EffectDescriptor> {
        self.factories.values().map(|f| f.descriptor()).collect()
    }

    /// Instantiates an effect from `config`.
    ///
    /// # Errors
    ///
    /// Returns [`EffectError::UnknownEffect`] if no factory is registered for
    /// the configuration's identifier, or propagates validation errors from
    /// the factory.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{EffectConfig, EffectRegistry};
    ///
    /// let registry = EffectRegistry::with_builtin_effects();
    /// let effect = registry.build(&EffectConfig::Passthrough)?;
    /// assert!(!effect.requires_depth());
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    pub fn build(&self, config: &EffectConfig) -> Result<Box<dyn FrameEffect>, EffectError> {
        let id = config.id();
        let factory = self
            .factories
            .get(&id)
            .ok_or_else(|| EffectError::UnknownEffect(id.to_string()))?;
        factory.build(config)
    }
}

/// Visualises a [`DepthMap`] as a colour-mapped video frame.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthMapEffect {
    colormap: Colormap,
    range: Option<DepthRange>,
    invert: bool,
}

impl DepthMapEffect {
    /// Registered identifier for the depth-map visualiser effect.
    pub const ID: &'static str = "depth-map";

    /// Constructs a validated depth-map visualiser effect.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured depth bounds are invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{DepthMapConfig, DepthMapEffect};
    ///
    /// let effect = DepthMapEffect::new(&DepthMapConfig::default())?;
    /// assert_eq!(effect.colormap(), dp2_core::Colormap::Turbo);
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    pub fn new(config: &DepthMapConfig) -> Result<Self, EffectError> {
        let range = config.depth_range()?;
        Ok(Self {
            colormap: config.colormap,
            range,
            invert: config.invert,
        })
    }

    /// Returns the configured colour palette.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Colormap, DepthMapConfig, DepthMapEffect};
    ///
    /// let effect = DepthMapEffect::new(&DepthMapConfig {
    ///     colormap: Colormap::Grayscale,
    ///     ..DepthMapConfig::default()
    /// })?;
    /// assert_eq!(effect.colormap(), Colormap::Grayscale);
    /// # Ok::<(), dp2_core::EffectError>(())
    /// ```
    #[must_use]
    pub const fn colormap(&self) -> Colormap {
        self.colormap
    }

    /// Returns the static descriptor for [`DepthMapEffect`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::DepthMapEffect;
    ///
    /// let desc = DepthMapEffect::static_descriptor();
    /// assert!(desc.requires_depth);
    /// ```
    #[must_use]
    pub fn static_descriptor() -> EffectDescriptor {
        EffectDescriptor {
            id: EffectId::new(Self::ID),
            name: "Depth Map".into(),
            description: "Renders the Depth Pro 2 metric depth map with a configurable colormap"
                .into(),
            requires_depth: true,
        }
    }
}

impl FrameEffect for DepthMapEffect {
    fn descriptor(&self) -> EffectDescriptor {
        Self::static_descriptor()
    }

    fn apply(
        &self,
        input: &VideoFrame,
        depth: Option<&DepthMap>,
    ) -> Result<VideoFrame, EffectError> {
        let depth = depth.ok_or(EffectError::MissingDepthMap)?;
        if depth.dimensions() != input.dimensions() {
            return Err(EffectError::DimensionMismatch {
                frame_width: input.dimensions().width(),
                frame_height: input.dimensions().height(),
                depth_width: depth.dimensions().width(),
                depth_height: depth.dimensions().height(),
            });
        }

        let norm = depth.normalised_inverse_depth(self.range);
        let mut rgb = Vec::with_capacity(norm.len() * 3);
        for value in norm {
            let sample_pos = if self.invert { 1.0 - value } else { value };
            rgb.extend_from_slice(&self.colormap.sample(sample_pos));
        }

        Ok(VideoFrame::from_rgb8(
            input.dimensions(),
            input.format(),
            input.timestamp_ns(),
            input.sequence(),
            &rgb,
        )?)
    }
}

struct DepthMapEffectFactory;

impl EffectFactory for DepthMapEffectFactory {
    fn descriptor(&self) -> EffectDescriptor {
        DepthMapEffect::static_descriptor()
    }

    fn build(&self, config: &EffectConfig) -> Result<Box<dyn FrameEffect>, EffectError> {
        let EffectConfig::DepthMap(cfg) = config else {
            return Err(EffectError::InvalidParameter(format!(
                "expected depth-map configuration, got {}",
                config.id()
            )));
        };

        Ok(Box::new(DepthMapEffect::new(cfg)?))
    }
}

/// Forwards input video frames unmodified and skips depth estimation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassthroughEffect;

impl PassthroughEffect {
    /// Registered identifier for the passthrough effect.
    pub const ID: &'static str = "passthrough";

    /// Returns the static descriptor for [`PassthroughEffect`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::PassthroughEffect;
    ///
    /// let desc = PassthroughEffect::static_descriptor();
    /// assert!(!desc.requires_depth);
    /// ```
    #[must_use]
    pub fn static_descriptor() -> EffectDescriptor {
        EffectDescriptor {
            id: EffectId::new(Self::ID),
            name: "Passthrough".into(),
            description: "Forwards the input video stream without running depth estimation".into(),
            requires_depth: false,
        }
    }
}

impl FrameEffect for PassthroughEffect {
    fn descriptor(&self) -> EffectDescriptor {
        Self::static_descriptor()
    }

    fn apply(
        &self,
        input: &VideoFrame,
        _depth: Option<&DepthMap>,
    ) -> Result<VideoFrame, EffectError> {
        Ok(input.clone())
    }
}

struct PassthroughEffectFactory;

impl EffectFactory for PassthroughEffectFactory {
    fn descriptor(&self) -> EffectDescriptor {
        PassthroughEffect::static_descriptor()
    }

    fn build(&self, config: &EffectConfig) -> Result<Box<dyn FrameEffect>, EffectError> {
        if !matches!(config, EffectConfig::Passthrough) {
            return Err(EffectError::InvalidParameter(format!(
                "expected passthrough configuration, got {}",
                config.id()
            )));
        }

        Ok(Box::new(PassthroughEffect))
    }
}

fn sample_lut(stops: &[[f32; 3]], t: f32) -> [u8; 3] {
    let last_index = stops.len() - 1;
    #[expect(
        clippy::cast_precision_loss,
        reason = "colour LUT has fewer than 16 stops, well within exact f32 range"
    )]
    let scaled = t * (last_index as f32);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "scaled is clamped to [0, last_index] before truncation"
    )]
    let lower = (scaled.floor() as usize).min(last_index);
    let upper = (lower + 1).min(last_index);
    #[expect(
        clippy::cast_precision_loss,
        reason = "lower is a small LUT index within exact f32 range"
    )]
    let frac = (scaled - (lower as f32)).clamp(0.0, 1.0);

    let a = stops[lower];
    let b = stops[upper];
    [
        float_to_u8(a[0] + (b[0] - a[0]) * frac),
        float_to_u8(a[1] + (b[1] - a[1]) * frac),
        float_to_u8(a[2] + (b[2] - a[2]) * frac),
    ]
}

fn float_to_u8(v: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is clamped to [0.0, 255.0] before conversion to u8"
    )]
    let byte = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    byte
}

const TURBO_STOPS: [[f32; 3]; 9] = [
    [0.189_95, 0.071_76, 0.232_17],
    [0.251_07, 0.252_37, 0.633_74],
    [0.193_76, 0.483_70, 0.931_26],
    [0.107_62, 0.729_78, 0.858_52],
    [0.377_58, 0.903_75, 0.562_73],
    [0.742_12, 0.962_65, 0.249_52],
    [0.983_60, 0.746_08, 0.197_64],
    [0.922_74, 0.371_32, 0.087_01],
    [0.479_60, 0.015_83, 0.010_55],
];

const VIRIDIS_STOPS: [[f32; 3]; 5] = [
    [0.267_00, 0.004_87, 0.329_42],
    [0.229_74, 0.322_36, 0.545_71],
    [0.127_57, 0.566_95, 0.550_56],
    [0.369_21, 0.788_89, 0.382_91],
    [0.993_25, 0.906_16, 0.143_94],
];

const MAGMA_STOPS: [[f32; 3]; 5] = [
    [0.001_46, 0.000_47, 0.013_87],
    [0.316_65, 0.071_69, 0.485_38],
    [0.716_39, 0.214_98, 0.475_29],
    [0.987_62, 0.535_58, 0.382_21],
    [0.987_05, 0.991_44, 0.749_50],
];

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{Dimensions, PixelFormat};

    #[test]
    fn depth_map_effect_renders_grayscale_inverse_depth() -> Result<(), Box<dyn std::error::Error>>
    {
        let dims = Dimensions::new(3, 1)?;
        let input = VideoFrame::packed(dims, PixelFormat::Rgba8, 1234, 9, vec![0; 12])?;
        let depth = DepthMap::new(dims, vec![1.0, 2.0, f32::INFINITY], 500.0)?;
        let effect = DepthMapEffect::new(&DepthMapConfig {
            colormap: Colormap::Grayscale,
            min_depth_meters: Some(1.0),
            max_depth_meters: Some(4.0),
            invert: false,
        })?;

        let output = effect.apply(&input, Some(&depth))?;
        let expected = VideoFrame::packed(
            dims,
            PixelFormat::Rgba8,
            1234,
            9,
            vec![255, 255, 255, 255, 85, 85, 85, 255, 0, 0, 0, 255],
        )?;
        assert_eq!(output, expected);
        Ok(())
    }

    #[test]
    fn depth_map_effect_requires_depth_and_matching_dimensions()
    -> Result<(), Box<dyn std::error::Error>> {
        let dims = Dimensions::new(2, 2)?;
        let input = VideoFrame::packed(dims, PixelFormat::Rgb8, 0, 0, vec![0; 12])?;
        let effect = DepthMapEffect::new(&DepthMapConfig::default())?;

        assert_matches!(
            effect.apply(&input, None),
            Err(EffectError::MissingDepthMap)
        );

        let wrong_depth = DepthMap::new(Dimensions::new(1, 1)?, vec![1.0], 500.0)?;
        assert_matches!(
            effect.apply(&input, Some(&wrong_depth)),
            Err(EffectError::DimensionMismatch {
                frame_width: 2,
                frame_height: 2,
                depth_width: 1,
                depth_height: 1
            })
        );
        Ok(())
    }

    #[test]
    fn depth_map_config_rejects_half_range() {
        let cfg = DepthMapConfig {
            min_depth_meters: Some(0.5),
            max_depth_meters: None,
            ..DepthMapConfig::default()
        };
        assert_matches!(cfg.depth_range(), Err(EffectError::InvalidParameter(_)));
    }

    #[test]
    fn registry_rejects_unknown_effect() {
        let registry = EffectRegistry::with_builtin_effects();
        let cfg = EffectConfig::Custom {
            id: EffectId::new("depth-of-field"),
            parameters: BTreeMap::new(),
        };
        assert_matches!(
            registry.build(&cfg).err(),
            Some(EffectError::UnknownEffect(id)) if id == "depth-of-field"
        );
    }

    #[test]
    fn custom_effect_can_be_registered_and_invoked() -> Result<(), Box<dyn std::error::Error>> {
        struct InvertEffect;
        impl FrameEffect for InvertEffect {
            fn descriptor(&self) -> EffectDescriptor {
                EffectDescriptor {
                    id: EffectId::new("invert"),
                    name: "Invert".into(),
                    description: "Inverts RGB channels".into(),
                    requires_depth: false,
                }
            }

            fn apply(
                &self,
                input: &VideoFrame,
                _depth: Option<&DepthMap>,
            ) -> Result<VideoFrame, EffectError> {
                let rgb: Vec<u8> = input.to_rgb8().into_iter().map(|v| 255 - v).collect();
                Ok(VideoFrame::from_rgb8(
                    input.dimensions(),
                    input.format(),
                    input.timestamp_ns(),
                    input.sequence(),
                    &rgb,
                )?)
            }
        }

        struct InvertFactory;
        impl EffectFactory for InvertFactory {
            fn descriptor(&self) -> EffectDescriptor {
                InvertEffect.descriptor()
            }

            fn build(&self, _config: &EffectConfig) -> Result<Box<dyn FrameEffect>, EffectError> {
                Ok(Box::new(InvertEffect))
            }
        }

        let mut registry = EffectRegistry::with_builtin_effects();
        registry.register(Box::new(InvertFactory));

        let effect = registry.build(&EffectConfig::Custom {
            id: EffectId::new("invert"),
            parameters: BTreeMap::new(),
        })?;
        let frame = VideoFrame::packed(
            Dimensions::new(1, 1)?,
            PixelFormat::Rgb8,
            0,
            0,
            vec![10, 20, 30],
        )?;
        let out = effect.apply(&frame, None)?;
        assert_eq!(out.data(), &[245, 235, 225]);
        Ok(())
    }
}
