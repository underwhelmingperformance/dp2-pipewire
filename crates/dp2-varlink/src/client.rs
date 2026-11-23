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
    AckReply, Dp2ServiceError, EffectInfo, EffectReply, EffectSettings, EffectsReply, StatusReply,
    StatusSnapshot, VarlinkError,
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
    use std::collections::BTreeMap;

    use assert_matches::assert_matches;
    use dp2_core::{
        Colormap, DepthMapConfig, EffectConfig, EffectId, EffectRegistry, PipelineController,
        SinkInfo, SourceInfo,
    };
    use pretty_assertions::assert_eq;
    use tokio::sync::oneshot;

    use super::*;
    use crate::serve_on_socket;

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

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server_fut = serve_on_socket(&socket_path, controller.clone(), async move {
            let _ = shutdown_rx.await;
        });

        let client_fut = async {
            let mut client = Dp2VarlinkClient::connect(&socket_path).await?;

            let effects = client.list_effects().await?;
            assert_eq!(
                effects,
                vec![
                    EffectInfo {
                        id: "depth-map".into(),
                        name: "Depth Map".into(),
                        description:
                            "Renders the Depth Pro 2 metric depth map with a configurable colormap"
                                .into(),
                        requires_depth: true,
                    },
                    EffectInfo {
                        id: "passthrough".into(),
                        name: "Passthrough".into(),
                        description:
                            "Forwards the input video stream without running depth estimation"
                                .into(),
                        requires_depth: false,
                    },
                ]
            );

            let swapped = client
                .set_effect(&EffectConfig::DepthMap(DepthMapConfig {
                    colormap: Colormap::Viridis,
                    min_depth_meters: Some(0.5),
                    max_depth_meters: Some(6.0),
                    invert: true,
                }))
                .await?;
            assert_eq!(
                swapped,
                EffectSettings {
                    id: "depth-map".into(),
                    colormap: Some("viridis".into()),
                    min_depth_meters: Some(0.5),
                    max_depth_meters: Some(6.0),
                    invert: Some(true),
                    parameters: None,
                }
            );
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
            assert_eq!(status.source_name, "cam0");
            assert_eq!(status.sink_name, "dp2-depth");
            assert_eq!(status.active_effect.id, "passthrough");

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
}
