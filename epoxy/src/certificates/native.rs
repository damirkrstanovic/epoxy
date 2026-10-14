use super::*;
use crate::nss::{Nss, Token};
trait PublicTokenSource {
    fn context(&self) -> Result<ContextKey, SelectionError>;
    fn label(&self) -> String;
    fn needs_login(&self) -> bool;
    fn public_der(&self) -> Result<Vec<Vec<u8>>, SelectionError>;
}
struct NativeSource<'a> {
    nss: &'a Nss,
    token: &'a Token<'a>,
}
impl PublicTokenSource for NativeSource<'_> {
    fn context(&self) -> Result<ContextKey, SelectionError> {
        self.token.context_key()
    }
    fn label(&self) -> String {
        self.token.name()
    }
    fn needs_login(&self) -> bool {
        self.token.needs_login()
    }
    fn public_der(&self) -> Result<Vec<Vec<u8>>, SelectionError> {
        // PK11_ListCertsInSlot traverses visible objects without PK11_Authenticate.
        // IsFriendly is an NSS configuration hint, not the object's CKA_PRIVATE.
        let Some(certs) = self.nss.list_certificates(self.token) else {
            return Err(SelectionError::Unavailable);
        };
        let mut result = Vec::new();
        for cert in certs.iter() {
            if result.len() >= 128 {
                return Err(SelectionError::Unavailable);
            }
            result.push(cert.der()?.as_ref().to_vec());
        }
        Ok(result)
    }
}
fn read_public_context(source: &impl PublicTokenSource) -> Result<PublicContext, SelectionError> {
    let key = source.context()?;
    let mut context = PublicContext {
        key: key.clone(),
        label: source.label(),
        requires_authentication: false,
        certificates: vec![],
    };
    let certificates = match source.public_der() {
        Ok(v) => v,
        Err(_) if source.needs_login() => {
            context.requires_authentication = true;
            return Ok(context);
        }
        Err(error) => return Err(error),
    };
    context.requires_authentication = certificates.is_empty() && source.needs_login();
    for der in certificates {
        if let Ok(cert) = PublicCertificate::from_der(key.clone(), &der) {
            context.certificates.push(cert);
        }
    }
    Ok(context)
}
pub fn public_catalog(
    nss: &Nss,
    allow_soft_tokens: bool,
) -> Result<Vec<PublicContext>, SelectionError> {
    let tokens = nss.get_tokens().ok_or(SelectionError::Unavailable)?;
    let mut result = vec![];
    for token in tokens
        .iter()
        .filter(|t| (allow_soft_tokens || t.is_hw()) && !t.is_internal())
    {
        if result.len() >= 64 {
            return Err(SelectionError::Unavailable);
        }
        result.push(read_public_context(&NativeSource { nss, token: &token })?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_nonfriendly_token_can_expose_public_certificates_without_login() {
        struct Nonfriendly {
            private_only: bool,
        }
        impl PublicTokenSource for Nonfriendly {
            fn context(&self) -> Result<ContextKey, SelectionError> {
                Ok(ContextKey {
                    module_name: "Synthetic nonfriendly".into(),
                    module_library: None,
                    token_uri: b"pkcs11:token=synthetic".to_vec(),
                })
            }
            fn label(&self) -> String {
                "Synthetic PIN context".into()
            }
            fn needs_login(&self) -> bool {
                true
            }
            fn public_der(&self) -> Result<Vec<Vec<u8>>, SelectionError> {
                Ok(if self.private_only {
                    vec![]
                } else {
                    vec![include_bytes!("../../tests/fixtures/eligible.der").to_vec()]
                })
            }
        }
        let readable = read_public_context(&Nonfriendly {
            private_only: false,
        })
        .unwrap();
        assert_eq!(readable.certificates.len(), 1);
        assert!(!readable.requires_authentication);
        let private = read_public_context(&Nonfriendly { private_only: true }).unwrap();
        assert!(private.certificates.is_empty());
        assert!(private.requires_authentication);
    }
}
