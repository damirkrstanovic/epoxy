use futures::{
    FutureExt,
    future::{self, LocalBoxFuture},
};
use tokio_util::sync::CancellationToken;

use crate::nss::{PinInfo, PinPrompt};

/// Request-scoped credential. Debug output must never reveal its contents.
#[derive(Default, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct RequestPin(String);
impl TryFrom<String> for RequestPin {
    type Error = &'static str;
    fn try_from(pin: String) -> Result<Self, Self::Error> {
        if pin.len() > 1024 {
            return Err("PIN exceeds request limit");
        }
        if pin.contains('\0') {
            return Err("invalid PIN encoding");
        }
        Ok(Self(pin))
    }
}
impl std::fmt::Debug for RequestPin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RequestPin([REDACTED])")
    }
}
impl RequestPin {
    pub fn into_prompt(
        self,
        fallback: &dyn PinPrompt,
        shutdown: CancellationToken,
    ) -> RequestPinPrompt<'_> {
        RequestPinPrompt {
            supplied: std::cell::RefCell::new((!self.0.is_empty()).then_some(self.0)),
            attempted: std::cell::Cell::new(false),
            fallback,
            shutdown,
        }
    }
}
pub struct RequestPinPrompt<'a> {
    supplied: std::cell::RefCell<Option<String>>,
    attempted: std::cell::Cell<bool>,
    fallback: &'a dyn PinPrompt,
    shutdown: CancellationToken,
}
impl PinPrompt for RequestPinPrompt<'_> {
    fn prompt_pin(&self, info: &PinInfo, token: String) -> Option<String> {
        if self.attempted.replace(true) || self.shutdown.is_cancelled() {
            return None;
        }
        let pin = match self.supplied.borrow_mut().take() {
            Some(pin) => Some(pin),
            None => self.fallback.prompt_pin(info, token),
        };
        if self.shutdown.is_cancelled() {
            None
        } else {
            pin
        }
    }
}

pub type PinMethod = (Box<dyn PinPrompt>, LocalBoxFuture<'static, ()>);

pub fn tty_pin_method(_: CancellationToken) -> PinMethod {
    (Box::new(TtyPinPrompt), future::pending().boxed_local())
}

pub struct TtyPinPrompt;

impl PinPrompt for TtyPinPrompt {
    fn prompt_pin(&self, pin_info: &PinInfo, token_name: String) -> Option<String> {
        rpassword::prompt_password(tty_prompt_text(pin_info, &token_name)).ok()
    }
}

fn tty_prompt_text(info: &PinInfo, token: &str) -> String {
    use crate::certificates::display_text;
    let cert = info
        .cert
        .as_ref()
        .map(|cert| format!("{}: ", display_text(cert, 160)))
        .unwrap_or_default();
    format!(
        "{cert}{}\nEnter PIN for {}: ",
        display_text(&info.reason, 160),
        display_text(token, 160)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Native(Cell<usize>);
    impl PinPrompt for Native {
        fn prompt_pin(&self, _: &PinInfo, _: String) -> Option<String> {
            self.0.set(self.0.get() + 1);
            Some("native-secret".into())
        }
    }
    #[test]
    fn tty_prompt_does_not_interpret_untrusted_terminal_controls() {
        let text = tty_prompt_text(
            &PinInfo {
                cert: Some("Signer\x1b[2J".into()),
                reason: "Tax\r\nforged prompt\x07".into(),
                reported_site: None,
            },
            "Token\x1b]52;c;secret\x07",
        );
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\r'));
        assert!(!text.contains('\x07'));
        assert_eq!(text.matches('\n').count(), 1);
        assert!(text.contains("Signer"));
    }
    #[test]
    fn oversized_page_pin_is_rejected_without_echo() {
        let error = RequestPin::try_from("s".repeat(1025)).unwrap_err();
        assert!(!error.contains("ssss"));
        assert!(RequestPin::try_from("s".repeat(1024)).is_ok());
    }
    #[test]
    fn embedded_null_pin_is_rejected_without_echoing_credentials() {
        let error = serde_json::from_str::<RequestPin>(r#""page-secret\u0000""#).unwrap_err();
        assert!(!error.to_string().contains("page-secret"));
    }
    #[test]
    fn shutdown_rejects_supplied_credentials_without_native_fallback() {
        let native = Native(Cell::new(0));
        let shutdown = CancellationToken::new();
        let pin: RequestPin = serde_json::from_str("\"page-secret\"").unwrap();
        let prompt = pin.into_prompt(&native, shutdown.clone());
        shutdown.cancel();
        assert_eq!(
            prompt.prompt_pin(
                &PinInfo {
                    cert: None,
                    reason: "Synthetic".into(),
                    reported_site: None,
                },
                "token".into()
            ),
            None
        );
        assert_eq!(native.0.get(), 0);
    }
    #[test]
    fn supplied_pin_is_used_once_without_native_fallback_or_cross_request_reuse() {
        let native = Native(Cell::new(0));
        let info = PinInfo {
            cert: None,
            reason: "Synthetic signing".into(),
            reported_site: None,
        };
        let pin: RequestPin = serde_json::from_str("\" page-secret \"").unwrap();
        assert!(!format!("{pin:?}").contains("page-secret"));
        let prompt = pin.into_prompt(&native, CancellationToken::new());
        assert_eq!(
            prompt.prompt_pin(&info, "token".into()),
            Some(" page-secret ".into())
        );
        assert_eq!(prompt.prompt_pin(&info, "token".into()), None);
        assert_eq!(native.0.get(), 0);
        let next = RequestPin::default().into_prompt(&native, CancellationToken::new());
        assert_eq!(
            next.prompt_pin(&info, "token".into()),
            Some("native-secret".into())
        );
        assert_eq!(next.prompt_pin(&info, "token".into()), None);
        assert_eq!(native.0.get(), 1);
    }
}
