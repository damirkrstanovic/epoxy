use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tracing::Subscriber;
use tracing_subscriber::{EnvFilter, Registry, fmt::MakeWriter, layer::SubscriberExt, reload};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}
impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[derive(Clone)]
struct Policy {
    baseline: String,
    active: String,
    requests: bool,
    deadline: Option<Instant>,
}
impl Policy {
    fn new(baseline: String) -> Self {
        Self {
            active: baseline.clone(),
            baseline,
            requests: false,
            deadline: None,
        }
    }
    fn enable_at(&mut self, level: Level, requests: bool, ttl: u64, now: Instant) -> Result<()> {
        ensure!(
            (1..=3600).contains(&ttl),
            "diagnostic lifetime must be between 1 and 3600 seconds"
        );
        self.active = if requests {
            format!("{},epoxy::diagnostics=info", level.as_str())
        } else {
            level.as_str().into()
        };
        self.requests = requests;
        self.deadline = Some(now + Duration::from_secs(ttl));
        Ok(())
    }
    fn disable(&mut self) {
        self.active.clone_from(&self.baseline);
        self.requests = false;
        self.deadline = None;
    }
    fn expire(&mut self, now: Instant) {
        if self.deadline.is_some_and(|d| now >= d) {
            self.disable();
        }
    }
    fn status_at(&self, now: Instant) -> DiagnosticsStatus {
        DiagnosticsStatus {
            filter: self.active.clone(),
            requests_masked: self.requests && self.deadline.is_some_and(|d| now < d),
            expires_in_seconds: self
                .deadline
                .map(|d| d.saturating_duration_since(now).as_secs()),
        }
    }
    fn active_filter(&self) -> &str {
        &self.active
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticsStatus {
    pub filter: String,
    pub requests_masked: bool,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Clone)]
pub struct Diagnostics {
    policy: Arc<Mutex<Policy>>,
    filter: reload::Handle<EnvFilter, Registry>,
}

pub fn subscriber<W>(
    log_spec: &str,
    writer: W,
) -> Result<(Diagnostics, impl Subscriber + Send + Sync)>
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let filter = EnvFilter::try_new(log_spec).context("invalid log filter")?;
    let (layer, handle) = reload::Layer::new(filter);
    let subscriber = Registry::default().with(layer).with(
        tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false),
    );
    let diagnostics = Diagnostics {
        policy: Arc::new(Mutex::new(Policy::new(log_spec.into()))),
        filter: handle,
    };
    Ok((diagnostics, subscriber))
}

impl Diagnostics {
    fn change(&self, update: impl FnOnce(&mut Policy) -> Result<()>) -> Result<()> {
        let mut policy = self
            .policy
            .lock()
            .map_err(|_| anyhow::anyhow!("diagnostics state unavailable"))?;
        let mut next = policy.clone();
        update(&mut next)?;
        if next.active_filter() != policy.active_filter() {
            self.filter
                .reload(EnvFilter::try_new(next.active_filter())?)
                .context("diagnostics subscriber unavailable")?;
        }
        *policy = next;
        Ok(())
    }
    pub fn enable(&self, level: Level, requests: bool, ttl: u64) -> Result<()> {
        self.change(|policy| policy.enable_at(level, requests, ttl, Instant::now()))
    }
    pub fn disable(&self) -> Result<()> {
        self.change(|policy| {
            policy.disable();
            Ok(())
        })
    }
    pub fn expire(&self, now: Instant) -> Result<()> {
        self.change(|policy| {
            policy.expire(now);
            Ok(())
        })
    }
    pub fn status(&self) -> Result<DiagnosticsStatus> {
        self.expire(Instant::now())?;
        let policy = self
            .policy
            .lock()
            .map_err(|_| anyhow::anyhow!("diagnostics state unavailable"))?;
        Ok(policy.status_at(Instant::now()))
    }
    pub fn wire(&self, direction: &'static str, text: &str, correlation_id: u64) {
        if !self.status().is_ok_and(|s| s.requests_masked) {
            return;
        }
        let masked = super::mask::masked_wire(text);
        // Recheck after sanitization; disabling never waits for payload processing.
        if self.status().is_ok_and(|s| s.requests_masked) {
            tracing::info!(target: "epoxy::diagnostics", direction, correlation_id, wire = %masked, "masked protocol trace");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expiry_restores_baseline_and_payloads_off() {
        let start = Instant::now();
        let mut policy = Policy::new("warn".into());
        policy.enable_at(Level::Debug, true, 300, start).unwrap();
        assert!(policy.status_at(start).requests_masked);
        policy.expire(start + Duration::from_secs(300));
        assert!(
            !policy
                .status_at(start + Duration::from_secs(300))
                .requests_masked
        );
        assert_eq!(policy.active_filter(), "warn");
    }
    #[test]
    fn invalid_lifetimes_are_rejected_without_state_change() {
        let mut policy = Policy::new("info".into());
        for ttl in [0, 3601, u64::MAX] {
            assert!(
                policy
                    .enable_at(Level::Debug, true, ttl, Instant::now())
                    .is_err()
            );
        }
        assert_eq!(policy.active_filter(), "info");
    }
    #[test]
    fn repeated_enable_replaces_deadline() {
        let start = Instant::now();
        let mut policy = Policy::new("warn".into());
        policy.enable_at(Level::Debug, true, 1, start).unwrap();
        policy.enable_at(Level::Trace, true, 10, start).unwrap();
        policy.expire(start + Duration::from_secs(2));
        assert!(
            policy
                .status_at(start + Duration::from_secs(2))
                .requests_masked
        );
        policy.expire(start + Duration::from_secs(10));
        assert_eq!(policy.active_filter(), "warn");
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Self;
        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }
    #[test]
    fn runtime_changes_affect_actual_output_without_exposing_pin() {
        let capture = Capture::default();
        let (controller, subscriber) = subscriber("trace", capture.clone()).unwrap();
        tracing::subscriber::with_default(subscriber, || {
            let input = r#"{"operation":"GET_CERTIFICATES","input":{"pin":"synthetic-secret","terminalId":1}}"#;
            controller.wire("request", input, 1);
            assert!(capture.0.lock().unwrap().is_empty());
            controller.enable(Level::Error, true, 300).unwrap();
            controller.wire("request", input, 2);
            let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
            assert!(output.contains("GET_CERTIFICATES"));
            assert!(!output.contains("synthetic-secret"));
            controller.disable().unwrap();
            let length = capture.0.lock().unwrap().len();
            controller.wire("request", input, 3);
            assert_eq!(capture.0.lock().unwrap().len(), length);
            assert_eq!(controller.status().unwrap().filter, "trace");
        });
    }
    #[test]
    fn concurrent_reads_and_invalid_enable_keep_consistent_state() {
        let (controller, _subscriber) = subscriber("warn", std::io::sink).unwrap();
        controller.enable(Level::Debug, true, 300).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let controller = controller.clone();
                scope.spawn(move || {
                    for _ in 0..100 {
                        assert!(controller.enable(Level::Trace, true, 0).is_err());
                        assert!(controller.status().unwrap().requests_masked);
                    }
                });
            }
        });
        controller.disable().unwrap();
        assert!(!controller.status().unwrap().requests_masked);
    }
}
