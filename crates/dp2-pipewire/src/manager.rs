// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use dp2_core::{Dimensions, SourceError, SourceFactory, VideoDeviceDescriptor, VideoSource};
use tracing::info;

use crate::{
    FfmpegVideoDecoder, LoopingVideoPublisher, PipeWireError, PipeWireSinkConfig,
    PipeWireSourceConfig, PipeWireVideoSink, PipeWireVideoSource, VideoFileDecoder,
    list_pipewire_video_sources, slugify_video_node_name,
};

struct ImportedEntry {
    descriptor: VideoDeviceDescriptor,
    _publisher: LoopingVideoPublisher,
}

/// Manages PipeWire camera discovery, runtime [`PipeWireVideoSource`] opening,
/// and imported looping video files published as PipeWire `Video/Source` nodes.
pub struct PipeWireSourceManager {
    remote_name: Option<String>,
    exclude_sink_name: String,
    import_dimensions: Dimensions,
    max_import_frames: usize,
    decoder: Arc<dyn VideoFileDecoder>,
    imported: Mutex<BTreeMap<String, ImportedEntry>>,
}

impl PipeWireSourceManager {
    /// Creates a source manager connected to `remote_name` (or the default
    /// PipeWire daemon) that hides `exclude_sink_name` from discovered sources.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::Dimensions;
    /// use dp2_pipewire::PipeWireSourceManager;
    ///
    /// let _manager = PipeWireSourceManager::new(None, "dp2-depth", Dimensions::new(640, 480)?);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub fn new(
        remote_name: Option<String>,
        exclude_sink_name: impl Into<String>,
        import_dimensions: Dimensions,
    ) -> Self {
        Self {
            remote_name,
            exclude_sink_name: exclude_sink_name.into(),
            import_dimensions,
            max_import_frames: 150,
            decoder: Arc::new(FfmpegVideoDecoder::default()),
            imported: Mutex::new(BTreeMap::new()),
        }
    }

    /// Replaces the [`VideoFileDecoder`] used when importing video files.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use dp2_core::Dimensions;
    /// use dp2_pipewire::{FfmpegVideoDecoder, PipeWireSourceManager};
    ///
    /// let _manager = PipeWireSourceManager::new(None, "dp2-depth", Dimensions::new(640, 480)?)
    ///     .with_decoder(Arc::new(FfmpegVideoDecoder::new("ffmpeg", 15)));
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub fn with_decoder(mut self, decoder: Arc<dyn VideoFileDecoder>) -> Self {
        self.decoder = decoder;
        self
    }

    /// Sets the maximum number of frames decoded from each imported video clip.
    ///
    /// # Examples
    ///
    /// ```
    /// use dp2_core::Dimensions;
    /// use dp2_pipewire::PipeWireSourceManager;
    ///
    /// let _manager = PipeWireSourceManager::new(None, "dp2-depth", Dimensions::new(640, 480)?)
    ///     .with_max_import_frames(60);
    /// # Ok::<(), dp2_core::FrameError>(())
    /// ```
    #[must_use]
    pub fn with_max_import_frames(mut self, max_frames: usize) -> Self {
        self.max_import_frames = max_frames.max(1);
        self
    }

    fn unique_node_name(&self, base: &str) -> String {
        let guard = self
            .imported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !guard.contains_key(base) {
            return base.to_string();
        }
        for idx in 2..1000 {
            let candidate = format!("{base}-{idx}");
            if !guard.contains_key(&candidate) {
                return candidate;
            }
        }
        format!("{base}-overflow")
    }
}

impl SourceFactory for PipeWireSourceManager {
    fn list_sources(&self) -> Result<Vec<VideoDeviceDescriptor>, SourceError> {
        let mut by_name: BTreeMap<String, VideoDeviceDescriptor> = BTreeMap::new();

        for device in
            list_pipewire_video_sources(self.remote_name.as_deref(), &self.exclude_sink_name)?
        {
            by_name.insert(device.name.clone(), device);
        }

        let guard = self
            .imported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (name, entry) in guard.iter() {
            by_name.insert(name.clone(), entry.descriptor.clone());
        }

        Ok(by_name.into_values().collect())
    }

    fn open_source(
        &self,
        target: Option<&str>,
        should_stop: Box<dyn Fn() -> bool + Send>,
    ) -> Result<Box<dyn VideoSource>, SourceError> {
        let mut cfg = PipeWireSourceConfig::new(target.map(ToOwned::to_owned));
        cfg.remote_name.clone_from(&self.remote_name);
        let source = PipeWireVideoSource::connect(cfg)?.with_shutdown_check(should_stop);
        Ok(Box::new(source))
    }

    fn import_video(&self, path: &Path) -> Result<VideoDeviceDescriptor, SourceError> {
        let decoded = self
            .decoder
            .decode(path, self.import_dimensions, self.max_import_frames)?;
        let base_name = slugify_video_node_name(path);
        let node_name = self.unique_node_name(&base_name);
        let file_label = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&node_name);
        let description = file_label.to_string();

        let mut sink_cfg = PipeWireSinkConfig::new(&node_name, &description, decoded.dimensions);
        sink_cfg.remote_name.clone_from(&self.remote_name);
        let sink = PipeWireVideoSink::connect(sink_cfg)?;
        let publisher = LoopingVideoPublisher::start(decoded, Box::new(sink))?;

        let descriptor = VideoDeviceDescriptor {
            name: node_name.clone(),
            description,
            is_imported_video: true,
            video_path: Some(path.display().to_string()),
        };

        self.imported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                node_name.clone(),
                ImportedEntry {
                    descriptor: descriptor.clone(),
                    _publisher: publisher,
                },
            );

        info!(node = %node_name, path = %path.display(), "imported looping video as PipeWire device");
        Ok(descriptor)
    }

    fn remove_video(&self, name: &str) -> Result<(), SourceError> {
        let removed = self
            .imported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(name);
        if removed.is_none() {
            return Err(
                PipeWireError::Buffer(format!("no imported video device named '{name}'")).into(),
            );
        }
        info!(node = %name, "removed imported looping video device");
        Ok(())
    }
}
