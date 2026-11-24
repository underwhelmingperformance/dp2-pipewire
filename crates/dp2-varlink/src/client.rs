// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use dp2_core::EffectConfig;
use zlink::{
    proxy,
    tokio::unix::{Stream, connect},
};

use crate::{
    AckReply, Dp2ServiceError, EffectInfo, EffectReply, EffectSettings, EffectsReply, ModelInfo,
    ModelReply, ModelsReply, PreviewFrameInfo, PreviewReply, SourceDeviceInfo, SourceReply,
    SourcesReply, StatusReply, StatusSnapshot, VarlinkError,
};

#[proxy("uk.org.orangesquash.dp2")]
trait Dp2Proxy {
    async fn get_status(&mut self) -> zlink::Result<Result<StatusReply, ()>>;
    async fn list_effects(&mut self) -> zlink::Result<Result<EffectsReply, ()>>;
    async fn get_effect(&mut self) -> zlink::Result<Result<EffectReply, ()>>;
    async fn set_effect(
        &mut self,
        effect: &EffectSettings,
    ) -> zlink::Result<Result<EffectReply, Dp2ServiceError>>;
    async fn list_sources(&mut self) -> zlink::Result<Result<SourcesReply, Dp2ServiceError>>;
    async fn set_source(
        &mut self,
        name: Option<&str>,
    ) -> zlink::Result<Result<SourceReply, Dp2ServiceError>>;
    async fn import_video(
        &mut self,
        path: &str,
        select: Option<bool>,
    ) -> zlink::Result<Result<SourceReply, Dp2ServiceError>>;
    async fn remove_video(
        &mut self,
        name: &str,
    ) -> zlink::Result<Result<AckReply, Dp2ServiceError>>;
    async fn list_models(&mut self) -> zlink::Result<Result<ModelsReply, ()>>;
    async fn set_model(&mut self, id: &str) -> zlink::Result<Result<ModelReply, Dp2ServiceError>>;
    async fn get_preview(
        &mut self,
        max_width: Option<u32>,
    ) -> zlink::Result<Result<PreviewReply, Dp2ServiceError>>;
    async fn reset_depth_state(&mut self) -> zlink::Result<Result<AckReply, ()>>;
    async fn shutdown(&mut self) -> zlink::Result<Result<AckReply, ()>>;
}

/// Typed Varlink client for controlling a running `dp2` daemon.
pub struct Dp2VarlinkClient {
    connection: zlink::Connection<Stream>,
}

impl Dp2VarlinkClient {
    /// Connects to a `dp2` Varlink Unix socket at `socket_path`.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError::Transport`] if connecting to the Unix socket
    /// fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let _status = client.get_status().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect<P: AsRef<Path>>(socket_path: P) -> Result<Self, VarlinkError> {
        let connection = connect(socket_path.as_ref()).await?;
        Ok(Self { connection })
    }

    /// Queries the current [`StatusSnapshot`] from the running daemon.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if the RPC call fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let status = client.get_status().await?;
    /// assert_eq!(status.sink_name, "dp2-depth");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_status(&mut self) -> Result<StatusSnapshot, VarlinkError> {
        let reply = self
            .connection
            .get_status()
            .await?
            .map_err(|()| unexpected_unit_error())?;
        Ok(reply.status)
    }

    /// Lists all frame effects registered in the running daemon.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if the RPC call fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let effects = client.list_effects().await?;
    /// assert!(!effects.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_effects(&mut self) -> Result<Vec<EffectInfo>, VarlinkError> {
        let reply = self
            .connection
            .list_effects()
            .await?
            .map_err(|()| unexpected_unit_error())?;
        Ok(reply.effects)
    }

    /// Queries the currently active [`EffectSettings`].
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if the RPC call fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let _effect = client.get_effect().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_effect(&mut self) -> Result<EffectSettings, VarlinkError> {
        let reply = self
            .connection
            .get_effect()
            .await?
            .map_err(|()| unexpected_unit_error())?;
        Ok(reply.effect)
    }

    /// Replaces the active effect at runtime using a strongly typed
    /// [`EffectConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError::Service`] if the daemon rejects the effect or
    /// parameters, or [`VarlinkError::Transport`] on IPC failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_core::EffectConfig;
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let _applied = client.set_effect(&EffectConfig::Passthrough).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_effect(
        &mut self,
        config: &EffectConfig,
    ) -> Result<EffectSettings, VarlinkError> {
        let wire = EffectSettings::from(config);
        let reply = self.connection.set_effect(&wire).await??;
        Ok(reply.effect)
    }

    /// Lists available PipeWire video sources and imported looping videos.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if discovery or IPC fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let _sources = client.list_sources().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_sources(&mut self) -> Result<Vec<SourceDeviceInfo>, VarlinkError> {
        let reply = self.connection.list_sources().await??;
        Ok(reply.sources)
    }

    /// Switches the active input video source at runtime.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if opening the source or IPC fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let _source = client.set_source(Some("my-camera")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_source(&mut self, name: Option<&str>) -> Result<SourceReply, VarlinkError> {
        let reply = self.connection.set_source(name).await??;
        Ok(reply)
    }

    /// Imports a video file as a looping PipeWire source device and optionally
    /// switches to it.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if decoding, publishing, or IPC fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let _source = client.import_video("/tmp/clip.webm", true).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn import_video(
        &mut self,
        path: &str,
        select: bool,
    ) -> Result<SourceReply, VarlinkError> {
        let reply = self.connection.import_video(path, Some(select)).await??;
        Ok(reply)
    }

    /// Removes a previously imported looping video device.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if the device does not exist or IPC fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// client.remove_video("dp2-video-clip").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn remove_video(&mut self, name: &str) -> Result<(), VarlinkError> {
        let _ = self.connection.remove_video(name).await??;
        Ok(())
    }

    /// Lists configured Depth Pro 2 model variants.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if IPC fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let _models = client.list_models().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_models(&mut self) -> Result<Vec<ModelInfo>, VarlinkError> {
        let reply = self
            .connection
            .list_models()
            .await?
            .map_err(|()| unexpected_unit_error())?;
        Ok(reply.models)
    }

    /// Switches the active Depth Pro 2 model variant at runtime.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if loading the model or IPC fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let _model = client.set_model("b-stream4").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_model(&mut self, id: &str) -> Result<String, VarlinkError> {
        let reply = self.connection.set_model(id).await??;
        Ok(reply.model)
    }

    /// Fetches a downsampled RGBA8 preview frame of the most recent output.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if preview generation or IPC fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// let _preview = client.get_preview(Some(240)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_preview(
        &mut self,
        max_width: Option<u32>,
    ) -> Result<Option<PreviewFrameInfo>, VarlinkError> {
        let reply = self.connection.get_preview(max_width).await??;
        Ok(reply.frame)
    }

    /// Requests a reset of the Depth Pro 2 temporal streaming state.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if the RPC call fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// client.reset_depth_state().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reset_depth_state(&mut self) -> Result<(), VarlinkError> {
        let _ = self
            .connection
            .reset_depth_state()
            .await?
            .map_err(|()| unexpected_unit_error())?;
        Ok(())
    }

    /// Requests clean shutdown of the running daemon.
    ///
    /// # Errors
    ///
    /// Returns [`VarlinkError`] if the RPC call fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use dp2_varlink::Dp2VarlinkClient;
    ///
    /// # async fn example() -> Result<(), dp2_varlink::VarlinkError> {
    /// let mut client = Dp2VarlinkClient::connect("/tmp/dp2.sock").await?;
    /// client.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn shutdown(&mut self) -> Result<(), VarlinkError> {
        let _ = self
            .connection
            .shutdown()
            .await?
            .map_err(|()| unexpected_unit_error())?;
        Ok(())
    }
}

fn unexpected_unit_error() -> VarlinkError {
    VarlinkError::Service(Dp2ServiceError::InvalidParameter {
        message: "unexpected empty error reply from service".into(),
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, os::unix::net::UnixListener as StdUnixListener, sync::Arc};

    use assert_matches::assert_matches;
    use dp2_core::{
        Colormap, DepthMapConfig, EffectConfig, EffectId, EffectRegistry, PipelineController,
        SinkInfo, SourceInfo,
        testing::{FakeModelFactory, FakeSourceFactory},
    };
    use pretty_assertions::assert_eq;
    use tokio::sync::oneshot;

    use super::*;
    use crate::{serve_on_listener, serve_on_socket};

    #[tokio::test]
    async fn varlink_round_trip_status_list_swap_and_shutdown()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let socket_path = dir.path().join("dp2.sock");
        let controller = PipelineController::new(
            EffectRegistry::with_builtin_effects(),
            EffectConfig::default(),
            SourceInfo {
                name: "cam0".into(),
                description: "USB Camera".into(),
            },
            SinkInfo {
                name: "dp2-depth".into(),
                description: "Depth Pro 2 Camera".into(),
            },
        )?;

        let source_factory = Arc::new(FakeSourceFactory::new());
        source_factory.register_empty("cam0", "USB Camera");
        source_factory.register_empty("cam1", "Desk Camera");
        controller.set_source_factory(source_factory);

        let model_factory = Arc::new(FakeModelFactory::new());
        model_factory.register_constant("s-stream4", "Depth Pro 2 S", "S (24M)", 1.0);
        model_factory.register_constant("b-stream4", "Depth Pro 2 B", "B (97M)", 2.0);
        controller.set_model_factory(model_factory);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server_fut = serve_on_socket(&socket_path, controller.clone(), async move {
            let _ = shutdown_rx.await;
        });

        let client_fut = async {
            let mut client = Dp2VarlinkClient::connect(&socket_path).await?;

            let effects = client.list_effects().await?;
            assert_eq!(effects.len(), 2);

            let sources = client.list_sources().await?;
            assert_eq!(sources.len(), 2);
            assert_eq!(sources[0].name, "cam0");
            assert_eq!(sources[1].name, "cam1");

            let switched = client.set_source(Some("cam1")).await?;
            assert_eq!(switched.name, "cam1");

            let imported = client.import_video("/tmp/sample.webm", true).await?;
            assert_eq!(imported.name, "dp2-video-sample");
            client.remove_video("dp2-video-sample").await?;

            let models = client.list_models().await?;
            assert_eq!(models.len(), 2);
            let active_model = client.set_model("b-stream4").await?;
            assert_eq!(active_model, "b-stream4");

            assert_eq!(client.get_preview(Some(240)).await?, None);

            let swapped = client
                .set_effect(&EffectConfig::DepthMap(DepthMapConfig {
                    colormap: Colormap::Viridis,
                    min_depth_meters: Some(0.5),
                    max_depth_meters: Some(6.0),
                    invert: true,
                }))
                .await?;
            assert_eq!(client.get_effect().await?, swapped);

            let err = client
                .set_effect(&EffectConfig::Custom {
                    id: EffectId::new("nonexistent"),
                    parameters: BTreeMap::new(),
                })
                .await;
            assert_matches!(
                err,
                Err(VarlinkError::Service(Dp2ServiceError::UnknownEffect { id }))
                    if id == "nonexistent"
            );

            client.set_effect(&EffectConfig::Passthrough).await?;
            let status = client.get_status().await?;
            assert_eq!(status.sink_name, "dp2-depth");
            assert_eq!(status.active_effect.id, "passthrough");
            assert_eq!(status.active_model.as_deref(), Some("b-stream4"));

            client.reset_depth_state().await?;
            client.shutdown().await?;
            assert!(controller.is_shutdown_requested());

            let _ = shutdown_tx.send(());
            Ok::<(), VarlinkError>(())
        };

        let (server_res, client_res) = tokio::join!(server_fut, client_fut);
        server_res?;
        client_res?;
        Ok(())
    }

    #[tokio::test]
    async fn serve_on_inherited_listener_preserves_socket_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let socket_path = dir.path().join("systemd-inherited.sock");
        let std_listener = StdUnixListener::bind(&socket_path)?;

        let controller = PipelineController::new(
            EffectRegistry::with_builtin_effects(),
            EffectConfig::Passthrough,
            SourceInfo {
                name: "none".into(),
                description: "Idle".into(),
            },
            SinkInfo {
                name: "dp2-depth".into(),
                description: "Depth".into(),
            },
        )?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server_fut = serve_on_listener(std_listener, controller, async move {
            let _ = shutdown_rx.await;
        });

        let client_fut = async {
            let mut client = Dp2VarlinkClient::connect(&socket_path).await?;
            let status = client.get_status().await?;
            assert_eq!(status.sink_name, "dp2-depth");
            let _ = shutdown_tx.send(());
            Ok::<(), VarlinkError>(())
        };

        let (server_res, client_res) = tokio::join!(server_fut, client_fut);
        server_res?;
        client_res?;
        // Systemd owns the socket inode, so serve_on_listener leaves it in
        // place.
        assert!(socket_path.exists());
        Ok(())
    }
}
