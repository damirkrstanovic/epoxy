//! Observe synthetic allocations immediately before deallocation, never after free.
use epoxy::{
    nss::{PinInfo, PinPrompt},
    pin::RequestPin,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

struct ObserveFree;
static TARGET: AtomicUsize = AtomicUsize::new(0);
static WIPED: AtomicBool = AtomicBool::new(false);
static SERIAL: Mutex<()> = Mutex::new(());
unsafe impl GlobalAlloc for ObserveFree {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TARGET
            .compare_exchange(ptr as usize, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            // The allocation is still valid here. Never inspect freed memory.
            let bytes = unsafe { std::slice::from_raw_parts(ptr, layout.size()) };
            WIPED.store(bytes.iter().all(|byte| *byte == 0), Ordering::SeqCst);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: ObserveFree = ObserveFree;
fn watch(text: &str) {
    WIPED.store(false, Ordering::SeqCst);
    TARGET.store(text.as_ptr() as usize, Ordering::SeqCst);
}
fn assert_wiped() {
    assert_eq!(
        TARGET.load(Ordering::SeqCst),
        0,
        "secret allocation was not released"
    );
    assert!(
        WIPED.load(Ordering::SeqCst),
        "secret allocation was not zeroed before free"
    );
}
struct NoNative;
impl PinPrompt for NoNative {
    fn prompt_pin(&self, _: &PinInfo, _: String) -> Option<epoxy::pin::SecretPin> {
        panic!("no native prompt expected")
    }
}
fn info() -> PinInfo {
    PinInfo {
        cert: None,
        reason: "synthetic".into(),
        reported_site: None,
    }
}

#[test]
fn unused_and_rejected_page_pins_are_wiped() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for secret in [
        "synthetic-secret".to_string(),
        "s".repeat(1025),
        "synthetic\0secret".to_string(),
    ] {
        watch(&secret);
        drop(RequestPin::try_from(secret));
        assert_wiped();
    }
}
#[test]
fn used_and_unused_request_credentials_are_wiped() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for used in [false, true] {
        let text = "synthetic-secret".to_string();
        watch(&text);
        let prompt = RequestPin::try_from(text)
            .unwrap()
            .into_prompt(&NoNative, CancellationToken::new());
        if used {
            drop(prompt.prompt_pin(&info(), "synthetic-token".into()));
        }
        drop(prompt);
        assert_wiped();
    }
}
#[test]
fn cancellation_clears_queued_pin_before_prompt_is_dropped() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let text = "synthetic-secret".to_string();
    watch(&text);
    let shutdown = CancellationToken::new();
    let prompt = RequestPin::try_from(text)
        .unwrap()
        .into_prompt(&NoNative, shutdown.clone());
    shutdown.cancel();
    assert!(
        prompt
            .prompt_pin(&info(), "synthetic-token".into())
            .is_none()
    );
    // Check before dropping the prompt so cancellation cannot retain the secret.
    assert_wiped();
}

#[cfg(feature = "ui-gtk")]
#[test]
fn closed_or_abandoned_reply_channels_wipe_native_pin() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for receiver_closed in [false, true] {
        let text = "synthetic-native-secret".to_string();
        watch(&text);
        let pin = epoxy::pin::SecretPin::from(text);
        let (tx, rx) = oneshot::channel();
        if receiver_closed {
            drop(rx);
            drop(tx.send(Some(pin)));
        } else {
            tx.send(Some(pin)).unwrap();
            drop(rx);
        }
        assert_wiped();
    }
}

#[test]
fn shutdown_during_native_reply_wipes_returned_pin() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    struct Native {
        pin: std::cell::RefCell<Option<epoxy::pin::SecretPin>>,
        shutdown: CancellationToken,
    }
    impl PinPrompt for Native {
        fn prompt_pin(&self, _: &PinInfo, _: String) -> Option<epoxy::pin::SecretPin> {
            self.shutdown.cancel();
            self.pin.borrow_mut().take()
        }
    }
    let text = "synthetic-native-secret".to_string();
    watch(&text);
    let shutdown = CancellationToken::new();
    let native = Native {
        pin: std::cell::RefCell::new(Some(text.into())),
        shutdown: shutdown.clone(),
    };
    let prompt = RequestPin::default().into_prompt(&native, shutdown);
    assert!(
        prompt
            .prompt_pin(&info(), "synthetic-token".into())
            .is_none()
    );
    assert_wiped();
}
