use super::*;
use crate::nss::{Nss, PinContext, PinInfo, PinPrompt};
struct NoPin;
impl PinPrompt for NoPin {
    fn prompt_pin(&self, _: &PinInfo, _: String) -> Option<String> {
        None
    }
}
pub fn public_catalog(
    nss: &Nss,
    allow_soft_tokens: bool,
) -> Result<Vec<PublicContext>, SelectionError> {
    let tokens = nss.get_tokens().ok_or(SelectionError::Unavailable)?;
    let no_pin = NoPin;
    let context = PinContext::new(
        &no_pin,
        PinInfo {
            cert: None,
            reason: "Read public certificates".into(),
        },
    );
    let mut result = vec![];
    for token in tokens
        .iter()
        .filter(|t| (allow_soft_tokens || t.is_hw()) && !t.is_internal())
    {
        if result.len() >= 64 {
            return Err(SelectionError::Unavailable);
        }
        let key = token.context_key()?;
        let mut entry = PublicContext {
            key: key.clone(),
            label: token.name(),
            requires_authentication: false,
            certificates: vec![],
        };
        match nss.get_certs_in_token(&context, &token) {
            Err(_) => entry.requires_authentication = true,
            Ok(Some(certs)) => {
                for cert in certs.iter() {
                    if entry.certificates.len() >= 128 {
                        return Err(SelectionError::Unavailable);
                    }
                    let Ok(der) = cert.der() else {
                        continue;
                    };
                    if let Ok(metadata) = PublicCertificate::from_der(key.clone(), der.as_ref()) {
                        entry.certificates.push(metadata);
                    }
                }
            }
            Ok(None) => (),
        }
        result.push(entry);
    }
    Ok(result)
}
