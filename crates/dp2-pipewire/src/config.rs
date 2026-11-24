// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use dp2_core::{Dimensions, PixelFormat};

/// Configuration for capturing video frames from an existing PipeWire node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeWireSourceConfig {
    /// Target PipeWire node name, serial, or object ID. When `None`, PipeWire
    /// selects the default camera source.
    pub target: Option<String>,
    /// Client stream name registered with PipeWire.
    pub stream_name: String,
    /// Optional PipeWire remote socket name (for example an isolated test
    /// daemon socket).
    pub remote_name: Option<String>,
    /// Maximum buffered frames kept between the PipeWire thread and the
    /// pipeline thread.
    pub queue_capacity: usize,
}

impl PipeWireSourceConfig {
    /// Creates a capture configuration targeting `target` with sensible
    /// defaults.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_pipewire::PipeWireSourceConfig;
    ///
    /// let cfg = PipeWireSourceConfig::new(Some("my-camera".into()));
    /// assert_eq!(cfg.target.as_deref(), Some("my-camera"));
    /// assert_eq!(cfg.queue_capacity, 2);
    /// ```
    #[must_use]
    pub fn new(target: Option<String>) -> Self {
        Self {
            target,
            stream_name: "dp2-capture".into(),
            remote_name: None,
            queue_capacity: 2,
        }
    }
}

/// Configuration for publishing processed frames as a PipeWire virtual camera
/// device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeWireSinkConfig {
    /// PipeWire `node.name` property for the virtual camera.
    pub node_name: String,
    /// Human-readable `node.description` shown in video applications.
    pub node_description: String,
    /// Optional PipeWire remote socket name.
    pub remote_name: Option<String>,
    /// Video dimensions advertised by the virtual camera stream.
    pub dimensions: Dimensions,
    /// Pixel format advertised by the virtual camera stream.
    pub format: PixelFormat,
    /// Nominal framerate numerator.
    pub framerate_num: u32,
    /// Nominal framerate denominator.
    pub framerate_denom: u32,
}

impl PipeWireSinkConfig {
    /// Creates a virtual camera configuration with the given node name,
    /// description, and output dimensions.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::{Dimensions, PixelFormat};
    /// use dp2_pipewire::PipeWireSinkConfig;
    ///
    /// let cfg = PipeWireSinkConfig::new(
    ///     "dp2-depth",
    ///     "Depth Pro 2 Camera",
    ///     Dimensions::new(640, 480)?,
    /// );
    /// assert_eq!(cfg.node_name, "dp2-depth");
    /// assert_eq!(cfg.format, PixelFormat::Rgba8);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub fn new(
        node_name: impl Into<String>,
        node_description: impl Into<String>,
        dimensions: Dimensions,
    ) -> Self {
        Self {
            node_name: node_name.into(),
            node_description: node_description.into(),
            remote_name: None,
            dimensions,
            format: PixelFormat::Rgba8,
            framerate_num: 30,
            framerate_denom: 1,
        }
    }
}
