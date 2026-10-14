use super::*;
use gtk4::{Button, PasswordEntry, Widget};
use std::time::Duration;
fn pump() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(80);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn descendants(widget: &Widget) -> Vec<Widget> {
    let mut result = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(widget) = child {
        result.extend(descendants(&widget));
        child = widget.next_sibling();
    }
    result
}
fn button(window: &gtk4::Window, name: &str) -> Button {
    descendants(window.upcast_ref())
        .into_iter()
        .filter_map(|w| w.downcast::<Button>().ok())
        .find(|b| b.label().as_deref() == Some(name))
        .unwrap()
}
#[test]
#[ignore = "requires isolated display and session bus; run with xvfb-run dbus-run-session"]
fn desktop_widgets_and_pin_lifecycle() {
    gtk4::init().unwrap();
    let (diagnostics, _subscriber) = crate::diagnostics::subscriber("info", std::io::sink).unwrap();
    let controller = DesktopController::new(diagnostics);
    controller.set_ready("127.0.0.1:17165".parse().unwrap());
    controller.set_tray(TrayAvailability::Available);
    let (pin_tx, pin_rx) = mpsc::channel(1);
    let (show_tx, show_rx) = mpsc::channel(1);
    let app = build_application(
        pin_rx,
        show_rx,
        controller.clone(),
        false,
        CancellationToken::new(),
    );
    app.register(None::<&gtk4::gio::Cancellable>).unwrap();
    app.activate();
    pump();
    let control = app
        .windows()
        .into_iter()
        .find(|w| w.title().as_deref() == Some("Epoxy"))
        .unwrap();
    assert!(!control.is_visible());
    let until = Instant::now() + Duration::from_millis(2200);
    while Instant::now() < until {
        pump();
    }
    assert!(
        !control.is_visible(),
        "a tray host must keep quiet startup from mapping a tiled window"
    );
    app.activate();
    app.activate();
    pump();
    assert_eq!(app.windows().len(), 1);
    button(&control, "Hide window").emit_clicked();
    pump();
    assert!(!control.is_visible());
    assert!(!controller.shutdown_requested().is_cancelled());
    let handle = UiHandle { show: show_tx };
    for _ in 0..100 {
        handle.show_window();
    }
    pump();
    assert!(control.is_visible());
    for action in ["close", "Cancel", "Enter", "Continue", "Escape", "shutdown"] {
        let (tx, rx) = oneshot::channel();
        let worker = if action == "shutdown" {
            let prompt = UiPrompt {
                sender: pin_tx.clone(),
                controller: controller.clone(),
            };
            Some(std::thread::spawn(move || {
                let result = prompt.prompt_pin(
                    &PinInfo {
                        cert: None,
                        reason: "Synthetic signing".into(),
                    },
                    "Synthetic token".into(),
                );
                let _ = tx.send(result);
            }))
        } else {
            pin_tx
                .try_send((
                    PinInfo {
                        cert: Some("Synthetic long certificate name Čertificate ".repeat(4)),
                        reason: "Sign test declaration".into(),
                    },
                    "Synthetic Digital Signature PIN".into(),
                    tx,
                ))
                .unwrap();
            None
        };
        pump();
        let window = app
            .windows()
            .into_iter()
            .find(|w| w.title().as_deref() == Some("Epoxy — Card PIN"))
            .unwrap();
        let entry = descendants(window.upcast_ref())
            .into_iter()
            .find_map(|w| w.downcast::<PasswordEntry>().ok())
            .unwrap();
        assert!(window.is_visible());
        let focused = gtk4::prelude::GtkWindowExt::focus(&window).unwrap();
        assert!(focused == entry.clone().upcast::<Widget>() || focused.is_ancestor(&entry));
        if action == "close"
            && let Ok(path) = std::env::var("EPOXY_UI_CAPTURE")
        {
            assert!(
                std::process::Command::new("import")
                    .args(["-window", "root", &path])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        entry.set_text("synthetic-secret");
        // Diagnostic controls remain usable while a PIN is pending.
        button(&control, "Enable troubleshooting logs").emit_clicked();
        assert!(
            controller
                .diagnostics()
                .status()
                .unwrap()
                .expires_in_seconds
                .is_some()
        );
        button(&control, "Disable troubleshooting logs").emit_clicked();
        assert!(
            controller
                .diagnostics()
                .status()
                .unwrap()
                .expires_in_seconds
                .is_none()
        );
        match action {
            "close" => window.close(),
            "Cancel" => button(&window, "Cancel").emit_clicked(),
            "Enter" => entry.emit_by_name::<()>("activate", &[]),
            "Continue" => button(&window, "Continue").emit_clicked(),
            "Escape" => {
                let controllers = window.observe_controllers();
                let key = controllers
                    .item(0)
                    .unwrap()
                    .downcast::<gtk4::EventControllerKey>()
                    .unwrap();
                key.emit_by_name::<bool>(
                    "key-pressed",
                    &[
                        &gtk4::gdk::Key::Escape,
                        &0u32,
                        &gtk4::gdk::ModifierType::empty(),
                    ],
                );
            }
            _ => controller.request_quit(),
        }
        pump();
        let expected = if action == "Enter" || action == "Continue" {
            Some("synthetic-secret".into())
        } else {
            None
        };
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(100)).unwrap(),
            expected
        );
        if let Some(worker) = worker {
            worker.join().unwrap();
        }
        assert!(entry.text().is_empty());
        assert!(!format!("{:?}", controller.snapshot()).contains("synthetic-secret"));
    }
    controller.service_finished();
    pump();
    // A quiet launch falls back once; repeated host loss never re-presents it.
    let (diagnostics, _subscriber) = crate::diagnostics::subscriber("info", std::io::sink).unwrap();
    let quiet = DesktopController::new(diagnostics);
    quiet.set_tray(TrayAvailability::Unavailable);
    let (_pin, pin_rx) = mpsc::channel(1);
    let (_show, show_rx) = mpsc::channel(1);
    let fallback = build_application(
        pin_rx,
        show_rx,
        quiet.clone(),
        false,
        CancellationToken::new(),
    );
    fallback.set_application_id(Some("dev.semyon.epoxy.FallbackTest"));
    fallback.register(None::<&gtk4::gio::Cancellable>).unwrap();
    fallback.activate();
    pump();
    let window = fallback.windows().first().unwrap().clone();
    assert!(!window.is_visible());
    let until = Instant::now() + Duration::from_millis(2200);
    while Instant::now() < until {
        pump();
    }
    assert!(window.is_visible());
    quiet.set_tray(TrayAvailability::Available);
    window.close();
    pump();
    assert!(!window.is_visible());
    quiet.set_tray(TrayAvailability::Unavailable);
    let until = Instant::now() + Duration::from_millis(1100);
    while Instant::now() < until {
        pump();
    }
    assert!(
        !window.is_visible(),
        "repeated tray loss must not steal focus"
    );
    quiet.service_finished();
    pump();
    // Without a tray, closing the control window requests service shutdown.
    let (diagnostics, _subscriber) = crate::diagnostics::subscriber("info", std::io::sink).unwrap();
    let no_tray = DesktopController::new(diagnostics);
    no_tray.set_tray(TrayAvailability::Unavailable);
    let controls = ControlWindow::new(&app, no_tray.clone());
    controls.window.present();
    pump();
    controls.window.close();
    pump();
    assert!(no_tray.shutdown_requested().is_cancelled());
    controls.window.destroy();
}
#[test]
#[ignore = "requires private Xvfb and D-Bus; synthetic certificates only"]
fn certificate_widgets_show_details_dispatch_choices_and_dismiss_without_quitting() {
    use crate::certificates::*;
    gtk4::init().unwrap();
    let app = Application::builder()
        .application_id("dev.semyon.epoxy.CertificateTest")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk4::gio::Cancellable>).unwrap();
    let (catalog, mut rx) = CertificateController::new();
    let view = super::certificates::CertificateWindow::new(&app, catalog.clone());
    view.window.present();
    pump();
    assert!(
        rx.try_recv().is_err(),
        "opening a view must not probe or authenticate"
    );
    button(view.window.upcast_ref(), "Refresh").emit_clicked();
    assert!(matches!(rx.try_recv(), Ok(CertificateCommand::Refresh)));
    let context = ContextKey {
        module_name: "Synthetic".into(),
        module_library: None,
        token_uri: b"synthetic".to_vec(),
    };
    let mut cert = PublicCertificate::from_der(
        context.clone(),
        include_bytes!("../../tests/fixtures/eligible.der"),
    )
    .unwrap();
    cert.holder = "<b>Željko_Čertificate</b> ".repeat(16);
    let key = cert.key.clone();
    catalog.publish(vec![PublicContext {
        key: context,
        label: "Synthetic Digital Signature PIN".repeat(4),
        requires_authentication: false,
        certificates: vec![cert],
    }]);
    view.refresh();
    pump();
    assert!(
        descendants(view.window.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk4::Label>().ok())
            .any(|l| l.text().contains("<b>Željko") && !l.uses_markup())
    );
    button(view.window.upcast_ref(), "Details").emit_clicked();
    pump();
    let details = app
        .windows()
        .into_iter()
        .find(|w| w.title().as_deref() == Some("Certificate details"))
        .unwrap();
    button(&details, "Close").emit_clicked();
    pump();
    assert!(!details.is_visible());
    button(view.window.upcast_ref(), "Details").emit_clicked();
    pump();
    let details = app
        .windows()
        .into_iter()
        .find(|w| w.title().as_deref() == Some("Certificate details"))
        .unwrap();
    let escape_controller = details
        .observe_controllers()
        .item(0)
        .unwrap()
        .downcast::<gtk4::EventControllerKey>()
        .unwrap();
    escape_controller.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk4::gdk::Key::Escape,
            &0u32,
            &gtk4::gdk::ModifierType::empty(),
        ],
    );
    pump();
    assert!(!details.is_visible());
    if let Ok(path) = std::env::var("EPOXY_CERT_UI_CAPTURE") {
        assert!(
            std::process::Command::new("import")
                .args(["-window", "root", &path])
                .status()
                .unwrap()
                .success()
        );
    }

    button(view.window.upcast_ref(), "Use for this session").emit_clicked();
    match rx.try_recv().unwrap() {
        CertificateCommand::Select {
            key: k,
            remember: false,
            ..
        } => assert!(k == key),
        _ => panic!("session selection expected"),
    }
    catalog.finish(Ok(()));
    view.refresh();
    button(view.window.upcast_ref(), "Use and remember").emit_clicked();
    assert!(matches!(
        rx.try_recv(),
        Ok(CertificateCommand::Select { remember: true, .. })
    ));
    catalog.finish(Ok(()));
    let guard = catalog.native_operation();
    view.refresh();
    assert!(!button(view.window.upcast_ref(), "Use for this session").is_sensitive());
    drop(guard);
    view.refresh();
    button(
        view.window.upcast_ref(),
        "Use portal selection and forget saved choice",
    )
    .emit_clicked();
    assert!(matches!(
        rx.try_recv(),
        Ok(CertificateCommand::Clear { forget_saved: true })
    ));
    button(view.window.upcast_ref(), "Close").emit_clicked();
    pump();
    assert!(!view.window.is_visible());
    app.quit();
}
