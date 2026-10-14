use crate::certificates::*;
use gtk4::{
    Application, ApplicationWindow, Box, Button, Label, Orientation, ScrolledWindow, glib,
    prelude::*,
};
use std::{cell::RefCell, rc::Rc, time::Duration};
fn text(value: &str) -> Label {
    let label = Label::new(Some(&display_text(value, 1024)));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_selectable(true);
    label
}
fn dismissal(window: &ApplicationWindow) {
    let keys = gtk4::EventControllerKey::new();
    let weak = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk4::gdk::Key::Escape {
            if let Some(w) = weak.upgrade() {
                w.close();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
}
fn details(
    app: &Application,
    parent: &ApplicationWindow,
    cert: &PublicCertificate,
    context: &PublicContext,
) {
    let body = Box::new(Orientation::Vertical, 12);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    for line in [
        format!("Holder: {}", cert.holder),
        format!("PIN context: {}", context.label),
        format!("Module: {}", context.key.module_name),
        format!("Issuer: {}", cert.issuer),
        format!("Valid from: {}", cert.not_before),
        format!("Valid until: {}", cert.not_after),
        format!("Purpose: {}", cert.purpose),
        cert.eligibility.label().into(),
    ] {
        body.append(&text(&line));
    }
    let close = Button::with_label("Close");
    body.append(&close);
    let scroll = ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .build();
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Certificate details")
        .transient_for(parent)
        .default_width(520)
        .default_height(460)
        .child(&scroll)
        .build();
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(w) = weak.upgrade() {
            w.close();
        }
    });
    dismissal(&window);
    window.present();
}
pub(super) struct CertificateWindow {
    pub window: ApplicationWindow,
    controller: CertificateController,
    status: Label,
    body: Box,
    actions: RefCell<Vec<Button>>,
    version: RefCell<Option<(u64, u64)>>,
    refresh_button: Button,
    clear_button: Button,
}
impl CertificateWindow {
    pub fn new(app: &Application, controller: CertificateController) -> Rc<Self> {
        let root = Box::new(Orientation::Vertical, 12);
        root.set_margin_start(18);
        root.set_margin_end(18);
        root.set_margin_top(18);
        root.set_margin_bottom(18);
        let status = text("");
        root.append(&status);
        let refresh_button = Button::with_label("Refresh");
        root.append(&refresh_button);
        let clear_button = Button::with_label("Use portal selection and forget saved choice");
        root.append(&clear_button);
        root.append(&text("After changing selection, reload/reselect certificates in the portal. PINs are never saved."));
        let body = Box::new(Orientation::Vertical, 18);
        let scroll = ScrolledWindow::builder()
            .child(&body)
            .vexpand(true)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();
        root.append(&scroll);
        let close = Button::with_label("Close");
        root.append(&close);
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Cards and certificates")
            .default_width(560)
            .default_height(560)
            .child(&root)
            .build();
        let view = Rc::new(Self {
            window,
            controller,
            status,
            body,
            actions: RefCell::new(vec![]),
            version: RefCell::new(None),
            refresh_button,
            clear_button,
        });
        let catalog = view.controller.clone();
        view.refresh_button.connect_clicked(move |_| {
            let _ = catalog.request_refresh();
        });
        let catalog = view.controller.clone();
        view.clear_button.connect_clicked(move |_| {
            let _ = catalog.request_clear(true);
        });
        let weak = view.window.downgrade();
        close.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.close();
            }
        });
        dismissal(&view.window);
        let updating = view.clone();
        glib::timeout_add_local(Duration::from_millis(500), move || {
            if !updating.window.is_visible() {
                return glib::ControlFlow::Break;
            }
            updating.refresh();
            glib::ControlFlow::Continue
        });
        view.refresh();
        view
    }
    pub fn refresh(&self) {
        let snapshot = self.controller.snapshot();
        let mut status = format!(
            "{}\n{}",
            catalog_status_text(&snapshot),
            selection_status_text(&snapshot)
        );
        if let Some(error) = snapshot.error {
            status.push_str(&format!("\n{error}"));
        }
        self.status.set_text(&status);
        let enabled = !matches!(
            snapshot.status,
            CatalogStatus::Busy | CatalogStatus::Loading
        );
        self.refresh_button.set_sensitive(enabled);
        self.clear_button.set_sensitive(enabled);
        if *self.version.borrow() != Some((snapshot.generation, snapshot.revision)) {
            self.version
                .replace(Some((snapshot.generation, snapshot.revision)));
            while let Some(child) = self.body.first_child() {
                self.body.remove(&child);
            }
            self.actions.borrow_mut().clear();
            for context in &snapshot.contexts {
                let heading = text(&display_text(&context.label, 160));
                heading.add_css_class("heading");
                self.body.append(&heading);
                self.body
                    .append(&text(&format!("Module: {}", context.key.module_name)));
                if context.requires_authentication {
                    self.body.append(&text(
                        "Certificates require authentication; choose this context in the portal",
                    ));
                } else if context.certificates.is_empty() {
                    self.body
                        .append(&text("No public signing certificates in this PIN context"));
                }
                for cert in &context.certificates {
                    let selected = snapshot.selected.as_ref() == Some(&cert.key);
                    self.body.append(&text(&format!(
                        "{}{}",
                        if selected { "✓ " } else { "" },
                        display_text(&cert.holder, 160)
                    )));
                    self.body.append(&text(&format!(
                        "Issuer: {}",
                        display_text(&cert.issuer, 200)
                    )));
                    self.body
                        .append(&text(&format!("Valid until: {}", cert.not_after)));
                    self.body.append(&text(&cert.purpose));
                    self.body.append(&text(cert.eligibility.label()));
                    let details_button = Button::with_label("Details");
                    self.body.append(&details_button);
                    let cert_details = cert.clone();
                    let context_details = context.clone();
                    let weak = self.window.downgrade();
                    details_button.connect_clicked(move |_| {
                        if let Some(parent) = weak.upgrade()
                            && let Some(app) = parent.application()
                        {
                            details(&app, &parent, &cert_details, &context_details);
                        }
                    });
                    if cert.eligibility == Eligibility::Eligible
                        && snapshot
                            .contexts
                            .iter()
                            .flat_map(|c| &c.certificates)
                            .filter(|c| c.key == cert.key)
                            .count()
                            == 1
                    {
                        for (label, remember) in
                            [("Use for this session", false), ("Use and remember", true)]
                        {
                            let button = Button::with_label(label);
                            let catalog = self.controller.clone();
                            let key = cert.key.clone();
                            let generation = snapshot.generation;
                            button.connect_clicked(move |_| {
                                let _ = catalog.request_select(key.clone(), remember, generation);
                            });
                            self.body.append(&button);
                            self.actions.borrow_mut().push(button);
                        }
                    }
                }
            }
        }
        for button in self.actions.borrow().iter() {
            button.set_sensitive(enabled);
        }
    }
}
