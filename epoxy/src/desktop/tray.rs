use super::*;
use crate::ui::UiHandle;
use ksni::{MenuItem, Tray, menu::StandardItem};
struct EpoxyTray {
    controller: DesktopController,
    ui: UiHandle,
}
impl Tray for EpoxyTray {
    fn id(&self) -> String {
        "dev.semyon.epoxy".into()
    }
    fn title(&self) -> String {
        "Epoxy".into()
    }
    fn icon_name(&self) -> String {
        "security-high-symbolic".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // Embedded monochrome smart-card fallback, ARGB network byte order.
        let mut data = vec![0; 24 * 24 * 4];
        for y in 4..20 {
            for x in 2..22 {
                if x == 2
                    || x == 21
                    || y == 4
                    || y == 19
                    || ((6..12).contains(&x) && (8..15).contains(&y))
                {
                    data[(y * 24 + x) * 4..(y * 24 + x + 1) * 4]
                        .copy_from_slice(&[255, 112, 112, 112]);
                }
            }
        }
        vec![ksni::Icon {
            width: 24,
            height: 24,
            data,
        }]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Epoxy".into(),
            description: status_label(self.controller.snapshot().activity).into(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.ui.show_window();
    }
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.controller.set_tray(TrayAvailability::Unavailable);
        true
    }
    fn watcher_online(&self) {
        self.controller.set_tray(TrayAvailability::Pending);
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let status = self.controller.diagnostics().status().ok();
        vec![
            StandardItem {
                label: status_label(self.controller.snapshot().activity).into(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open Epoxy".into(),
                activate: Box::new(|tray: &mut Self| tray.ui.show_window()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Enable masked diagnostics for 5 minutes".into(),
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
                label: "Disable diagnostics".into(),
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
            MenuItem::Separator,
            StandardItem {
                label: "Quit Epoxy".into(),
                activate: Box::new(|tray: &mut Self| tray.controller.request_quit()),
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
    fn tray_actions_control_existing_diagnostics_and_presentation() {
        let (diagnostics, _subscriber) =
            crate::diagnostics::subscriber("warn", std::io::sink).unwrap();
        let controller = DesktopController::new(diagnostics);
        let (ui, mut rx) = crate::ui::presentation_channel();
        let mut tray = EpoxyTray {
            controller: controller.clone(),
            ui,
        };
        let menu = tray.menu();
        let activate = |menu: &Vec<MenuItem<EpoxyTray>>, index: usize, tray: &mut EpoxyTray| {
            if let MenuItem::Standard(item) = &menu[index] {
                (item.activate)(tray);
            } else {
                panic!("expected action");
            }
        };
        activate(&menu, 1, &mut tray);
        assert_eq!(rx.try_recv().unwrap(), ());
        activate(&menu, 3, &mut tray);
        let status = controller.diagnostics().status().unwrap();
        assert!(status.requests_masked);
        assert!(status.expires_in_seconds.unwrap() <= 300);
        activate(&menu, 4, &mut tray);
        assert_eq!(controller.diagnostics().status().unwrap().filter, "warn");
        controller
            .diagnostics()
            .enable(crate::diagnostics::Level::Debug, false, 1)
            .unwrap();
        let menu = tray.menu();
        if let MenuItem::Standard(item) = &menu[4] {
            assert!(item.enabled);
        }
        controller
            .diagnostics()
            .expire(std::time::Instant::now() + std::time::Duration::from_secs(2))
            .unwrap();
        let menu = tray.menu();
        if let MenuItem::Standard(item) = &menu[4] {
            assert!(!item.enabled);
        }
        activate(&menu, 6, &mut tray);
        assert!(controller.shutdown_requested().is_cancelled());
    }
}

#[cfg(test)]
mod bus_tests {
    use super::*;
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    struct Watcher {
        registrations: Arc<Mutex<Vec<String>>>,
        stall: bool,
    }
    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        async fn register_status_notifier_item(&self, service: &str) {
            self.registrations.lock().unwrap().push(service.to_owned());
            if self.stall {
                std::future::pending::<()>().await;
            }
        }
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
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
            let (ui, _rx) = crate::ui::presentation_channel();
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
            item.call::<_, _, ()>("Activate", &(0i32, 0i32))
                .await
                .unwrap();
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
    let spawn = EpoxyTray {
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
    controller.confirm_tray();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _=stop.cancelled()=>break,_=completion.cancelled()=>break,
            _=tick.tick()=> {
                if handle.is_closed() {return Err("tray_connection_closed");}
                // update waits behind watcher registration; only confirm after it finishes.
                let update=tokio::time::timeout(Duration::from_secs(5),handle.update(|_|()));
                let result=tokio::select! {_=stop.cancelled()=>break,_=completion.cancelled()=>break,_=bus.closed()=>return Err("tray_bus_disconnected"),result=update=>result};
                if !matches!(result,Ok(Some(()))) {return Err("tray_update_timeout");}
                controller.confirm_tray();
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_millis(500), handle.shutdown()).await;
    Ok(())
}
