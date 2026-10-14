use epoxy::{config, desktop, diagnostics, nss, pin, xmlsec};
mod proto;
mod server;
#[cfg(feature = "ui-gtk")]
use epoxy::ui;

use std::{cell::LazyCell, io, rc::Rc, time::Duration};

use anyhow::{Context, Result};
use futures::{FutureExt, StreamExt, future::LocalBoxFuture};
use tokio::{
    net::{TcpListener, TcpStream},
    select,
};
use tokio_debouncer::{DebounceMode, Debouncer};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_tungstenite::accept_async;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::{
    config::{PinPromptKind, get_config},
    nss::Nss,
    pin::tty_pin_method,
    server::{Server, ServerConfig},
    xmlsec::XmlSec,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let config = get_config()?;

    let diagnostics = init_tracing(&config.log).context("failed to initialize tracing")?;
    let runtime = diagnostics::control::runtime_dir()?;
    let _control = diagnostics::control::ControlServer::start(diagnostics.clone(), &runtime)
        .context("failed to initialize diagnostics controls")?;

    let (certificates, mut certificate_rx) = epoxy::certificates::CertificateController::new();
    let saved = epoxy::certificates::PreferenceStore::new(config.selection_path.clone().into())
        .load()
        .context(
            "Certificate preference could not be loaded; inspect --selection-path before retrying",
        )?;
    if let Some(key) = saved {
        certificates.commit_selection(Some(key), true);
    }
    let controller =
        desktop::DesktopController::with_certificates(diagnostics.clone(), certificates.clone());
    let _lifetime = controller.service_lifetime();
    let listener = get_listener(config.endpoint).await?;
    controller.set_ready(listener.local_addr()?);
    let pin_method_ct = CancellationToken::new();
    #[cfg(feature = "tray")]
    let mut tray = None;
    #[cfg(feature = "ui-gtk")]
    let gui = matches!(config.pin_prompt, PinPromptKind::Ui);
    #[cfg(not(feature = "ui-gtk"))]
    let gui = false;
    let (pin_prompt, mut pin_method_fut) = match config.pin_prompt {
        PinPromptKind::Tty => tty_pin_method(pin_method_ct.clone()),
        #[cfg(feature = "ui-gtk")]
        PinPromptKind::Ui => {
            let (method, handle) = ui::desktop_pin_method(
                controller.clone(),
                config.show_window,
                pin_method_ct.clone(),
            );
            #[cfg(feature = "tray")]
            {
                tray = desktop::tray::start_tray(controller.clone(), handle).ok();
            }
            #[cfg(not(feature = "tray"))]
            {
                let _ = handle;
                controller.set_tray(desktop::TrayAvailability::Unavailable);
            }
            method
        }
    };

    let nss_xmlsec = LazyCell::new(move || {
        let nss = Nss::initialize(config.nssdb_path).context("NSS initialization failed")?;
        debug!("NSS initialized");
        let xmlsec = XmlSec::initialize().context("XMLSec initialization failed")?;
        debug!("xmlsec initialized");
        Ok((nss, xmlsec))
    });

    let server_config = ServerConfig {
        allow_soft_tokens: config.allow_soft_tokens,
        selection_path: config.selection_path.into(),
    };
    let server = Rc::new(Server::new(
        server_config,
        pin_prompt,
        nss_xmlsec,
        diagnostics.clone(),
        controller.clone(),
    ));
    let server_fut = serve(
        config.max_connections,
        config.max_idle_seconds,
        listener,
        server.clone(),
    );
    let catalog_server = server.clone();
    drop(server);
    if gui {
        // Discover an already connected card without requiring Refresh. The
        // listener is ready and GTK runs independently; native work still uses
        // the same serialized dispatcher as later refresh/portal requests.
        certificates
            .request_refresh()
            .context("failed to queue initial public certificate discovery")?;
    }
    let catalog_fut = async move {
        // Let the other service futures be polled before native discovery.
        tokio::task::yield_now().await;
        while let Some(command) = certificate_rx.recv().await {
            catalog_server.certificate_command(command);
        }
    };

    let shutdown = controller.shutdown_requested();
    let ui_exited = select! {
        _ = catalog_fut => false,
        _ = server_fut => false,
        _ = &mut pin_method_fut => true,
        _ = shutdown.cancelled() => false,
    };
    // The service future and all native sessions have dropped before GUI completion.
    certificates.close();
    controller.request_quit();
    drop(_control);
    #[cfg(feature = "tray")]
    drop(tray);
    controller.service_finished();
    pin_method_ct.cancel();
    if gui && !ui_exited {
        pin_method_fut.await;
    }
    info!("service stopped");

    Ok(())
}

fn init_tracing(log_spec: &str) -> Result<diagnostics::Diagnostics> {
    let (diagnostics, subscriber) = diagnostics::subscriber(log_spec, io::stderr)?;
    tracing::subscriber::set_global_default(subscriber)
        .context("failed to install tracing subscriber")?;
    Ok(diagnostics)
}

async fn get_listener(endpoint: String) -> Result<TcpListener> {
    match endpoint.as_ref() {
        "sd-listen" => get_fd_listener(),
        endpoint => get_sockaddr_listener(endpoint).await,
    }
}

#[cfg(feature = "systemd")]
fn get_fd_listener() -> Result<TcpListener> {
    use anyhow::anyhow;
    use sd_notify::NotifyState;
    use std::os::fd::FromRawFd;

    let fd = sd_notify::listen_fds()
        .context("missing fds")?
        .next()
        .ok_or(anyhow!("missing fd"))?;

    let listener = TcpListener::from_std(unsafe { std::net::TcpListener::from_raw_fd(fd) })
        .context("failed to create listener from fd")?;

    let socket_addr = listener.local_addr().context("failed to get socket addr")?;
    info!("listening on {socket_addr} (fd {fd})");

    sd_notify::notify(true, &[NotifyState::Ready]).context("sd_notify failed")?;

    Ok(listener)
}

#[cfg(not(feature = "systemd"))]
fn get_fd_listener() -> Result<TcpListener> {
    use anyhow::anyhow;

    Err(anyhow!(
        "cannot listen on fd: \"systemd\" feature is disabled"
    ))
}

async fn get_sockaddr_listener(endpoint: &str) -> Result<TcpListener> {
    let listener = TcpListener::bind(endpoint)
        .await
        .context(format!("failed to listen on {}", endpoint))?;

    let socket_addr = listener.local_addr().context("failed to get socket addr")?;
    info!("listening on {socket_addr}");

    Ok(listener)
}

fn serve<'a, F>(
    max_connections: u8,
    max_idle_seconds: u32,
    listener: TcpListener,
    server: Rc<Server<F>>,
) -> LocalBoxFuture<'a, ()>
where
    F: FnOnce() -> Result<(Nss, XmlSec)> + 'a,
{
    if max_idle_seconds == 0 {
        return TcpListenerStream::new(listener)
            .map(move |conn| handle_conn(server.clone(), conn))
            .buffer_unordered(max_connections as usize)
            .collect::<()>()
            .boxed_local();
    }

    let timeout = Duration::from_secs(max_idle_seconds as u64);
    let idle_debouncer = Debouncer::new(timeout, DebounceMode::Trailing);
    idle_debouncer.trigger();

    let server_fut = TcpListenerStream::new(listener)
        .map({
            let idle_debouncer = idle_debouncer.clone();
            move |conn| {
                idle_debouncer.clone().trigger();
                handle_conn(server.clone(), conn)
            }
        })
        .buffer_unordered(max_connections as usize)
        .collect::<()>();

    async move {
        select! {
            _ = idle_debouncer.ready() => info!("no new connections accepted within {max_idle_seconds} seconds, shutting down..."),
            _ = server_fut => (),
        }
    }
    .boxed_local()
}

async fn handle_conn<F>(server: Rc<Server<F>>, conn: Result<TcpStream, io::Error>)
where
    F: FnOnce() -> Result<(Nss, XmlSec)>,
{
    let stream = match conn {
        Ok(stream) => stream,
        Err(e) => {
            warn!("failed to accept connection: {e}");
            return;
        }
    };

    let Ok(peer_addr) = stream.peer_addr() else {
        warn!("failed to accept connection: no peer address");
        return;
    };

    let Ok(ws_stream) = accept_async(stream).await else {
        warn!("failed to accept connection from {peer_addr}: WebSocket handshake error");
        return;
    };

    info!("connection accepted: {peer_addr}");

    match server.run(ws_stream).await {
        Ok(()) => {
            info!("connection closed: {peer_addr}");
        }
        Err(_) => {
            warn!(
                error_kind = "protocol_or_transport",
                "connection terminated: {peer_addr}"
            )
        }
    }
}
