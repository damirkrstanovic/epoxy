use crate::diagnostics::Diagnostics;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Starting,
    Ready,
    Working,
    WaitingForPin,
    Stopping,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    Cancelled,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Information,
    Enumeration,
    Signing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAvailability {
    Pending,
    Available,
    Unavailable,
}
#[derive(Clone, Debug)]
pub struct DesktopSnapshot {
    pub endpoint: Option<SocketAddr>,
    pub connections: usize,
    pub activity: Activity,
    pub last_result: Option<Outcome>,
    pub tray: TrayAvailability,
}
struct State {
    endpoint: Option<SocketAddr>,
    connections: usize,
    next_operation: u64,
    operations: BTreeMap<u64, bool>,
    prompts: usize,
    last_result: Option<Outcome>,
    tray: TrayAvailability,
}
#[derive(Clone)]
pub struct DesktopController {
    state: Arc<Mutex<State>>,
    diagnostics: Diagnostics,
    shutdown: CancellationToken,
    stopped: CancellationToken,
}
impl DesktopController {
    pub fn new(diagnostics: Diagnostics) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                endpoint: None,
                connections: 0,
                next_operation: 0,
                operations: BTreeMap::new(),
                prompts: 0,
                last_result: None,
                tray: TrayAvailability::Pending,
            })),
            diagnostics,
            shutdown: CancellationToken::new(),
            stopped: CancellationToken::new(),
        }
    }
    pub fn set_ready(&self, address: SocketAddr) {
        self.state.lock().unwrap().endpoint = Some(address);
    }
    pub fn snapshot(&self) -> DesktopSnapshot {
        let state = self.state.lock().unwrap();
        DesktopSnapshot {
            endpoint: state.endpoint,
            connections: state.connections,
            activity: if self.shutdown.is_cancelled() {
                Activity::Stopping
            } else if state.prompts > 0 {
                Activity::WaitingForPin
            } else if !state.operations.is_empty() {
                Activity::Working
            } else if state.endpoint.is_some() {
                Activity::Ready
            } else {
                Activity::Starting
            },
            last_result: state.last_result,
            tray: state.tray,
        }
    }
    pub fn connection_started(&self) -> ConnectionGuard {
        self.state.lock().unwrap().connections += 1;
        ConnectionGuard(self.clone())
    }
    pub fn operation_started(&self, _id: u64, _kind: OperationKind) -> OperationGuard {
        let mut state = self.state.lock().unwrap();
        state.next_operation += 1;
        let key = state.next_operation;
        state.operations.insert(key, false);
        OperationGuard {
            controller: self.clone(),
            key,
            finished: false,
        }
    }
    pub fn pin_requested(&self) -> PromptGuard {
        let mut state = self.state.lock().unwrap();
        state.prompts += 1;
        let key = state.operations.last_key_value().map(|(key, _)| *key);
        PromptGuard {
            controller: self.clone(),
            operation: key,
        }
    }
    pub fn request_quit(&self) {
        self.shutdown.cancel();
    }
    pub fn shutdown_requested(&self) -> CancellationToken {
        self.shutdown.clone()
    }
    pub fn service_stopped(&self) -> CancellationToken {
        self.stopped.clone()
    }
    pub fn service_finished(&self) {
        self.stopped.cancel();
    }
    pub fn service_lifetime(&self) -> ServiceLifetime {
        ServiceLifetime(self.clone())
    }
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
    pub fn set_tray(&self, availability: TrayAvailability) {
        let changed = {
            let mut state = self.state.lock().unwrap();
            let changed = state.tray != availability;
            state.tray = availability;
            changed
        };
        if changed {
            tracing::debug!(availability=?availability,"tray availability changed");
        }
    }
    #[cfg(feature = "tray")]
    pub(crate) fn confirm_tray(&self) {
        let mut state = self.state.lock().unwrap();
        if state.tray == TrayAvailability::Pending {
            state.tray = TrayAvailability::Available;
        }
    }
}
pub struct ConnectionGuard(DesktopController);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().connections -= 1;
    }
}
pub struct OperationGuard {
    controller: DesktopController,
    key: u64,
    finished: bool,
}
impl OperationGuard {
    pub fn complete(mut self, status: u32) {
        self.finish(status == 0);
    }
    fn finish(&mut self, success: bool) {
        if self.finished {
            return;
        }
        let mut state = self.controller.state.lock().unwrap();
        let cancelled = state.operations.remove(&self.key).unwrap_or(false);
        state.last_result = Some(if cancelled || self.controller.shutdown.is_cancelled() {
            Outcome::Cancelled
        } else if success {
            Outcome::Succeeded
        } else {
            Outcome::Failed
        });
        self.finished = true;
    }
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.finish(false);
    }
}
pub struct PromptGuard {
    controller: DesktopController,
    operation: Option<u64>,
}
impl PromptGuard {
    pub fn cancel(self) {
        let mut state = self.controller.state.lock().unwrap();
        if let Some(key) = self.operation
            && let Some(cancelled) = state.operations.get_mut(&key)
        {
            *cancelled = true;
        }
    }
}
impl Drop for PromptGuard {
    fn drop(&mut self) {
        self.controller.state.lock().unwrap().prompts -= 1;
    }
}
pub struct ServiceLifetime(DesktopController);
impl Drop for ServiceLifetime {
    fn drop(&mut self) {
        self.0.request_quit();
        self.0.service_finished();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn controller() -> (DesktopController, impl tracing::Subscriber + Send + Sync) {
        let (diagnostics, subscriber) =
            crate::diagnostics::subscriber("info", std::io::sink).unwrap();
        (DesktopController::new(diagnostics), subscriber)
    }
    #[test]
    fn pin_wait_and_shutdown_take_priority_and_guards_release() {
        let (controller, _subscriber) = controller();
        controller.set_ready("127.0.0.1:17165".parse().unwrap());
        let connection = controller.connection_started();
        let operation = controller.operation_started(7, OperationKind::Signing);
        let prompt = controller.pin_requested();
        assert_eq!(controller.snapshot().activity, Activity::WaitingForPin);
        controller.request_quit();
        controller.request_quit();
        assert_eq!(controller.snapshot().activity, Activity::Stopping);
        drop(prompt);
        drop(operation);
        drop(connection);
        assert_eq!(controller.snapshot().connections, 0);
        assert_eq!(controller.snapshot().activity, Activity::Stopping);
        assert!(controller.shutdown_requested().is_cancelled());
    }
    #[test]
    fn cancellation_is_not_overwritten_by_authentication_failure() {
        let (controller, _subscriber) = controller();
        controller.set_ready("127.0.0.1:17165".parse().unwrap());
        let operation = controller.operation_started(1, OperationKind::Signing);
        controller.pin_requested().cancel();
        operation.complete(1500);
        assert_eq!(controller.snapshot().last_result, Some(Outcome::Cancelled));
        controller
            .operation_started(2, OperationKind::Information)
            .complete(0);
        assert_eq!(controller.snapshot().last_result, Some(Outcome::Succeeded));
        assert_eq!(controller.snapshot().activity, Activity::Ready);
        let operation = controller.operation_started(3, OperationKind::Signing);
        drop(operation);
        assert_eq!(controller.snapshot().last_result, Some(Outcome::Failed));
    }
    #[test]
    fn concurrent_connections_and_unrelated_completion_preserve_pending_pin() {
        let (controller, _subscriber) = controller();
        controller.set_ready("127.0.0.1:17165".parse().unwrap());
        let signing = controller.operation_started(1, OperationKind::Signing);
        let prompt = controller.pin_requested();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let controller = controller.clone();
                scope.spawn(move || {
                    for _ in 0..100 {
                        let connection = controller.connection_started();
                        controller
                            .operation_started(2, OperationKind::Information)
                            .complete(0);
                        assert_eq!(controller.snapshot().activity, Activity::WaitingForPin);
                        drop(connection);
                    }
                });
            }
        });
        prompt.cancel();
        signing.complete(1500);
        assert_eq!(controller.snapshot().last_result, Some(Outcome::Cancelled));
        assert_eq!(controller.snapshot().connections, 0);
    }
    #[test]
    fn shutdown_wins_over_late_success_and_lifetime_drop_signals_completion() {
        let (controller, _subscriber) = controller();
        let lifetime = controller.service_lifetime();
        let operation = controller.operation_started(1, OperationKind::Signing);
        controller.request_quit();
        operation.complete(0);
        assert_eq!(controller.snapshot().last_result, Some(Outcome::Cancelled));
        drop(lifetime);
        assert!(controller.service_stopped().is_cancelled());
    }
}
