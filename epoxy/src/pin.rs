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
        if let Some(cert) = &pin_info.cert {
            print!("{}: ", cert);
        }
        println!("{}", pin_info.reason);
        let prompt = format!("Enter PIN for {}: ", token_name);
        rpassword::prompt_password(prompt).ok()
    }
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
