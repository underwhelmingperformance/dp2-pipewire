// SPDX-FileCopyrightText: 2026 Iain Lane <iain@orangesquash.org.uk>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    future::Future,
    os::unix::fs::FileTypeExt,
    path::Path,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use dp2_core::{EffectConfig, EffectError, PipelineController, PipelineStatus, StateObserver};
use futures_util::Stream;
use tokio::sync::watch;
use tracing::{debug, info};
use zlink::{Reply, Server, Service, interface, tokio::unix::bind};

use crate::{
    AckReply, DimensionsInfo, Dp2ServiceError, EffectInfo, EffectReply, EffectSettings,
    EffectsReply, StatusReply, StatusSnapshot, VarlinkError,
};

/// Reverse-DNS Varlink interface name exposed by `dp2`.
pub const INTERFACE_NAME: &str = "uk.org.orangesquash.dp2";

struct WatchObserver {
    tx: watch::Sender<StatusSnapshot>,
}

impl StateObserver for WatchObserver {
    fn on_status_changed(&self, status: &PipelineStatus) {
        let _ = self.tx.send(StatusSnapshot::from(status));
    }
}

struct StatusWatchStream {
    rx: watch::Receiver<StatusSnapshot>,
    pending_initial: Option<StatusSnapshot>,
    more: bool,
    waiter: Option<Pin<Box<dyn Future<Output = Option<StatusSnapshot>> + Send>>>,
}

impl StatusWatchStream {
    fn new(rx: watch::Receiver<StatusSnapshot>, more: bool) -> Self {
        let initial = rx.borrow().clone();
        Self {
            rx,
            pending_initial: Some(initial),
            more,
            waiter: None,
        }
    }
}

impl Stream for StatusWatchStream {
    type Item = Reply<StatusReply>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if let Some(snapshot) = this.pending_initial.take() {
            return Poll::Ready(Some(
                Reply::new(Some(StatusReply { status: snapshot })).set_continues(Some(this.more)),
            ));
        }

        if !this.more {
            return Poll::Ready(None);
        }

        if this.waiter.is_none() {
            let mut rx = this.rx.clone();
            this.waiter = Some(Box::pin(async move {
                rx.changed().await.ok()?;
                Some(rx.borrow_and_update().clone())
            }));
        }

        let Some(waiter) = this.waiter.as_mut() else {
            return Poll::Ready(None);
        };

        match waiter.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(snapshot)) => {
                this.waiter = None;
                let _ = this.rx.borrow_and_update();
                Poll::Ready(Some(
                    Reply::new(Some(StatusReply { status: snapshot })).set_continues(Some(true)),
                ))
            }
            Poll::Ready(None) => {
                this.waiter = None;
                Poll::Ready(None)
            }
        }
    }
}

struct Dp2Interface {
    controller: PipelineController,
    status_rx: watch::Receiver<StatusSnapshot>,
}

#[expect(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "zlink::interface awaits every generated method handler"
)]
#[interface(
    interface = "uk.org.orangesquash.dp2",
    types = [DimensionsInfo, EffectInfo, EffectSettings, StatusSnapshot]
)]
impl Dp2Interface {
    async fn get_status(&self) -> StatusReply {
        StatusReply {
            status: StatusSnapshot::from(&self.controller.status()),
        }
    }

    async fn list_effects(&self) -> EffectsReply {
        EffectsReply {
            effects: self
                .controller
                .list_effects()
                .into_iter()
                .map(EffectInfo::from)
                .collect(),
        }
    }

    async fn get_effect(&self) -> EffectReply {
        EffectReply {
            effect: EffectSettings::from(&self.controller.active_effect()),
        }
    }

    async fn set_effect(&mut self, effect: EffectSettings) -> Result<EffectReply, Dp2ServiceError> {
        let config = EffectConfig::try_from(effect).map_err(map_effect_error)?;
        let applied = self
            .controller
            .set_effect(config)
            .map_err(map_effect_error)?;
        Ok(EffectReply {
            effect: EffectSettings::from(&applied),
        })
    }

    async fn reset_depth_state(&mut self) -> AckReply {
        self.controller.request_depth_reset();
        AckReply { ok: true }
    }

    async fn shutdown(&mut self) -> AckReply {
        info!("shutdown requested over Varlink");
        self.controller.request_shutdown();
        AckReply { ok: true }
    }

    #[zlink(more)]
    async fn watch_status(&self, more: bool) -> impl Stream<Item = Reply<StatusReply>> + Unpin {
        StatusWatchStream::new(self.status_rx.clone(), more)
    }
}

#[derive(Service)]
#[zlink(
    vendor = "dp2",
    product = "dp2 PipeWire Depth Pro 2 Camera",
    version = "0.1.0",
    url = "https://github.com/underwhelmingperformance/dp2"
)]
struct Dp2Service(Dp2Interface);

fn map_effect_error(err: EffectError) -> Dp2ServiceError {
    match err {
        EffectError::UnknownEffect(id) => Dp2ServiceError::UnknownEffect { id },
        other => Dp2ServiceError::InvalidParameter {
            message: other.to_string(),
        },
    }
}

/// Binds a Unix domain socket at `socket_path` and serves the
/// `uk.org.orangesquash.dp2` Varlink interface until `shutdown` resolves.
///
/// Any existing stale Unix socket file at `socket_path` is removed before
/// binding, and the socket file is removed on clean exit.
///
/// # Errors
///
/// Returns [`VarlinkError`] if binding the socket or serving connections fails.
///
/// # Examples
///
/// ```no_run
/// use dp2_core::{EffectConfig, EffectRegistry, PipelineController, SinkInfo, SourceInfo};
/// use dp2_varlink::serve_on_socket;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let controller = PipelineController::new(
///     EffectRegistry::with_builtin_effects(),
///     EffectConfig::default(),
///     SourceInfo {
///         name: "in".into(),
///         description: "In".into(),
///     },
///     SinkInfo {
///         name: "out".into(),
///         description: "Out".into(),
///     },
/// )?;
/// serve_on_socket("/tmp/dp2-example.sock", controller, std::future::pending()).await?;
/// # Ok(())
/// # }
/// ```
pub async fn serve_on_socket<P, F>(
    socket_path: P,
    controller: PipelineController,
    shutdown: F,
) -> Result<(), VarlinkError>
where
    P: AsRef<Path>,
    F: Future<Output = ()>,
{
    let socket_path = socket_path.as_ref();
    prepare_socket_path(socket_path)?;

    let initial_status = StatusSnapshot::from(&controller.status());
    let (tx, rx) = watch::channel(initial_status);
    controller.set_observer(Arc::new(WatchObserver { tx }));

    let listener = bind(socket_path)?;
    info!(socket = %socket_path.display(), interface = INTERFACE_NAME, "listening on Varlink socket");

    let service = Dp2Service(Dp2Interface {
        controller,
        status_rx: rx,
    });
    let server = Server::new(listener, service);

    let result = tokio::select! {
        res = server.run() => res.map_err(VarlinkError::from),
        () = shutdown => {
            debug!(socket = %socket_path.display(), "stopping Varlink server");
            Ok(())
        }
    };

    let _ = std::fs::remove_file(socket_path);
    result
}

fn prepare_socket_path(socket_path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = socket_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    match std::fs::symlink_metadata(socket_path) {
        Ok(meta) if meta.file_type().is_socket() => {
            std::fs::remove_file(socket_path)?;
        }
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "refusing to remove non-socket path '{}'",
                    socket_path.display()
                ),
            ));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    Ok(())
}
