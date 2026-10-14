use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x509_parser::prelude::{FromDer, X509Certificate};

// Deliberately no Debug/Display: token URIs and module labels can contain identity data.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextKey {
    pub module_name: String,
    pub module_library: Option<String>,
    pub token_uri: Vec<u8>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateKey {
    pub context: ContextKey,
    pub der_sha256: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eligibility {
    Eligible,
    Expired,
    NotYetValid,
    Unsupported,
}
impl Eligibility {
    pub fn label(self) -> &'static str {
        match self {
            Self::Eligible => "Available for signing",
            Self::Expired => "Expired",
            Self::NotYetValid => "Not yet valid",
            Self::Unsupported => "Unsupported signing key or key usage",
        }
    }
}
#[derive(Clone)]
pub struct PublicCertificate {
    pub key: CertificateKey,
    pub holder: String,
    pub issuer: String,
    pub not_before: String,
    pub not_after: String,
    pub purpose: String,
    pub eligibility: Eligibility,
}
impl PublicCertificate {
    pub fn from_der(context: ContextKey, der: &[u8]) -> Result<Self, SelectionError> {
        let (remaining, x509) =
            X509Certificate::from_der(der).map_err(|_| SelectionError::InvalidCertificate)?;
        if !remaining.is_empty() {
            return Err(SelectionError::InvalidCertificate);
        }
        let usage = x509.key_usage().ok().flatten();
        let digital = usage.as_ref().is_some_and(|u| u.value.digital_signature());
        let commitment = usage.as_ref().is_some_and(|u| u.value.non_repudiation());
        let now = x509_parser::time::ASN1Time::now();
        let eligibility = if x509.validity().not_after < now {
            Eligibility::Expired
        } else if x509.validity().not_before > now {
            Eligibility::NotYetValid
        } else if !digital
            || !commitment
            || x509.public_key().algorithm.algorithm.to_id_string() != "1.2.840.113549.1.1.1"
        {
            Eligibility::Unsupported
        } else {
            Eligibility::Eligible
        };
        let holder = x509
            .subject()
            .iter_common_name()
            .next()
            .and_then(|a| a.as_str().ok())
            .map(str::to_owned)
            .unwrap_or_else(|| x509.subject().to_string());
        let purpose = match (digital, commitment) {
            (true, true) => "Digital signature, content commitment",
            (true, false) => "Digital signature",
            (false, true) => "Content commitment",
            _ => "No supported signing usage",
        }
        .into();
        Ok(Self {
            key: CertificateKey {
                context,
                der_sha256: Sha256::digest(der).into(),
            },
            holder,
            issuer: x509.issuer().to_string(),
            not_before: x509.validity().not_before.to_string(),
            not_after: x509.validity().not_after.to_string(),
            purpose,
            eligibility,
        })
    }
}
#[derive(Clone)]
pub struct PublicContext {
    pub key: ContextKey,
    pub label: String,
    pub requires_authentication: bool,
    pub certificates: Vec<PublicCertificate>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    #[error("Certificate list changed; refresh and select again")]
    Stale,
    #[error("Selected certificate is unavailable; refresh or clear the selection")]
    Missing,
    #[error("More than one context matches; selection is ambiguous")]
    Ambiguous,
    #[error("Certificate is not eligible for signing")]
    Ineligible,
    #[error("Epoxy is busy; try again after the current operation")]
    Busy,
    #[error("Certificate service is unavailable")]
    Unavailable,
    #[error("Invalid certificate data")]
    InvalidCertificate,
    #[error("Cannot read or save the certificate preference safely")]
    Preference,
}
#[derive(Clone, Default)]
pub struct CatalogModel {
    entries: Vec<PublicCertificate>,
    generation: u64,
    selected: Option<CertificateKey>,
    revision: u64,
}
impl CatalogModel {
    pub fn from_entries(entries: Vec<PublicCertificate>) -> Self {
        Self {
            entries,
            generation: 1,
            ..Default::default()
        }
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn selected_key(&self) -> Option<&CertificateKey> {
        self.selected.as_ref()
    }
    pub fn replace_entries(&mut self, entries: Vec<PublicCertificate>) {
        self.entries = entries;
        self.generation += 1;
    }
    pub fn unique(&self, key: &CertificateKey) -> Result<&PublicCertificate, SelectionError> {
        let mut matching = self.entries.iter().filter(|c| &c.key == key);
        let cert = matching.next().ok_or(SelectionError::Missing)?;
        if matching.next().is_some() {
            return Err(SelectionError::Ambiguous);
        }
        if cert.eligibility != Eligibility::Eligible {
            return Err(SelectionError::Ineligible);
        }
        Ok(cert)
    }
    pub fn select(&mut self, key: &CertificateKey, generation: u64) -> Result<(), SelectionError> {
        if generation != self.generation {
            return Err(SelectionError::Stale);
        }
        self.unique(key)?;
        self.set_selection(Some(key.clone()));
        Ok(())
    }
    pub(crate) fn set_selection(&mut self, key: Option<CertificateKey>) {
        self.selected = key;
        self.revision += 1;
    }
}
