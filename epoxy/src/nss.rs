use std::{
    ffi::{CStr, CString},
    marker::PhantomData,
    os::raw::{c_char, c_void},
    ptr::{self, null, null_mut},
    slice,
};

use thiserror::Error;
use tracing::{debug, error, warn};
use xmlsec_nss_sys::*;

use crate::pin::SecretPin;

pub trait PinPrompt {
    fn prompt_pin(&self, pin_info: &PinInfo, token_name: String) -> Option<SecretPin>;
}

#[derive(Debug, Clone)]
pub struct PinInfo {
    pub cert: Option<String>,
    pub reason: String,
    /// Portal-reported display context; never authority for authentication.
    pub reported_site: Option<String>,
}

pub struct PinContext<'a> {
    pin_prompt: &'a dyn PinPrompt,
    pin_info: PinInfo,
}

impl<'a> PinContext<'a> {
    pub fn new(pin_prompt: &'a dyn PinPrompt, pin_info: PinInfo) -> PinContext<'a> {
        PinContext {
            pin_prompt,
            pin_info,
        }
    }
}

// Allocates a password buffer whose ownership transfers to NSS on return.
fn pin_to_nss(pin: SecretPin) -> *mut c_char {
    if !pin.is_valid() {
        return null_mut();
    }
    let pin_buf = pin.expose().as_bytes();
    let out_buf_ptr = unsafe { PORT_Alloc(pin_buf.len() + 1) } as *mut c_char;
    if out_buf_ptr.is_null() {
        return null_mut();
    }
    unsafe {
        ptr::copy_nonoverlapping(
            pin_buf.as_ptr().cast::<c_char>(),
            out_buf_ptr,
            pin_buf.len(),
        );
        *out_buf_ptr.add(pin_buf.len()) = 0;
    }
    // No intermediate CString copy. Wipe our allocation before handing off.
    // NSS owns the returned buffer; its normal authentication path clears
    // it before PORT_Free. Never retain or touch that pointer after return.
    drop(pin);

    out_buf_ptr
}

pub struct Nss {
    ptr: *mut NSSInitContext,
}

impl Drop for Nss {
    fn drop(&mut self) {
        unsafe { NSS_ShutdownContext(self.ptr) };
    }
}

impl Nss {
    pub fn initialize(nssdb_path: String) -> Result<Nss, InitError> {
        unsafe { PK11_SetPasswordFunc(Some(Self::password_func)) };

        let nssdb_path = CString::new(nssdb_path).map_err(|_| InitError::Ffi)?;
        let flags = 0;
        let ptr = unsafe {
            NSS_InitContext(
                nssdb_path.as_ptr(),
                null(),
                null(),
                null(),
                null_mut(),
                flags,
            )
        };

        if ptr.is_null() {
            return Err(InitError::Native);
        }
        Ok(Nss { ptr })
    }

    extern "C" fn password_func(
        token_ptr: *mut PK11SlotInfo,
        retry: PRBool,
        pin_context_ptr: *mut c_void,
    ) -> *mut c_char {
        if pin_context_ptr.is_null() {
            error!("bug: pin context not provided");
            return null_mut();
        }
        let PinContext {
            pin_prompt,
            pin_info,
        } = unsafe { &*(pin_context_ptr as *const PinContext) };

        if retry != 0 {
            return null_mut();
        }

        if token_ptr.is_null() {
            return null_mut();
        }
        let token_name = Token::from_raw(token_ptr, false).name();

        debug!("requesting card authentication credential");
        let Some(pin) = pin_prompt.prompt_pin(pin_info, token_name) else {
            return null_mut();
        };

        pin_to_nss(pin)
    }

    pub fn get_tokens<'a>(&'a self) -> Option<TokenList<'a>> {
        let ty = CKM_SHA256_RSA_PKCS as u64;
        let need_rw = PR_FALSE as i32;
        let load_certs = PR_FALSE as i32;
        let pin_context_ptr = null_mut(); // not sure if this operation ever needs login in practice

        let ptr = unsafe { PK11_GetAllTokens(ty, need_rw, load_certs, pin_context_ptr) };
        if ptr.is_null() {
            None
        } else {
            Some(TokenList::from_raw(ptr))
        }
    }

    pub fn get_token<'a>(&'a self, token_uri: &TokenUri) -> Option<Token<'a>> {
        let ptr = unsafe { PK11_FindSlotByName(token_uri.0.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(Token::from_raw(ptr, true))
        }
    }

    pub fn find_context<'a>(
        &'a self,
        key: &crate::certificates::ContextKey,
    ) -> Result<Token<'a>, crate::certificates::SelectionError> {
        use crate::certificates::SelectionError;
        let list = self.get_tokens().ok_or(SelectionError::Unavailable)?;
        let mut found = None;
        for token in list.iter() {
            if token.context_key()? == *key {
                if found.is_some() {
                    return Err(SelectionError::Ambiguous);
                }
                let ptr = unsafe { PK11_ReferenceSlot(token.ptr) };
                if ptr.is_null() {
                    return Err(SelectionError::Unavailable);
                }
                found = Some(Token::from_raw(ptr, true));
            }
        }
        found.ok_or(SelectionError::Missing)
    }

    pub fn list_certificates<'a>(&'a self, token: &'a Token<'a>) -> Option<CertList<'a>> {
        let ptr = unsafe { PK11_ListCertsInSlot(token.ptr) };
        if ptr.is_null() {
            None
        } else {
            Some(CertList::from_raw(ptr))
        }
    }

    pub fn get_certs_in_token<'a>(
        &'a self,
        pin_context: &'a PinContext,
        token: &'a Token<'a>,
    ) -> Result<Option<CertList<'a>>, TokenAuthError> {
        let _logout_guard = self.authenticate_unfriendly(pin_context, token)?;

        let ptr = unsafe { PK11_ListCertsInSlot(token.ptr) };
        if ptr.is_null() {
            Ok(None)
        } else {
            Ok(Some(CertList::from_raw(ptr)))
        }
    }

    pub fn authenticate<'a>(
        &'a self,
        pin_context: &'a PinContext,
        token: &'a Token<'a>,
    ) -> Result<TokenLogoutGuard<'a>, TokenAuthError> {
        let pin_context_ptr = &raw const *pin_context as *mut c_void;
        let code = unsafe { PK11_Authenticate(token.ptr, PR_FALSE as i32, pin_context_ptr) };
        if code != _SECStatus_SECSuccess {
            // NSS exposes a numeric, thread-local error; do not request its free-form text.
            let native_error = unsafe { PORT_GetError() };
            warn!(
                operation = "PK11_Authenticate",
                native_status = code,
                native_error,
                "token authentication failed"
            );
            return Err(TokenAuthError(()));
        }
        Ok(TokenLogoutGuard::new(token))
    }

    fn authenticate_unfriendly<'a>(
        &'a self,
        pin_context: &'a PinContext,
        token: &'a Token<'a>,
    ) -> Result<TokenLogoutGuard<'a>, TokenAuthError> {
        let pin_context_ptr = &raw const *pin_context as *mut c_void;
        if !token.is_friendly() {
            let code = unsafe { PK11_Authenticate(token.ptr, PR_FALSE as i32, pin_context_ptr) };
            if code != _SECStatus_SECSuccess {
                // NSS exposes a numeric, thread-local error; do not request its free-form text.
                let native_error = unsafe { PORT_GetError() };
                warn!(
                    operation = "PK11_Authenticate",
                    native_status = code,
                    native_error,
                    "token authentication failed"
                );
                return Err(TokenAuthError(()));
            }
        }
        Ok(TokenLogoutGuard::new(token))
    }
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("C interop error")]
    Ffi,
    #[error("NSS initialization failed")]
    Native,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenUri(CString);

pub struct TokenLogoutGuard<'a> {
    token: &'a Token<'a>,
}

impl<'a> TokenLogoutGuard<'a> {
    fn new(token: &'a Token<'a>) -> TokenLogoutGuard<'a> {
        TokenLogoutGuard { token }
    }
}

impl<'a> Drop for TokenLogoutGuard<'a> {
    fn drop(&mut self) {
        unsafe {
            PK11_Logout(self.token.ptr);
        }
    }
}

#[derive(Debug, Error)]
#[error("token auth failed")]
pub struct TokenAuthError(());

pub struct SecItem {
    item: SECItem,
    bytes: Vec<u8>,
}
impl AsRef<[u8]> for SecItem {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl SecItem {
    pub fn from_der_bytes(bytes: Vec<u8>) -> Result<Self, crate::certificates::SelectionError> {
        if bytes.is_empty() || bytes.len() > 65536 {
            return Err(crate::certificates::SelectionError::InvalidCertificate);
        }
        let item = SECItem {
            type_: SECItemType_siBuffer,
            data: bytes.as_ptr() as *mut u8,
            len: bytes.len() as u32,
        };
        Ok(Self { item, bytes })
    }
    fn new(value: &SECItem) -> Result<Self, crate::certificates::SelectionError> {
        if value.data.is_null() || value.len == 0 || value.len > 65536 {
            return Err(crate::certificates::SelectionError::InvalidCertificate);
        }
        Self::from_der_bytes(
            unsafe { slice::from_raw_parts(value.data, value.len as usize) }.to_vec(),
        )
    }
    pub fn as_ptr(&self) -> *const SECItem {
        &self.item
    }
}

pub struct Token<'a> {
    ptr: *mut PK11SlotInfo,
    is_owned: bool,
    _marker: PhantomData<&'a Nss>,
}

impl<'a> Drop for Token<'a> {
    fn drop(&mut self) {
        if self.is_owned {
            unsafe { PK11_FreeSlot(self.ptr) }
        }
    }
}

impl<'a> Token<'a> {
    fn from_raw(ptr: *mut PK11SlotInfo, is_owned: bool) -> Token<'a> {
        Token {
            ptr,
            is_owned,
            _marker: PhantomData,
        }
    }

    pub fn uri(&self) -> Option<TokenUri> {
        let uri_ptr = unsafe { PK11_GetTokenURI(self.ptr as *mut _) };
        if uri_ptr.is_null() {
            return None;
        }
        let uri = unsafe { CStr::from_ptr(uri_ptr).to_owned() };
        unsafe { PORT_Free(uri_ptr as *mut c_void) }; // `PK11_GetTokenURI` allocates string
        Some(TokenUri(uri))
    }

    pub fn context_key(
        &self,
    ) -> Result<crate::certificates::ContextKey, crate::certificates::SelectionError> {
        use crate::certificates::{ContextKey, SelectionError};
        let module = unsafe { PK11_GetModule(self.ptr) };
        if module.is_null() {
            return Err(SelectionError::Unavailable);
        }
        // Borrowed module fields are copied while this slot reference is alive.
        let name = unsafe { (*module).commonName };
        if name.is_null() {
            return Err(SelectionError::Unavailable);
        }
        let module_name = unsafe { CStr::from_ptr(name) }
            .to_str()
            .map_err(|_| SelectionError::Unavailable)?
            .to_owned();
        let library = unsafe { (*module).dllName };
        let module_library = if library.is_null() {
            None
        } else {
            Some(
                unsafe { CStr::from_ptr(library) }
                    .to_str()
                    .map_err(|_| SelectionError::Unavailable)?
                    .to_owned(),
            )
        };
        let uri = self.uri().ok_or(SelectionError::Unavailable)?;
        let mut info: CK_TOKEN_INFO = unsafe { std::mem::zeroed() };
        if unsafe { PK11_GetTokenInfo(self.ptr, &mut info) } != _SECStatus_SECSuccess
            || !crate::certificates::uri_matches_info(
                uri.0.as_bytes(),
                [
                    &info.label,
                    &info.manufacturerID,
                    &info.serialNumber,
                    &info.model,
                ],
            )
        {
            return Err(SelectionError::Missing);
        }
        Ok(ContextKey {
            module_name,
            module_library,
            token_uri: uri.0.as_bytes().to_vec(),
        })
    }
    pub fn identity(
        &self,
    ) -> Result<crate::certificates::TokenIdentity, crate::certificates::SelectionError> {
        if unsafe { PK11_IsPresent(self.ptr) } == 0 {
            return Err(crate::certificates::SelectionError::Missing);
        }
        let series = unsafe { PK11_GetSlotSeries(self.ptr) };
        let context = self.context_key()?;
        if series != unsafe { PK11_GetSlotSeries(self.ptr) } {
            return Err(crate::certificates::SelectionError::Stale);
        }
        Ok(crate::certificates::TokenIdentity { context, series })
    }
    pub fn needs_login(&self) -> bool {
        unsafe { PK11_NeedLogin(self.ptr) != 0 }
    }

    pub(crate) fn matches_private_key(&self, key: *mut SECKEYPrivateKey) -> bool {
        if key.is_null() {
            return false;
        }
        // NSS returns a referenced slot; always release it, even on mismatch.
        let slot = unsafe { PK11_GetSlotFromPrivateKey(key) };
        if slot.is_null() {
            return false;
        }
        let actual = Self::from_raw(slot, true);
        self.ptr == actual.ptr
    }

    pub fn name(&self) -> String {
        let token_name_ptr = unsafe { PK11_GetTokenName(self.ptr as *mut _) };
        if token_name_ptr.is_null() {
            return "Unnamed PIN context".into();
        }
        unsafe { CStr::from_ptr(token_name_ptr).to_string_lossy().to_string() }
    }

    /// Can read certificates without PIN
    pub fn is_friendly(&self) -> bool {
        (unsafe { PK11_IsFriendly(self.ptr as *mut _) }) > 0
    }

    /// Is a hardware token
    pub fn is_hw(&self) -> bool {
        (unsafe { PK11_IsHW(self.ptr as *mut _) }) > 0
    }

    /// Is an NSS internal token
    pub fn is_internal(&self) -> bool {
        (unsafe { PK11_IsInternal(self.ptr as *mut _) }) > 0
    }
}

pub struct TokenList<'a> {
    ptr: *mut PK11SlotList,
    _marker: PhantomData<&'a Nss>,
}

impl<'a> TokenList<'a> {
    pub fn iter(&'a self) -> TokenListIter<'a> {
        TokenListIter::new(self)
    }

    fn from_raw(ptr: *mut PK11SlotList) -> TokenList<'a> {
        TokenList {
            ptr,
            _marker: PhantomData,
        }
    }
}

impl<'a> Drop for TokenList<'a> {
    fn drop(&mut self) {
        unsafe { PK11_FreeSlotList(self.ptr) };
    }
}

pub struct TokenListIter<'a> {
    list: &'a TokenList<'a>,
    item_ptr: *mut PK11SlotListElement,
}

impl<'a> TokenListIter<'a> {
    fn new(list: &'a TokenList<'a>) -> TokenListIter<'a> {
        TokenListIter {
            list,
            item_ptr: unsafe { PK11_GetFirstSafe(list.ptr) },
        }
    }
}

impl<'a> Iterator for TokenListIter<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.item_ptr.is_null() {
            return None;
        }

        let token = Token::from_raw(unsafe { *self.item_ptr }.slot, false);

        self.item_ptr =
            unsafe { PK11_GetNextSafe(self.list.ptr as *mut _, self.item_ptr, PR_FALSE as i32) };

        Some(token)
    }
}

pub struct Cert<'a> {
    ptr: *const CERTCertificate,
    _marker: PhantomData<&'a Nss>,
}

impl<'a> Cert<'a> {
    fn from_raw(ptr: *const CERTCertificate) -> Cert<'a> {
        Cert {
            ptr,
            _marker: PhantomData,
        }
    }

    pub fn der(&self) -> Result<SecItem, crate::certificates::SelectionError> {
        SecItem::new(&unsafe { *self.ptr }.derCert)
    }
}

pub struct CertList<'a> {
    ptr: *mut CERTCertList,
    _marker: PhantomData<&'a Nss>,
}

impl<'a> Drop for CertList<'a> {
    fn drop(&mut self) {
        unsafe {
            CERT_DestroyCertList(self.ptr);
        }
    }
}

impl<'a> CertList<'a> {
    pub fn iter(&'a self) -> CertListIter<'a> {
        CertListIter::new(self)
    }

    fn from_raw(ptr: *mut CERTCertList) -> CertList<'a> {
        CertList {
            ptr,
            _marker: PhantomData,
        }
    }

    fn list_ptr(&self) -> *const PRCList {
        unsafe { &raw const (*self.ptr).list }
    }

    fn is_end(&self, node_ptr: *const CERTCertListNode) -> bool {
        node_ptr.addr() == self.list_ptr().addr()
    }

    fn head_ptr(&self) -> Option<*const CERTCertListNode> {
        let node_ptr = unsafe { *self.list_ptr() }.next as *const CERTCertListNode;
        if self.is_end(node_ptr) {
            None
        } else {
            Some(node_ptr)
        }
    }

    fn next_ptr(&self, node_ptr: *const CERTCertListNode) -> Option<*const CERTCertListNode> {
        let node_ptr = unsafe { *node_ptr }.links.next as *const CERTCertListNode;
        if self.is_end(node_ptr) {
            None
        } else {
            Some(node_ptr)
        }
    }
}

pub struct CertListIter<'a> {
    list: &'a CertList<'a>,
    item_ptr: Option<*const CERTCertListNode>,
}

impl<'a> CertListIter<'a> {
    fn new(list: &'a CertList<'a>) -> CertListIter<'a> {
        CertListIter {
            list,
            item_ptr: list.head_ptr(),
        }
    }
}

impl<'a> Iterator for CertListIter<'a> {
    type Item = Cert<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let item_ptr = self.item_ptr?;
        self.item_ptr = self.list.next_ptr(item_ptr);
        Some(Cert::from_raw(unsafe { *item_ptr }.cert))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_initialization_never_constructs_a_null_native_context() {
        assert!(Nss::initialize("sql:/nonexistent/epoxy-synthetic-invalid-db".into()).is_err());
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    #[test]
    #[ignore = "requires scripts/test-certificates-native.sh private synthetic NSS fixture"]
    fn xmlsec_key_slot_is_checked_and_all_references_are_released() {
        let db =
            std::env::var("EPOXY_SYNTHETIC_NSS_DB").expect("private synthetic database required");
        let der_path = std::env::var("EPOXY_SYNTHETIC_CERT_DER").expect("synthetic DER required");
        let output =
            std::env::var("EPOXY_SYNTHETIC_SIGNED_XML").expect("synthetic output required");
        let nss = std::mem::ManuallyDrop::new(Nss::initialize(format!("sql:{db}")).unwrap());
        {
            let good = Token::from_raw(unsafe { PK11_GetInternalKeySlot() }, true);
            let wrong = Token::from_raw(unsafe { PK11_GetInternalSlot() }, true);
            assert!(!good.ptr.is_null() && !wrong.ptr.is_null() && good.ptr != wrong.ptr);
            let identity = good.identity().unwrap();
            identity
                .ensure_unchanged(&good.identity().unwrap())
                .unwrap();
            let xmlsec = crate::xmlsec::XmlSec::initialize().unwrap();
            let der = SecItem::from_der_bytes(std::fs::read(der_path).unwrap()).unwrap();
            let signer = crate::xmlsec::XmlSigner::with_cert(&xmlsec, &der).unwrap();
            for _ in 0..100 {
                assert!(signer.matches_token(&good));
                assert!(!signer.matches_token(&wrong));
            }
            let mut doc = libxml::parser::Parser::default()
                .parse_string("<synthetic>native fixture</synthetic>")
                .unwrap();
            signer.sign(&mut doc).unwrap();
            std::fs::write(output, doc.to_string()).unwrap();
        }
        // NSS refuses shutdown with leaked references: this catches missing drops
        // of the XMLSec key copy or PK11_GetSlotFromPrivateKey's referenced slot.
        assert_eq!(
            unsafe { NSS_ShutdownContext(nss.ptr) },
            _SECStatus_SECSuccess
        );
    }
}

#[cfg(test)]
mod pin_buffer_tests {
    use super::*;
    #[test]
    fn native_pin_handoff_is_terminated_and_rejects_invalid_values() {
        for secret in ["synthetic\0secret".to_string(), "s".repeat(1025)] {
            assert!(pin_to_nss(secret.into()).is_null());
        }
        let pointer = pin_to_nss("synthetic-secret".into());
        assert!(!pointer.is_null());
        unsafe {
            // This test owns the buffer; it has never been passed to NSS.
            let text = CStr::from_ptr(pointer);
            assert_eq!(text.to_bytes(), b"synthetic-secret");
            PORT_ZFree(pointer.cast(), text.to_bytes_with_nul().len());
        }
    }
}
