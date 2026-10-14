mod certificates;
mod control_window;
mod pin_dialog;
mod prompt;
#[cfg(test)]
mod tests;

use crate::{
    desktop::{DesktopController, TrayAvailability},
    nss::{PinInfo, PinPrompt},
    pin::{PinMethod, SecretPin},
};
use control_window::ControlWindow;
use futures::FutureExt;
use gtk4::{Application, glib, prelude::*};
use pin_dialog::new_pin_popup;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

type PinReply = oneshot::Sender<Option<SecretPin>>;
type PinChannelMessage = (PinInfo, String, PinReply);
#[derive(Clone)]
pub struct UiHandle {
    show: mpsc::Sender<()>,
}
pub(crate) fn presentation_channel() -> (UiHandle, mpsc::Receiver<()>) {
    let (show, rx) = mpsc::channel(1);
    (UiHandle { show }, rx)
}
impl UiHandle {
    pub fn show_window(&self) {
        let _ = self.show.try_send(());
    }
}

pub fn ui_pin_method(ct: CancellationToken) -> PinMethod {
    let (diagnostics, _subscriber) =
        crate::diagnostics::subscriber("info", std::io::sink).expect("valid static log filter");
    let controller = DesktopController::new(diagnostics);
    let (method, _handle) = desktop_pin_method(controller, false, ct);
    method
}

pub fn desktop_pin_method(
    controller: DesktopController,
    show_window: bool,
    ct: CancellationToken,
) -> (PinMethod, UiHandle) {
    let (pin_tx, pin_rx) = mpsc::channel(1);
    let (handle, show_rx) = presentation_channel();
    let ui_controller = controller.clone();
    let future = tokio::task::spawn_blocking(move || {
        run_ui(pin_rx, show_rx, ui_controller, show_window, ct)
    })
    .map(|result| match result {
        Ok(code) if code.get() == 0 => tracing::debug!("desktop UI stopped"),
        _ => tracing::error!(error_kind = "gui_exit", "desktop UI exited unexpectedly"),
    });
    (
        (
            std::boxed::Box::new(UiPrompt {
                sender: pin_tx,
                controller,
            }),
            future.boxed_local(),
        ),
        handle,
    )
}
struct UiPrompt {
    sender: mpsc::Sender<PinChannelMessage>,
    controller: DesktopController,
}
impl PinPrompt for UiPrompt {
    fn prompt_pin(&self, info: &PinInfo, token: String) -> Option<SecretPin> {
        if self.controller.shutdown_requested().is_cancelled() {
            return None;
        }
        let guard = self.controller.pin_requested();
        let (tx, rx) = oneshot::channel();
        let result = self.sender.try_send((info.clone(), token, tx));
        let pin = if result.is_ok() {
            rx.recv().ok().flatten()
        } else {
            tracing::warn!(
                error_kind = "pin_channel_unavailable",
                "PIN dialog unavailable"
            );
            None
        };
        if pin.is_none() {
            guard.cancel();
        }
        pin
    }
}

fn run_ui(
    pin_rx: mpsc::Receiver<PinChannelMessage>,
    show_rx: mpsc::Receiver<()>,
    controller: DesktopController,
    show_window: bool,
    ct: CancellationToken,
) -> glib::ExitCode {
    let app = build_application(pin_rx, show_rx, controller.clone(), show_window, ct);
    // GLib owns these independently of the service runtime, which can be in a native PIN callback.
    let signals = [libc::SIGINT, libc::SIGTERM].map(|signal| {
        let controller = controller.clone();
        glib::unix_signal_add_local(signal, move || {
            controller.request_quit();
            glib::ControlFlow::Continue
        })
    });
    let exit = app.run_with_args(&[] as &[&str]);
    for signal in signals {
        signal.remove();
    }
    controller.request_quit();
    exit
}

fn build_application(
    pin_rx: mpsc::Receiver<PinChannelMessage>,
    show_rx: mpsc::Receiver<()>,
    controller: DesktopController,
    show_window: bool,
    ct: CancellationToken,
) -> Application {
    let ui_started = Instant::now();
    let app = Application::builder()
        .application_id("dev.semyon.epoxy")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let receivers = RefCell::new(Some((pin_rx, show_rx)));
    let control = Rc::new(RefCell::new(None::<Rc<ControlWindow>>));
    let holder = Rc::new(RefCell::new(None));
    let pending = Rc::new(RefCell::new(None::<pin_dialog::PinDialog>));
    let active_controller = controller.clone();
    app.connect_activate(move |app| {
        if let Some(control) = control.borrow().as_ref() {
            control.window.present();
            return;
        }
        let Some((mut pin_rx, mut show_rx)) = receivers.take() else {
            return;
        };
        holder.replace(Some(app.hold()));
        let controls = Rc::new(ControlWindow::new(app, active_controller.clone()));
        controls.refresh(&active_controller);
        control.replace(Some(controls.clone()));
        if show_window {
            controls.window.present();
            tracing::debug!(
                elapsed_ms = ui_started.elapsed().as_millis() as u64,
                reason = "requested",
                "desktop window presented"
            );
        }
        let app_weak = app.downgrade();
        let pending_pin = pending.clone();
        let pin_controller = active_controller.clone();
        glib::spawn_future_local(async move {
            while let Some((info, token, reply)) = pin_rx.recv().await {
                if pin_controller.shutdown_requested().is_cancelled() {
                    prompt::PendingReply::new(reply).cancel();
                    continue;
                }
                let Some(app) = app_weak.upgrade() else {
                    break;
                };
                if let Some(dialog) = pending_pin.take() {
                    dialog.cancel();
                }
                pending_pin.replace(Some(new_pin_popup(&app, info, token, reply)));
            }
        });
        let show_controls = controls.clone();
        glib::spawn_future_local(async move {
            while show_rx.recv().await.is_some() {
                show_controls.window.present();
            }
        });
        let shutdown = active_controller.shutdown_requested();
        let pending_pin = pending.clone();
        let stopping_controls = controls.clone();
        let stopping_controller = active_controller.clone();
        glib::spawn_future_local(async move {
            shutdown.cancelled().await;
            if let Some(dialog) = pending_pin.take() {
                dialog.cancel();
            }
            stopping_controls.refresh(&stopping_controller);
        });
        let app_weak = app.downgrade();
        let pending_pin = pending.clone();
        let held = holder.clone();
        let completion = active_controller.service_stopped();
        let completion_ct = ct.clone();
        glib::spawn_future_local(async move {
            tokio::select! {_=completion.cancelled()=>(),_=completion_ct.cancelled()=>()}
            if let Some(dialog) = pending_pin.take() {
                dialog.cancel();
            }
            held.replace(None);
            if let Some(app) = app_weak.upgrade() {
                app.quit();
            }
        });
        let refresh_controller = active_controller.clone();
        let started = Instant::now();
        let mut fallback_shown = show_window;
        glib::timeout_add_local(Duration::from_secs(1), move || {
            if refresh_controller.service_stopped().is_cancelled() {
                return glib::ControlFlow::Break;
            }
            controls.refresh(&refresh_controller);
            if !fallback_shown
                && started.elapsed() >= Duration::from_secs(2)
                && refresh_controller.snapshot().tray != TrayAvailability::Available
            {
                controls.window.present();
                tracing::debug!(
                    elapsed_ms = ui_started.elapsed().as_millis() as u64,
                    reason = "tray_unavailable",
                    "desktop window presented"
                );
                fallback_shown = true;
            }
            glib::ControlFlow::Continue
        });
        tracing::debug!("desktop UI initialized");
    });
    app
}
