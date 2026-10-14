//! Open PIN dialog with mock data

use anyhow::Error;
use epoxy::{nss::PinInfo, ui};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt::init();

    let pin_method_ct = CancellationToken::new();
    let (pin_prompt, pin_method_fut) = ui::ui_pin_method(pin_method_ct.clone());

    let pin_info = PinInfo {
        cert: Some("John Doe 012345678 Sign".into()),
        reason: "Sign form (unidentified)".into(),
        reported_site: Some("example.invalid".into()),
    };
    let token_name = "My Token".into();

    let pin = pin_prompt.prompt_pin(&pin_info, token_name);

    info!(pin_received = pin.is_some(), "PIN prompt finished");
    drop(pin);

    pin_method_ct.cancel();
    pin_method_fut.await;

    Ok(())
}
