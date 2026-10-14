use crate::{
    desktop::{Activity, DesktopController, Outcome, TrayAvailability},
    diagnostics::Level,
};
use gtk4::{
    Application, ApplicationWindow, Box, Button, CheckButton, DropDown, Label, Orientation,
    SpinButton, glib, prelude::*,
};

pub(super) struct ControlWindow {
    pub window: ApplicationWindow,
    status: Label,
    endpoint: Label,
    activity: Label,
    diagnostics: Label,
    pub enable: Button,
    pub disable: Button,
    error: Label,
    dismiss: Button,
}
fn label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}
pub(super) fn activity_label(activity: Activity) -> &'static str {
    match activity {
        Activity::Starting => "Starting",
        Activity::Ready => "Service ready",
        Activity::Working => "Working",
        Activity::WaitingForPin => "Waiting for PIN",
        Activity::Stopping => "Stopping",
    }
}
impl ControlWindow {
    pub fn new(app: &Application, controller: DesktopController) -> Self {
        let content = Box::new(Orientation::Vertical, 12);
        content.set_margin_start(18);
        content.set_margin_end(18);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        let status = label("Starting");
        status.add_css_class("title-2");
        let endpoint = label("");
        let activity = label("");
        let certificates = Button::with_label("Cards and certificates…");
        certificates.connect_clicked({
            let app = app.clone();
            let catalog = controller.certificates().clone();
            move |_| {
                if let Some(window) = app
                    .windows()
                    .into_iter()
                    .find(|w| w.title().as_deref() == Some("Cards and certificates"))
                {
                    window.present();
                } else {
                    super::certificates::CertificateWindow::new(&app, catalog.clone())
                        .window
                        .present();
                }
            }
        });
        let heading = label("Diagnostics");
        heading.add_css_class("heading");
        let diagnostics = label("");
        let error = label("");
        error.add_css_class("error");
        let level = DropDown::from_strings(&["Error", "Warning", "Info", "Debug", "Trace"]);
        level.set_selected(3);
        level.update_property(&[gtk4::accessible::Property::Label("Diagnostic log level")]);
        let masked = CheckButton::with_label("Trace requests with sensitive data masked");
        let lifetime = SpinButton::with_range(1.0, 3600.0, 1.0);
        lifetime.set_value(300.0);
        lifetime.update_property(&[gtk4::accessible::Property::Label(
            "Diagnostic lifetime in seconds",
        )]);
        let settings = Box::new(Orientation::Horizontal, 12);
        settings.append(&label("Log level"));
        settings.append(&level);
        let timing = Box::new(Orientation::Horizontal, 12);
        timing.append(&label("Lifetime (seconds)"));
        timing.append(&lifetime);
        let enable = Button::with_label("Enable troubleshooting logs");
        let disable = Button::with_label("Disable troubleshooting logs");
        let actions = Box::new(Orientation::Horizontal, 12);
        actions.append(&enable);
        actions.append(&disable);
        let quit = Button::with_label("Quit Epoxy");
        quit.set_halign(gtk4::Align::End);
        for widget in [&status, &endpoint, &activity, &heading, &diagnostics] {
            content.append(widget);
        }
        content.append(&certificates);
        content.append(&settings);
        content.append(&masked);
        content.append(&timing);
        content.append(&actions);
        content.append(&error);
        content.append(&gtk4::Separator::new(Orientation::Horizontal));
        let dismiss = Button::with_label("Close window");
        let footer = Box::new(Orientation::Horizontal, 12);
        footer.set_halign(gtk4::Align::End);
        footer.append(&dismiss);
        footer.append(&quit);
        content.append(&footer);
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Epoxy")
            .default_width(460)
            .child(&content)
            .build();
        dismiss.connect_clicked({
            let window = window.downgrade();
            move |_| {
                if let Some(window) = window.upgrade() {
                    window.close();
                }
            }
        });
        let keys = gtk4::EventControllerKey::new();
        keys.connect_key_pressed({
            let window = window.downgrade();
            move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    if let Some(window) = window.upgrade() {
                        window.close();
                    }
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        window.add_controller(keys);
        enable.connect_clicked({
            let controller = controller.clone();
            let error = error.clone();
            move |_| {
                let chosen = match level.selected() {
                    0 => Level::Error,
                    1 => Level::Warn,
                    2 => Level::Info,
                    3 => Level::Debug,
                    _ => Level::Trace,
                };
                let result = controller.diagnostics().enable(
                    chosen,
                    masked.is_active(),
                    lifetime.value_as_int() as u64,
                );
                error.set_text(if result.is_ok() {
                    ""
                } else {
                    "Could not enable diagnostics. See operational logs."
                });
            }
        });
        disable.connect_clicked({
            let controller = controller.clone();
            let error = error.clone();
            move |_| {
                let result = controller.diagnostics().disable();
                error.set_text(if result.is_ok() {
                    ""
                } else {
                    "Could not disable diagnostics. See operational logs."
                });
            }
        });
        quit.connect_clicked({
            let controller = controller.clone();
            move |_| controller.request_quit()
        });
        window.connect_close_request(move |window| {
            if controller.snapshot().tray == TrayAvailability::Available {
                window.set_visible(false);
            } else {
                controller.request_quit();
            }
            glib::Propagation::Stop
        });
        Self {
            window,
            status,
            endpoint,
            activity,
            diagnostics,
            enable,
            disable,
            error,
            dismiss,
        }
    }
    pub fn refresh(&self, controller: &DesktopController) {
        let snapshot = controller.snapshot();
        self.status.set_text(activity_label(snapshot.activity));
        self.dismiss
            .set_label(if snapshot.tray == TrayAvailability::Available {
                "Hide window"
            } else {
                "Close window"
            });
        self.endpoint.set_text(
            &snapshot
                .endpoint
                .map(|address| format!("Listening on {address}"))
                .unwrap_or_else(|| "Preparing local service".into()),
        );
        let outcome = match snapshot.last_result {
            None => "No completed operation",
            Some(Outcome::Succeeded) => "Last operation succeeded",
            Some(Outcome::Cancelled) => "Last operation cancelled",
            Some(Outcome::Failed) => "Last operation failed — see operational logs",
        };
        self.activity.set_text(&format!(
            "{} connection(s)\n{outcome}",
            snapshot.connections
        ));
        if let Ok(status) = controller.diagnostics().status() {
            let text = match status.expires_in_seconds {
                Some(seconds) => format!(
                    "Active: {}\nRequest traces: {}\nExpires in {seconds} seconds",
                    status.filter,
                    if status.requests_masked {
                        "masked"
                    } else {
                        "off"
                    }
                ),
                None => format!("Startup settings: {}\nRequest traces off", status.filter),
            };
            self.diagnostics.set_text(&text);
            self.disable
                .set_sensitive(status.expires_in_seconds.is_some());
        } else {
            self.error.set_text("Diagnostic status unavailable");
        }
        self.enable
            .set_sensitive(snapshot.activity != Activity::Stopping);
    }
}
