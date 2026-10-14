use super::*;
use crate::ui::UiHandle;
use ksni::{MenuItem, Tray, menu::StandardItem};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
struct EpoxyTray {
    controller: DesktopController,
    ui: UiHandle,
    registered: Arc<AtomicBool>,
}
impl Tray for EpoxyTray {
    const MENU_ON_ACTIVATE: bool = true;
    fn id(&self) -> String {
        "dev.semyon.epoxy".into()
    }
    fn title(&self) -> String {
        "Epoxy".into()
    }
    fn icon_name(&self) -> String {
        // Some hosts render symbolic theme icons without recoloring them. Use
        // our contrast-safe pixmaps directly rather than preferring a theme name.
        String::new()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [
            (16, include_bytes!("../../assets/tray-16.argb").as_slice()),
            (24, include_bytes!("../../assets/tray-24.argb").as_slice()),
            (32, include_bytes!("../../assets/tray-32.argb").as_slice()),
            (48, include_bytes!("../../assets/tray-48.argb").as_slice()),
        ]
        .into_iter()
        .map(|(size, data)| ksni::Icon {
            width: size,
            height: size,
            data: data.to_vec(),
        })
        .collect()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Epoxy".into(),
            description: status_label(self.controller.snapshot().activity).into(),
            ..Default::default()
        }
    }
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        self.registered.store(false, Ordering::SeqCst);
        self.controller.set_tray(TrayAvailability::Unavailable);
        // Ordinary absence keeps ksni's ownership monitor alive. A rejected
        // registration must end this attempt so the outer retry can register again.
        matches!(
            reason,
            ksni::OfflineReason::No
                | ksni::OfflineReason::Error(ksni::Error::Watcher(
                    zbus::fdo::Error::ServiceUnknown(_)
                ))
                | ksni::OfflineReason::Error(ksni::Error::WontShow)
        )
    }
    fn watcher_online(&self) {
        self.registered.store(true, Ordering::SeqCst);
        self.controller.set_tray(TrayAvailability::Pending);
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let status = self.controller.diagnostics().status().ok();
        let mut advanced = vec![
            StandardItem {
                label: "Advanced diagnostics…".into(),
                activate: Box::new(|tray: &mut Self| tray.ui.show_window()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
        ];
        advanced.extend(super::certificate_menu::advanced(
            self.controller.certificates(),
        ));
        advanced.push(MenuItem::Separator);
        advanced.push(
            StandardItem {
                label: "Quit Epoxy".into(),
                activate: Box::new(|tray: &mut Self| tray.controller.request_quit()),
                ..Default::default()
            }
            .into(),
        );
        vec![
            StandardItem {
                label: status_label(self.controller.snapshot().activity).into(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            ksni::menu::SubMenu {
                label: "Cards and certificates".into(),
                submenu: super::certificate_menu::build(self.controller.certificates()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Enable troubleshooting logs (5 minutes)".into(),
                activate: Box::new(|tray: &mut Self| {
                    if tray
                        .controller
                        .diagnostics()
                        .enable(crate::diagnostics::Level::Debug, true, 300)
                        .is_err()
                    {
                        tracing::warn!(
                            error_kind = "diagnostics_enable",
                            "could not enable diagnostics"
                        );
                    }
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Disable troubleshooting logs".into(),
                enabled: status.is_some_and(|s| s.expires_in_seconds.is_some()),
                activate: Box::new(|tray: &mut Self| {
                    if tray.controller.diagnostics().disable().is_err() {
                        tracing::warn!(
                            error_kind = "diagnostics_disable",
                            "could not disable diagnostics"
                        );
                    }
                }),
                ..Default::default()
            }
            .into(),
            ksni::menu::SubMenu {
                label: "Advanced".into(),
                submenu: advanced,
                ..Default::default()
            }
            .into(),
        ]
    }
}
fn status_label(activity: Activity) -> &'static str {
    match activity {
        Activity::Starting => "Starting",
        Activity::Ready => "Service ready",
        Activity::Working => "Working",
        Activity::WaitingForPin => "Waiting for PIN",
        Activity::Stopping => "Stopping",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_icon_is_self_contained_and_visible_on_dark_and_light_panels() {
        let (diagnostics, _subscriber) =
            crate::diagnostics::subscriber("info", std::io::sink).unwrap();
        let (ui, _rx) = crate::ui::presentation_channel();
        let tray = EpoxyTray {
            controller: DesktopController::new(diagnostics),
            ui,
            registered: Arc::new(AtomicBool::new(true)),
        };
        // A host may prefer IconName and render symbolic SVG black without recoloring.
        assert!(
            tray.icon_name().is_empty(),
            "use the supplied pixmaps without a theme-dependent name"
        );
        let icons = tray.icon_pixmap();
        assert!(!icons.is_empty());
        for icon in icons {
            assert_eq!(icon.data.len(), (icon.width * icon.height * 4) as usize);
            for background in [24u32, 245u32] {
                let mut low = 255;
                let mut high = 0;
                for pixel in icon.data.chunks_exact(4) {
                    let alpha = pixel[0] as u32;
                    let foreground = (pixel[1] as u32 + pixel[2] as u32 + pixel[3] as u32) / 3;
                    let brightness = (foreground * alpha + background * (255 - alpha)) / 255;
                    low = low.min(brightness);
                    high = high.max(brightness);
                }
                assert!(
                    high - low >= 140,
                    "icon needs a visible light surface and dark outline on either panel"
                );
            }
        }
    }
    fn standard_action<'a>(
        menu: &'a [MenuItem<EpoxyTray>],
        label: &str,
    ) -> &'a StandardItem<EpoxyTray> {
        for item in menu {
            match item {
                MenuItem::Standard(s) if s.label == label => return s,
                MenuItem::SubMenu(s) => {
                    if let Some(found) = find_action(&s.submenu, label) {
                        return found;
                    }
                }
                _ => (),
            }
        }
        panic!("missing action {label}")
    }
    fn find_action<'a>(
        menu: &'a [MenuItem<EpoxyTray>],
        label: &str,
    ) -> Option<&'a StandardItem<EpoxyTray>> {
        for item in menu {
            match item {
                MenuItem::Standard(s) if s.label == label => return Some(s),
                MenuItem::SubMenu(s) => {
                    if let Some(found) = find_action(&s.submenu, label) {
                        return Some(found);
                    }
                }
                _ => (),
            }
        }
        None
    }
    #[test]
    fn tray_actions_keep_logs_at_top_and_windows_and_quit_in_advanced() {
        let (diagnostics, _subscriber) =
            crate::diagnostics::subscriber("warn", std::io::sink).unwrap();
        let controller = DesktopController::new(diagnostics);
        let (ui, mut rx) = crate::ui::presentation_channel();
        let mut tray = EpoxyTray {
            controller: controller.clone(),
            ui,
            registered: Arc::new(AtomicBool::new(true)),
        };
        let menu = tray.menu();
        let labels: Vec<_> = menu
            .iter()
            .filter_map(|item| match item {
                MenuItem::Standard(s) => Some(s.label.as_str()),
                MenuItem::SubMenu(s) => Some(s.label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            vec![
                "Starting",
                "Cards and certificates",
                "Enable troubleshooting logs (5 minutes)",
                "Disable troubleshooting logs",
                "Advanced"
            ]
        );
        (standard_action(&menu, "Enable troubleshooting logs (5 minutes)").activate)(&mut tray);
        assert!(controller.diagnostics().status().unwrap().requests_masked);
        assert!(
            rx.try_recv().is_err(),
            "ordinary tray changes must not present a GTK window"
        );
        (standard_action(&menu, "Disable troubleshooting logs").activate)(&mut tray);
        assert_eq!(controller.diagnostics().status().unwrap().filter, "warn");
        assert!(rx.try_recv().is_err());
        (standard_action(&menu, "Advanced diagnostics…").activate)(&mut tray);
        assert_eq!(rx.try_recv().unwrap(), ());
        let menu = tray.menu();
        assert!(!standard_action(&menu, "Disable troubleshooting logs").enabled);
        controller
            .diagnostics()
            .enable(crate::diagnostics::Level::Debug, false, 1)
            .unwrap();
        assert!(standard_action(&tray.menu(), "Disable troubleshooting logs").enabled);
        controller
            .diagnostics()
            .expire(std::time::Instant::now() + std::time::Duration::from_secs(2))
            .unwrap();
        assert!(!standard_action(&tray.menu(), "Disable troubleshooting logs").enabled);
        (standard_action(&menu, "Quit Epoxy").activate)(&mut tray);
        assert!(controller.shutdown_requested().is_cancelled());
    }
}

#[cfg(test)]
mod bus_tests {
    use super::*;
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };
    struct Watcher {
        registrations: Arc<Mutex<Vec<String>>>,
        stall: bool,
        host: Arc<AtomicBool>,
        failures: Arc<AtomicUsize>,
        attempts: Arc<AtomicUsize>,
    }
    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        async fn register_status_notifier_item(&self, service: &str) -> zbus::fdo::Result<()> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            if self
                .failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(zbus::fdo::Error::Failed(
                    "synthetic temporary rejection".into(),
                ));
            }
            self.registrations.lock().unwrap().push(service.to_owned());
            if self.stall {
                std::future::pending::<()>().await;
            }
            Ok(())
        }
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            self.host.load(Ordering::SeqCst)
        }
        #[zbus(property)]
        fn protocol_version(&self) -> i32 {
            0
        }
        #[zbus(property)]
        fn registered_status_notifier_items(&self) -> Vec<String> {
            self.registrations.lock().unwrap().clone()
        }
    }
    async fn wait_for(controller: &DesktopController, availability: TrayAvailability) {
        let end = Instant::now() + Duration::from_secs(8);
        while controller.snapshot().tray != availability {
            assert!(
                Instant::now() < end,
                "tray did not recover: {:?}",
                controller.snapshot()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    #[test]
    #[ignore = "requires private dbus-run-session and EPOXY_PRIVATE_TEST_BUS=1"]
    fn delayed_watcher_recovery_and_shutdown() {
        assert_eq!(std::env::var("EPOXY_PRIVATE_TEST_BUS").as_deref(), Ok("1"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (diagnostics, _subscriber) =
                crate::diagnostics::subscriber("info", std::io::sink).unwrap();
            let controller = DesktopController::new(diagnostics);
            let (ui, mut ui_requests) = crate::ui::presentation_channel();
            let tray = start_tray(controller.clone(), ui).unwrap();
            wait_for(&controller, TrayAvailability::Unavailable).await;
            tokio::time::sleep(Duration::from_millis(2200)).await;
            let registrations = Arc::new(Mutex::new(Vec::new()));
            let watcher = zbus::connection::Builder::session()
                .unwrap()
                .name("org.kde.StatusNotifierWatcher")
                .unwrap()
                .serve_at(
                    "/StatusNotifierWatcher",
                    Watcher {
                        registrations: registrations.clone(),
                        stall: false,
                        host: Arc::new(AtomicBool::new(true)),
                        failures: Arc::new(AtomicUsize::new(0)),
                        attempts: Arc::new(AtomicUsize::new(0)),
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            wait_for(&controller, TrayAvailability::Available).await;
            assert_eq!(registrations.lock().unwrap().len(), 1);
            let service = registrations.lock().unwrap()[0].clone();
            let item = zbus::Proxy::new(
                &watcher,
                service.as_str(),
                "/StatusNotifierItem",
                "org.kde.StatusNotifierItem",
            )
            .await
            .unwrap();
            let title: String = item.get_property("Title").await.unwrap();
            assert_eq!(title, "Epoxy");
            let icon_name: String = item.get_property("IconName").await.unwrap();
            assert!(icon_name.is_empty());
            let pixmaps: Vec<(i32, i32, Vec<u8>)> = item.get_property("IconPixmap").await.unwrap();
            assert!(!pixmaps.is_empty());
            assert!(pixmaps.iter().all(|(w, h, bytes)| *w > 0
                && *h > 0
                && bytes.len() == (*w as usize) * (*h as usize) * 4));
            let menu_only: bool = item.get_property("ItemIsMenu").await.unwrap();
            assert!(menu_only, "tray activation must show its menu");
            let result = item.call::<_, _, ()>("Activate", &(0i32, 0i32)).await;
            assert!(matches!(result, Err(zbus::Error::MethodError(..))));
            assert!(
                ui_requests.try_recv().is_err(),
                "tray clicks must not request a standalone window"
            );
            drop(item);
            watcher
                .release_name("org.kde.StatusNotifierWatcher")
                .await
                .unwrap();
            wait_for(&controller, TrayAvailability::Unavailable).await;
            watcher
                .request_name("org.kde.StatusNotifierWatcher")
                .await
                .unwrap();
            wait_for(&controller, TrayAvailability::Available).await;
            assert_eq!(registrations.lock().unwrap().len(), 2);
            {
                let registrations = registrations.lock().unwrap();
                assert_eq!(registrations[0], registrations[1]);
            }
            controller.service_finished();
            let started = Instant::now();
            drop(tray);
            assert!(started.elapsed() < Duration::from_secs(2));
        });
    }
    #[test]
    #[ignore = "requires private bus socket path EPOXY_TEST_BUS_SOCKET and EPOXY_PRIVATE_TEST_BUS=1"]
    fn session_bus_restart_recovers_without_restarting_epoxy() {
        assert_eq!(std::env::var("EPOXY_PRIVATE_TEST_BUS").as_deref(), Ok("1"));
        let path = std::env::var("EPOXY_TEST_BUS_SOCKET").unwrap();
        assert_eq!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
            format!("unix:path={path}")
        );
        struct Bus(std::process::Child);
        impl Drop for Bus {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let spawn = || {
            let child = std::process::Command::new("dbus-daemon")
                .args([
                    "--session",
                    "--nofork",
                    "--address",
                    &format!("unix:path={path}"),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            std::thread::sleep(Duration::from_millis(100));
            Bus(child)
        };
        let bus = spawn();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (diagnostics, _subscriber) =
                crate::diagnostics::subscriber("info", std::io::sink).unwrap();
            let controller = DesktopController::new(diagnostics);
            let (ui, _rx) = crate::ui::presentation_channel();
            let registrations = Arc::new(Mutex::new(Vec::new()));
            let watcher = zbus::connection::Builder::session()
                .unwrap()
                .name("org.kde.StatusNotifierWatcher")
                .unwrap()
                .serve_at(
                    "/StatusNotifierWatcher",
                    Watcher {
                        registrations: registrations.clone(),
                        stall: false,
                        host: Arc::new(AtomicBool::new(true)),
                        failures: Arc::new(AtomicUsize::new(0)),
                        attempts: Arc::new(AtomicUsize::new(0)),
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            let tray = start_tray(controller.clone(), ui).unwrap();
            wait_for(&controller, TrayAvailability::Available).await;
            drop(bus);
            drop(watcher);
            wait_for(&controller, TrayAvailability::Unavailable).await;
            let _bus = spawn();
            let _watcher = zbus::connection::Builder::session()
                .unwrap()
                .name("org.kde.StatusNotifierWatcher")
                .unwrap()
                .serve_at(
                    "/StatusNotifierWatcher",
                    Watcher {
                        registrations: registrations.clone(),
                        stall: false,
                        host: Arc::new(AtomicBool::new(true)),
                        failures: Arc::new(AtomicUsize::new(0)),
                        attempts: Arc::new(AtomicUsize::new(0)),
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            wait_for(&controller, TrayAvailability::Available).await;
            assert_eq!(registrations.lock().unwrap().len(), 2);
            controller.service_finished();
            drop(tray);
        });
    }
    #[test]
    #[ignore = "requires private dbus-run-session and EPOXY_PRIVATE_TEST_BUS=1"]
    fn nonresponding_registration_is_bounded_and_quit_cancels_retry() {
        assert_eq!(std::env::var("EPOXY_PRIVATE_TEST_BUS").as_deref(), Ok("1"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let registrations = Arc::new(Mutex::new(Vec::new()));
            let _watcher = zbus::connection::Builder::session()
                .unwrap()
                .name("org.kde.StatusNotifierWatcher")
                .unwrap()
                .serve_at(
                    "/StatusNotifierWatcher",
                    Watcher {
                        registrations: registrations.clone(),
                        stall: true,
                        host: Arc::new(AtomicBool::new(true)),
                        failures: Arc::new(AtomicUsize::new(0)),
                        attempts: Arc::new(AtomicUsize::new(0)),
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            let (diagnostics, _subscriber) =
                crate::diagnostics::subscriber("info", std::io::sink).unwrap();
            let controller = DesktopController::new(diagnostics);
            let (ui, _rx) = crate::ui::presentation_channel();
            let tray = start_tray(controller.clone(), ui).unwrap();
            wait_for(&controller, TrayAvailability::Unavailable).await;
            assert_eq!(registrations.lock().unwrap().len(), 1);
            controller.service_finished();
            let start = Instant::now();
            drop(tray);
            assert!(start.elapsed() < Duration::from_secs(2));
        });
    }
    #[test]
    #[ignore = "requires private dbus-run-session and EPOXY_PRIVATE_TEST_BUS=1"]
    fn watcher_without_host_recovers_when_host_changes_without_owner_change() {
        assert_eq!(std::env::var("EPOXY_PRIVATE_TEST_BUS").as_deref(), Ok("1"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let registrations = Arc::new(Mutex::new(Vec::new()));
            let host = Arc::new(AtomicBool::new(false));
            let _watcher = zbus::connection::Builder::session()
                .unwrap()
                .name("org.kde.StatusNotifierWatcher")
                .unwrap()
                .serve_at(
                    "/StatusNotifierWatcher",
                    Watcher {
                        registrations: registrations.clone(),
                        stall: false,
                        host: host.clone(),
                        failures: Arc::new(AtomicUsize::new(0)),
                        attempts: Arc::new(AtomicUsize::new(0)),
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            let (diagnostics, _subscriber) =
                crate::diagnostics::subscriber("info", std::io::sink).unwrap();
            let controller = DesktopController::new(diagnostics);
            let (ui, _rx) = crate::ui::presentation_channel();
            let tray = start_tray(controller.clone(), ui).unwrap();
            tokio::time::sleep(Duration::from_millis(2200)).await;
            assert_eq!(registrations.lock().unwrap().len(), 1);
            assert_eq!(
                controller.snapshot().tray,
                TrayAvailability::Unavailable,
                "watcher is not a display host"
            );
            host.store(true, Ordering::SeqCst);
            wait_for(&controller, TrayAvailability::Available).await;
            host.store(false, Ordering::SeqCst);
            wait_for(&controller, TrayAvailability::Unavailable).await;
            host.store(true, Ordering::SeqCst);
            wait_for(&controller, TrayAvailability::Available).await;
            assert_eq!(
                registrations.lock().unwrap().len(),
                1,
                "host changes must not recreate the tray"
            );
            controller.service_finished();
            drop(tray);
        });
    }
    #[test]
    #[ignore = "requires private dbus-run-session and EPOXY_PRIVATE_TEST_BUS=1"]
    fn late_watcher_transient_rejection_retries_without_owner_change() {
        assert_eq!(std::env::var("EPOXY_PRIVATE_TEST_BUS").as_deref(), Ok("1"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (diagnostics, _subscriber) =
                crate::diagnostics::subscriber("info", std::io::sink).unwrap();
            let controller = DesktopController::new(diagnostics);
            let (ui, _rx) = crate::ui::presentation_channel();
            let tray = start_tray(controller.clone(), ui).unwrap();
            wait_for(&controller, TrayAvailability::Unavailable).await;
            let registrations = Arc::new(Mutex::new(Vec::new()));
            let attempts = Arc::new(AtomicUsize::new(0));
            let watcher = zbus::connection::Builder::session()
                .unwrap()
                .name("org.kde.StatusNotifierWatcher")
                .unwrap()
                .serve_at(
                    "/StatusNotifierWatcher",
                    Watcher {
                        registrations: registrations.clone(),
                        stall: false,
                        host: Arc::new(AtomicBool::new(true)),
                        failures: Arc::new(AtomicUsize::new(1)),
                        attempts: attempts.clone(),
                    },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            wait_for(&controller, TrayAvailability::Available).await;
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
            assert_eq!(registrations.lock().unwrap().len(), 1);
            let daemon = zbus::fdo::DBusProxy::new(&watcher).await.unwrap();
            let prefix = format!("org.kde.StatusNotifierItem-{}-", std::process::id());
            let live = daemon
                .list_names()
                .await
                .unwrap()
                .into_iter()
                .filter(|name| name.as_str().starts_with(&prefix))
                .count();
            assert_eq!(live, 1, "failed attempt must release its previous tray");
            controller.service_finished();
            drop(tray);
        });
    }
}

pub struct TrayRuntime {
    stop: tokio_util::sync::CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for TrayRuntime {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}
pub fn start_tray(controller: DesktopController, ui: UiHandle) -> anyhow::Result<TrayRuntime> {
    let stop = tokio_util::sync::CancellationToken::new();
    let stop_thread = stop.clone();
    let thread = std::thread::Builder::new()
        .name("epoxy-tray".into())
        .spawn(move || {
            let mut backoff = 1;
            let mut last_error = None;
            while !stop_thread.is_cancelled() && !controller.service_stopped().is_cancelled() {
                // Each attempt owns its runtime: cancellation drops every D-Bus task/connection
                // before a retry, including a partially registered service.
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        controller.set_tray(TrayAvailability::Unavailable);
                        tracing::warn!(error_kind = "tray_runtime", "tray unavailable");
                        break;
                    }
                };
                let result = runtime.block_on(run_attempt(
                    controller.clone(),
                    ui.clone(),
                    stop_thread.clone(),
                ));
                drop(runtime);
                if let Err(kind) = result {
                    controller.set_tray(TrayAvailability::Unavailable);
                    if last_error != Some(kind) {
                        tracing::warn!(
                            error_kind = kind,
                            "tray unavailable; background recovery continues"
                        );
                        last_error = Some(kind);
                    }
                } else {
                    break;
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(backoff);
                while std::time::Instant::now() < deadline
                    && !stop_thread.is_cancelled()
                    && !controller.service_stopped().is_cancelled()
                {
                    std::thread::park_timeout(std::time::Duration::from_millis(100));
                }
                backoff = (backoff * 2).min(30);
            }
        })?;
    Ok(TrayRuntime {
        stop,
        thread: Some(thread),
    })
}
async fn run_attempt(
    controller: DesktopController,
    ui: UiHandle,
    stop: tokio_util::sync::CancellationToken,
) -> Result<(), &'static str> {
    use ksni::TrayMethods;
    use std::time::Duration;
    let completion = controller.service_stopped();
    controller.set_tray(TrayAvailability::Pending);
    // ksni currently swallows update errors after a bus disconnect. An independent
    // connection supplies a reliable close notification for restarting the attempt.
    let bus = tokio::select! {
        _=stop.cancelled()=>return Ok(()),_=completion.cancelled()=>return Ok(()),
        result=tokio::time::timeout(Duration::from_secs(5),zbus::Connection::session())=>
            match result {Ok(Ok(bus))=>bus,_=>return Err("tray_connection")}
    };
    let registered = Arc::new(AtomicBool::new(true));
    let spawn = EpoxyTray {
        registered: registered.clone(),
        controller: controller.clone(),
        ui,
    }
    .assume_sni_available(true)
    .spawn();
    let handle = tokio::select! {
        _=stop.cancelled()=>return Ok(()),_=completion.cancelled()=>return Ok(()),
        result=tokio::time::timeout(Duration::from_secs(5),spawn)=>match result {
            Ok(Ok(handle))=>handle,Ok(Err(_))=>return Err("tray_connection"),Err(_)=>return Err("tray_registration_timeout"),
        }
    };
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _=stop.cancelled()=>break,_=completion.cancelled()=>break,
            _=bus.closed()=>return Err("tray_bus_disconnected"),
            _=tick.tick()=> {
                if handle.is_closed() {return Err("tray_connection_closed");}
                // update waits behind watcher registration; only confirm after it finishes.
                let update=tokio::time::timeout(Duration::from_secs(5),handle.update(|_|()));
                let result=tokio::select! {_=stop.cancelled()=>break,_=completion.cancelled()=>break,_=bus.closed()=>return Err("tray_bus_disconnected"),result=update=>result};
                if !matches!(result,Ok(Some(()))) {return Err("tray_update_timeout");}
                if registered.load(Ordering::SeqCst) {
                    // Use an uncached Properties.Get: watcher ownership alone does not
                    // imply a display host, and host changes may omit property signals.
                    let probe=async {
                        let proxy=zbus::Proxy::new(&bus,"org.kde.StatusNotifierWatcher","/StatusNotifierWatcher","org.freedesktop.DBus.Properties").await?;
                        let value:zbus::zvariant::OwnedValue=proxy.call("Get",&("org.kde.StatusNotifierWatcher","IsStatusNotifierHostRegistered")).await?;
                        bool::try_from(value).map_err(zbus::Error::from)
                    };
                    let host=tokio::select! {
                        _=stop.cancelled()=>break,_=completion.cancelled()=>break,
                        _=bus.closed()=>return Err("tray_bus_disconnected"),
                        result=tokio::time::timeout(Duration::from_secs(5),probe)=>match result {Ok(Ok(host))=>host,_=>return Err("tray_host_probe")}
                    };
                    controller.set_tray(if host && registered.load(Ordering::SeqCst) {TrayAvailability::Available} else {TrayAvailability::Unavailable});
                } else {controller.set_tray(TrayAvailability::Unavailable);}
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_millis(500), handle.shutdown()).await;
    Ok(())
}
