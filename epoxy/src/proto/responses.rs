use serde::Serialize;
use serde_repr::Serialize_repr;
use serde_with::base64::{Base64, Standard};
use serde_with::formats::Unpadded;
use serde_with::{DisplayFromStr, serde_as};

#[derive(Debug, Serialize)]
#[serde(tag = "operation")]
pub enum Response {
    #[serde(rename = "ON_OPEN_EVENT")]
    OnOpenEvent(SuccessResponse<OnOpenEvent>),
    #[serde(rename = "GET_INFO")]
    GetInfo(ResultResponse<GetInfo, GetInfoErrorCode>),
    #[serde(rename = "GET_PROVIDERS")]
    GetProviders(ResultResponse<GetProviders, GetProvidersErrorCode>),
    #[serde(rename = "GET_TERMINALS")]
    GetTerminals(ResultResponse<GetTerminals, GetTerminalsErrorCode>),
    #[serde(rename = "GET_CERTIFICATES")]
    GetCertificates(ResultResponse<GetCertificates, GetCertificatesErrorCode>),
    #[serde(rename = "GET_SIGNED_XML")]
    GetSignedXml(ResultResponse<GetSignedXml, GetSignedXmlErrorCode>),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ResultResponse<T, S> {
    Success(SuccessResponse<T>),
    Error(ErrorResponse<S>),
}

impl<T, S> ResultResponse<T, S> {
    pub fn success(payload: T) -> Self {
        ResultResponse::Success(SuccessResponse::new(payload))
    }

    pub fn error(status: S) -> Self {
        ResultResponse::Error(ErrorResponse::new(status))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessResponse<T> {
    status: u32,
    payload: T,
}

impl<T> SuccessResponse<T> {
    pub fn new(payload: T) -> Self {
        Self { status: 0, payload }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse<S> {
    status: S,
}

impl<S> ErrorResponse<S> {
    pub fn new(status: S) -> Self {
        Self { status }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnOpenEvent {
    pub app_name: String,
    pub app_version: String,
    pub app_build: u32,
    pub os_name: String,
    pub os_version: String,
    pub os_arch: String,
}

impl OnOpenEvent {
    pub fn with_mocked_info() -> Self {
        Self {
            app_name: "SmartBox".into(),
            app_version: "2.0.0".into(),
            app_build: 1,
            os_name: "".into(),
            os_version: "".into(),
            os_arch: "".into(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize_repr)]
#[repr(u32)]
pub enum GetInfoErrorCode {
    GenericError = 1100,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetProviders {
    pub providers: Vec<Provider>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: usize,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Serialize_repr)]
#[repr(u32)]
pub enum GetProvidersErrorCode {
    GenericError = 1200,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTerminals {
    pub terminals: Vec<Terminal>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    pub id: usize,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Serialize_repr)]
#[repr(u32)]
pub enum GetTerminalsErrorCode {
    GenericError = 1300,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetCertificates {
    pub certificates: Vec<Certificate>,
}

#[serde_as]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Certificate {
    #[serde_as(as = "DisplayFromStr")]
    pub alias: usize,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Serialize_repr)]
#[repr(u32)]
pub enum GetCertificatesErrorCode {
    GenericError = 1400,
}

#[serde_as]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSignedXml {
    #[serde_as(as = "Base64<Standard, Unpadded>")]
    pub xml: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Serialize_repr)]
#[repr(u32)]
pub enum GetSignedXmlErrorCode {
    GenericError = 1500,
}

impl Response {
    pub fn operation(&self) -> &'static str {
        match self {
            Self::OnOpenEvent(_) => "ON_OPEN_EVENT",
            Self::GetInfo(_) => "GET_INFO",
            Self::GetProviders(_) => "GET_PROVIDERS",
            Self::GetTerminals(_) => "GET_TERMINALS",
            Self::GetCertificates(_) => "GET_CERTIFICATES",
            Self::GetSignedXml(_) => "GET_SIGNED_XML",
        }
    }
    pub fn status(&self) -> u32 {
        fn code<T, S: Copy + Into<u32>>(result: &ResultResponse<T, S>) -> u32 {
            match result {
                ResultResponse::Success(success) => success.status,
                ResultResponse::Error(error) => error.status.into(),
            }
        }
        match self {
            Self::OnOpenEvent(success) => success.status,
            Self::GetInfo(result) => code(result),
            Self::GetProviders(result) => code(result),
            Self::GetTerminals(result) => code(result),
            Self::GetCertificates(result) => code(result),
            Self::GetSignedXml(result) => code(result),
        }
    }
}

impl From<GetInfoErrorCode> for u32 {
    fn from(code: GetInfoErrorCode) -> Self {
        code as u32
    }
}

impl From<GetProvidersErrorCode> for u32 {
    fn from(code: GetProvidersErrorCode) -> Self {
        code as u32
    }
}

impl From<GetTerminalsErrorCode> for u32 {
    fn from(code: GetTerminalsErrorCode) -> Self {
        code as u32
    }
}

impl From<GetCertificatesErrorCode> for u32 {
    fn from(code: GetCertificatesErrorCode) -> Self {
        code as u32
    }
}

impl From<GetSignedXmlErrorCode> for u32 {
    fn from(code: GetSignedXmlErrorCode) -> Self {
        code as u32
    }
}
