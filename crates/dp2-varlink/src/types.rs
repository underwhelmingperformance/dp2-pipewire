// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;

use dp2_core::{
    Colormap, DepthMapConfig, DepthMapEffect, Dimensions, EffectConfig, EffectDescriptor,
    EffectError, EffectId, PassthroughEffect, PipelineStatus,
};
use serde::{Deserialize, Serialize};
use zlink::introspect;

/// Varlink wire representation of video frame dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, introspect::CustomType)]
pub struct DimensionsInfo {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
}

impl From<Dimensions> for DimensionsInfo {
    fn from(value: Dimensions) -> Self {
        Self {
            width: value.width(),
            height: value.height(),
        }
    }
}

/// Varlink wire representation of an [`EffectDescriptor`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, introspect::CustomType)]
pub struct EffectInfo {
    /// Stable effect identifier.
    pub id: String,
    /// Human-readable effect name.
    pub name: String,
    /// Short summary of the effect's behaviour.
    pub description: String,
    /// Whether the effect requires Depth Pro 2 inference.
    pub requires_depth: bool,
}

impl From<EffectDescriptor> for EffectInfo {
    fn from(value: EffectDescriptor) -> Self {
        Self {
            id: value.id.into(),
            name: value.name,
            description: value.description,
            requires_depth: value.requires_depth,
        }
    }
}

/// Varlink wire representation of an [`EffectConfig`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, introspect::CustomType)]
pub struct EffectSettings {
    /// Registered effect identifier (`"depth-map"`, `"passthrough"`, or a
    /// custom identifier).
    pub id: String,
    /// Optional colour palette name (`"turbo"`, `"viridis"`, `"magma"`, or
    /// `"grayscale"`).
    pub colormap: Option<String>,
    /// Optional fixed near bound in metres.
    pub min_depth_meters: Option<f64>,
    /// Optional fixed far bound in metres.
    pub max_depth_meters: Option<f64>,
    /// Optional flag reversing near and far colours.
    pub invert: Option<bool>,
    /// Optional key-value parameters for custom effects.
    pub parameters: Option<BTreeMap<String, String>>,
}

impl From<&EffectConfig> for EffectSettings {
    fn from(value: &EffectConfig) -> Self {
        match value {
            EffectConfig::DepthMap(cfg) => Self {
                id: DepthMapEffect::ID.to_string(),
                colormap: Some(colormap_to_str(cfg.colormap).to_string()),
                min_depth_meters: cfg.min_depth_meters.map(f64::from),
                max_depth_meters: cfg.max_depth_meters.map(f64::from),
                invert: Some(cfg.invert),
                parameters: None,
            },
            EffectConfig::Passthrough => Self {
                id: PassthroughEffect::ID.to_string(),
                colormap: None,
                min_depth_meters: None,
                max_depth_meters: None,
                invert: None,
                parameters: None,
            },
            EffectConfig::Custom { id, parameters } => Self {
                id: id.to_string(),
                colormap: None,
                min_depth_meters: None,
                max_depth_meters: None,
                invert: None,
                parameters: Some(parameters.clone()),
            },
        }
    }
}

impl TryFrom<EffectSettings> for EffectConfig {
    type Error = EffectError;

    fn try_from(value: EffectSettings) -> Result<Self, Self::Error> {
        match value.id.as_str() {
            DepthMapEffect::ID => {
                let colormap = match value.colormap.as_deref() {
                    None => Colormap::default(),
                    Some(name) => parse_colormap(name)?,
                };
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "Varlink floats are f64 while DepthMapConfig bounds are f32 metres"
                )]
                let min_depth_meters = value.min_depth_meters.map(|v| v as f32);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "Varlink floats are f64 while DepthMapConfig bounds are f32 metres"
                )]
                let max_depth_meters = value.max_depth_meters.map(|v| v as f32);
                Ok(Self::DepthMap(DepthMapConfig {
                    colormap,
                    min_depth_meters,
                    max_depth_meters,
                    invert: value.invert.unwrap_or(false),
                }))
            }
            PassthroughEffect::ID => Ok(Self::Passthrough),
            other => Ok(Self::Custom {
                id: EffectId::new(other),
                parameters: value.parameters.unwrap_or_default(),
            }),
        }
    }
}

/// Varlink wire representation of a [`PipelineStatus`] snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, introspect::CustomType)]
pub struct StatusSnapshot {
    /// Whether the processing loop is running.
    pub running: bool,
    /// Input PipeWire node name.
    pub source_name: String,
    /// Input PipeWire node description.
    pub source_description: String,
    /// Output virtual camera node name.
    pub sink_name: String,
    /// Output virtual camera node description.
    pub sink_description: String,
    /// Active effect settings.
    pub active_effect: EffectSettings,
    /// Total processed frames.
    pub frames_processed: u64,
    /// Most recent depth inference duration in milliseconds.
    pub last_inference_ms: Option<f64>,
    /// Most recent effect duration in milliseconds.
    pub last_effect_ms: Option<f64>,
    /// Most recent focal length in pixels.
    pub last_focal_length_px: Option<f64>,
    /// Most recent frame dimensions.
    pub frame_dimensions: Option<DimensionsInfo>,
}

impl From<&PipelineStatus> for StatusSnapshot {
    fn from(value: &PipelineStatus) -> Self {
        Self {
            running: value.running,
            source_name: value.source.name.clone(),
            source_description: value.source.description.clone(),
            sink_name: value.sink.name.clone(),
            sink_description: value.sink.description.clone(),
            active_effect: EffectSettings::from(&value.active_effect),
            frames_processed: value.frames_processed,
            last_inference_ms: value.last_inference_ms,
            last_effect_ms: value.last_effect_ms,
            last_focal_length_px: value.last_focal_length_px.map(f64::from),
            frame_dimensions: value.frame_dimensions.map(DimensionsInfo::from),
        }
    }
}

/// Reply payload for `GetStatus` and `WatchStatus`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, introspect::Type)]
pub struct StatusReply {
    /// Current pipeline status snapshot.
    pub status: StatusSnapshot,
}

/// Reply payload for `ListEffects`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, introspect::Type)]
pub struct EffectsReply {
    /// Registered effects available for runtime switching.
    pub effects: Vec<EffectInfo>,
}

/// Reply payload for `GetEffect` and `SetEffect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, introspect::Type)]
pub struct EffectReply {
    /// Active effect settings.
    pub effect: EffectSettings,
}

/// Reply payload for state-mutating control operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, introspect::Type)]
pub struct AckReply {
    /// Always `true` when the request was accepted.
    pub ok: bool,
}

const fn colormap_to_str(colormap: Colormap) -> &'static str {
    match colormap {
        Colormap::Turbo => "turbo",
        Colormap::Viridis => "viridis",
        Colormap::Magma => "magma",
        Colormap::Grayscale => "grayscale",
    }
}

fn parse_colormap(name: &str) -> Result<Colormap, EffectError> {
    match name {
        "turbo" => Ok(Colormap::Turbo),
        "viridis" => Ok(Colormap::Viridis),
        "magma" => Ok(Colormap::Magma),
        "grayscale" | "greyscale" => Ok(Colormap::Grayscale),
        other => Err(EffectError::InvalidParameter(format!(
            "unsupported colormap '{other}'; expected one of turbo, viridis, magma, grayscale"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::turbo(Colormap::Turbo, "turbo")]
    #[case::viridis(Colormap::Viridis, "viridis")]
    #[case::magma(Colormap::Magma, "magma")]
    #[case::grayscale(Colormap::Grayscale, "grayscale")]
    fn effect_settings_round_trips_depth_map(
        #[case] colormap: Colormap,
        #[case] expected_name: &str,
    ) -> Result<(), EffectError> {
        let cfg = EffectConfig::DepthMap(DepthMapConfig {
            colormap,
            min_depth_meters: Some(0.5),
            max_depth_meters: Some(8.0),
            invert: true,
        });
        let wire = EffectSettings::from(&cfg);
        assert_eq!(wire.colormap.as_deref(), Some(expected_name));
        let back = EffectConfig::try_from(wire)?;
        assert_eq!(back, cfg);
        Ok(())
    }

    #[test]
    fn effect_settings_rejects_unknown_colormap() {
        let wire = EffectSettings {
            id: "depth-map".into(),
            colormap: Some("neon".into()),
            min_depth_meters: None,
            max_depth_meters: None,
            invert: None,
            parameters: None,
        };
        assert_matches!(
            EffectConfig::try_from(wire),
            Err(EffectError::InvalidParameter(_))
        );
    }
}
