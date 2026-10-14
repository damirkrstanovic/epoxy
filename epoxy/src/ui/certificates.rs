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
        format!("Issuer: {}", cert.issuer),
        format!("Valid from: {}", cert.not_before),
        format!("Valid until: {}", cert.not_after),
        format!("Purpose: {}", cert.purpose),
        cert.eligibility.label().into(),
    ] {
        body.append(&text(&line));
    }
    let advanced = gtk4::Expander::new(Some("Technical details"));
    advanced.set_child(Some(&text(&format!(
        "PIN context: {}; module: {}",
        context.label, context.key.module_name
    ))));
    body.append(&advanced);
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
    clear_options: gtk4::Expander,
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
        let refresh_button = Button::with_label("Refresh certificates");
        root.append(&refresh_button);
        let clear_button = Button::with_label("Forget saved certificate choice");
        let advanced = gtk4::Expander::new(Some("Saved certificate choice"));
        advanced.set_child(Some(&clear_button));
        root.append(&advanced);
        root.append(&text("Your choice is remembered. PINs are never saved. After changing certificates, reload the portal."));
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
            .title("Certificates")
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
            clear_options: advanced,
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
        self.clear_options.set_visible(!snapshot.automatic);
        if *self.version.borrow() != Some((snapshot.generation, snapshot.revision)) {
            self.version
                .replace(Some((snapshot.generation, snapshot.revision)));
            while let Some(child) = self.body.first_child() {
                self.body.remove(&child);
            }
            self.actions.borrow_mut().clear();
            let mut contexts: Vec<_> = snapshot.contexts.iter().collect();
            contexts.sort_by_key(|c| !c.is_signing_context());
            for context in contexts {
                for cert in &context.certificates {
                    let selected = snapshot.selected.as_ref() == Some(&cert.key);
                    self.body.append(&text(&format!(
                        "{}{}",
                        if selected { "✓ " } else { "" },
                        signing_certificate_title(context, cert)
                    )));
                    self.body
                        .append(&text(&format!("Valid until: {}", cert.not_after)));
                    if cert.eligibility != Eligibility::Eligible {
                        self.body.append(&text(cert.eligibility.label()));
                    }
                    let details_button = Button::with_label("Certificate details…");
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
                        let label = if selected {
                            "Used for signing"
                        } else {
                            "Use for signing"
                        };
                        let remember = true;
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
                    self.body.append(&details_button);
                }
            }
        }
        for button in self.actions.borrow().iter() {
            button.set_sensitive(enabled && button.label().as_deref() != Some("Used for signing"));
        }
    }
}
