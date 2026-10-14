//! WebSocket admission policy. Origin is a browser boundary, not local-process authentication.
use std::{net::IpAddr, time::Duration};
use tokio::{net::TcpStream, time::timeout};
use tokio_tungstenite::{WebSocketStream, accept_hdr_async_with_config};
use tungstenite::{
    handshake::server::{ErrorResponse, Request, Response},
    http::{HeaderMap, StatusCode, header::ORIGIN},
    protocol::WebSocketConfig,
};

pub const APPROVED_ORIGINS: [&str; 2] = ["https://eporezi.purs.gov.rs", "https://test.purs.gov.rs"];
pub const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OriginRejection {
    #[error("an approved browser Origin is required")]
    Missing,
    #[error("multiple Origin headers are not permitted")]
    Multiple,
    #[error("browser Origin is not approved")]
    Unapproved,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AcceptError {
    #[error("peer address unavailable")]
    PeerUnavailable,
    #[error("only loopback clients are permitted")]
    NonLoopback,
    #[error("{0}")]
    Origin(OriginRejection),
    #[error("WebSocket upgrade timed out")]
    Timeout,
    #[error("WebSocket upgrade failed")]
    Handshake,
}
impl AcceptError {
    pub fn kind(self) -> &'static str {
        match self {
            Self::PeerUnavailable => "peer_unavailable",
            Self::NonLoopback => "non_loopback_peer",
            Self::Origin(OriginRejection::Missing) => "origin_missing",
            Self::Origin(OriginRejection::Multiple) => "origin_multiple",
            Self::Origin(OriginRejection::Unapproved) => "origin_unapproved",
            Self::Timeout => "websocket_upgrade_timeout",
            Self::Handshake => "websocket_upgrade_failed",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct BrowserPolicy {
    allow_missing_origin: bool,
    handshake_timeout: Duration,
}
impl Default for BrowserPolicy {
    fn default() -> Self {
        Self::new(false)
    }
}
impl BrowserPolicy {
    pub fn new(allow_missing_origin: bool) -> Self {
        Self {
            allow_missing_origin,
            handshake_timeout: Duration::from_secs(5),
        }
    }
    pub fn validate_origin(self, headers: &HeaderMap) -> Result<(), OriginRejection> {
        let mut origins = headers.get_all(ORIGIN).iter();
        let Some(origin) = origins.next() else {
            return if self.allow_missing_origin {
                Ok(())
            } else {
                Err(OriginRejection::Missing)
            };
        };
        if origins.next().is_some() {
            return Err(OriginRejection::Multiple);
        }
        if origin
            .to_str()
            .is_ok_and(|value| APPROVED_ORIGINS.contains(&value))
        {
            Ok(())
        } else {
            Err(OriginRejection::Unapproved)
        }
    }
    pub fn websocket_config(self) -> WebSocketConfig {
        WebSocketConfig::default()
            .max_message_size(Some(MAX_REQUEST_BYTES))
            .max_frame_size(Some(MAX_REQUEST_BYTES))
            .max_write_buffer_size(2 * MAX_REQUEST_BYTES)
    }
    pub async fn accept(
        self,
        stream: TcpStream,
    ) -> Result<WebSocketStream<TcpStream>, AcceptError> {
        let peer = stream
            .peer_addr()
            .map_err(|_| AcceptError::PeerUnavailable)?;
        if !is_loopback(peer.ip()) {
            return Err(AcceptError::NonLoopback);
        }
        let mut rejection = None;
        // Tungstenite requires this response type in its upgrade callback.
        #[allow(clippy::result_large_err)]
        let callback = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            match self.validate_origin(request.headers()) {
                Ok(()) => Ok(response),
                Err(reason) => {
                    rejection = Some(reason);
                    let mut error =
                        ErrorResponse::new(Some("Portal origin is not permitted".into()));
                    *error.status_mut() = StatusCode::FORBIDDEN;
                    Err(error)
                }
            }
        };
        match timeout(
            self.handshake_timeout,
            accept_hdr_async_with_config(stream, callback, Some(self.websocket_config())),
        )
        .await
        {
            Err(_) => Err(AcceptError::Timeout),
            Ok(Ok(socket)) => Ok(socket),
            Ok(Err(_)) => Err(rejection
                .map(AcceptError::Origin)
                .unwrap_or(AcceptError::Handshake)),
        }
    }
}

pub fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => {
            ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|ip| ip.is_loopback())
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NetworkDeadlines {
    pub first_request: Duration,
    pub request_idle: Duration,
    pub response_write: Duration,
}
impl Default for NetworkDeadlines {
    fn default() -> Self {
        Self {
            first_request: Duration::from_secs(10),
            request_idle: Duration::from_secs(600),
            response_write: Duration::from_secs(5),
        }
    }
}
#[cfg(test)]
mod tests;
