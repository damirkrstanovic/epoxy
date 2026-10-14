use super::*;
use tungstenite::http::{HeaderMap, HeaderValue, header::ORIGIN};

fn headers(value: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(ORIGIN, HeaderValue::from_str(value).unwrap());
    h
}
#[test]
fn origins_are_exact_and_cannot_be_spoofed_by_lookalikes_or_extra_values() {
    let policy = BrowserPolicy::default();
    for origin in APPROVED_ORIGINS {
        assert!(policy.validate_origin(&headers(origin)).is_ok());
    }
    for origin in [
        "null",
        "",
        "http://test.purs.gov.rs",
        "https://test.purs.gov.rs.evil.example",
        "https://eporezi.purs.gov.rs/",
        "https://test.purs.gov.rs:443",
        "https://user@test.purs.gov.rs",
        "https://TEST.PURS.GOV.RS",
        "https://test.purs.gov.rs https://evil.example",
    ] {
        assert_eq!(
            policy.validate_origin(&headers(origin)),
            Err(OriginRejection::Unapproved)
        );
    }
    let mut duplicate = headers(APPROVED_ORIGINS[0]);
    duplicate.append(ORIGIN, HeaderValue::from_static(APPROVED_ORIGINS[0]));
    assert_eq!(
        policy.validate_origin(&duplicate),
        Err(OriginRejection::Multiple)
    );
    let mut binary = HeaderMap::new();
    binary.insert(ORIGIN, HeaderValue::from_bytes(b"\xff").unwrap());
    assert_eq!(
        policy.validate_origin(&binary),
        Err(OriginRejection::Unapproved)
    );
}
#[test]
fn development_exception_admits_only_absent_origin() {
    assert_eq!(
        BrowserPolicy::default().validate_origin(&HeaderMap::new()),
        Err(OriginRejection::Missing)
    );
    let dev = BrowserPolicy::new(true);
    assert!(dev.validate_origin(&HeaderMap::new()).is_ok());
    assert_eq!(
        dev.validate_origin(&headers("null")),
        Err(OriginRejection::Unapproved)
    );
    assert_eq!(
        dev.validate_origin(&headers("https://evil.example")),
        Err(OriginRejection::Unapproved)
    );
}
#[test]
fn loopback_validation_covers_both_families_and_mapped_ipv4() {
    for ip in ["127.0.0.1", "127.0.0.2", "::1", "::ffff:127.0.0.1"] {
        assert!(is_loopback(ip.parse().unwrap()));
    }
    for ip in [
        "0.0.0.0",
        "::",
        "192.168.1.1",
        "::ffff:192.168.1.1",
        "2001:db8::1",
    ] {
        assert!(!is_loopback(ip.parse().unwrap()));
    }
}

async fn attempt(
    origin: Option<&str>,
    policy: BrowserPolicy,
) -> (Result<(), AcceptError>, Result<(), tungstenite::Error>) {
    use tungstenite::client::IntoClientRequest;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut request = format!("ws://{}/", listener.local_addr().unwrap())
        .into_client_request()
        .unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert(ORIGIN, HeaderValue::from_str(origin).unwrap());
    }
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        policy.accept(stream).await.map(|_| ())
    };
    let client = async { tokio_tungstenite::connect_async(request).await.map(|_| ()) };
    tokio::join!(server, client)
}
#[tokio::test]
async fn actual_upgrade_accepts_portals_and_denies_other_or_missing_origins_without_echo() {
    for origin in APPROVED_ORIGINS {
        let (server, client) = attempt(Some(origin), BrowserPolicy::default()).await;
        assert!(server.is_ok() && client.is_ok());
    }
    for origin in [
        None,
        Some("null"),
        Some("https://secret-origin.evil.example"),
    ] {
        let (server, client) = attempt(origin, BrowserPolicy::default()).await;
        assert!(matches!(server, Err(AcceptError::Origin(_))));
        let Err(tungstenite::Error::Http(response)) = client else {
            panic!("HTTP rejection expected")
        };
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(!format!("{server:?}").contains("secret-origin"));
        assert!(
            !String::from_utf8_lossy(response.body().as_ref().unwrap()).contains("secret-origin")
        );
    }
    let (server, client) = attempt(None, BrowserPolicy::new(true)).await;
    assert!(server.is_ok() && client.is_ok());
    let (server, _) = attempt(Some("https://evil.example"), BrowserPolicy::new(true)).await;
    assert!(matches!(
        server,
        Err(AcceptError::Origin(OriginRejection::Unapproved))
    ));
}
#[tokio::test]
async fn incomplete_upgrade_has_a_deadline_and_oversized_http_headers_are_bounded() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let policy = BrowserPolicy {
        handshake_timeout: Duration::from_millis(40),
        ..Default::default()
    };
    assert!(matches!(
        policy.accept(stream).await,
        Err(AcceptError::Timeout)
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let policy = BrowserPolicy {
        handshake_timeout: Duration::from_secs(2),
        ..Default::default()
    };
    let server = async {
        assert!(matches!(
            policy.accept(stream).await,
            Err(AcceptError::Handshake)
        ));
    };
    let send = async {
        let _ = client
            .write_all(format!("GET / HTTP/1.1\r\nX-Ignore: {}", "a".repeat(70_000)).as_bytes())
            .await;
    };
    tokio::join!(server, send);
}
#[tokio::test]
async fn fragment_aggregate_and_single_frame_limits_reject_oversized_messages() {
    use futures::{SinkExt, StreamExt};
    use tungstenite::{
        Message,
        client::IntoClientRequest,
        protocol::frame::{
            Frame,
            coding::{Data, OpCode},
        },
    };
    for fragmented in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut request = format!("ws://{}/", listener.local_addr().unwrap())
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(ORIGIN, HeaderValue::from_static(APPROVED_ORIGINS[0]));
        let server = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = BrowserPolicy::default().accept(stream).await.unwrap();
            let result = tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(result, Err(tungstenite::Error::Capacity(_))),
                "oversized message must fail before application decoding"
            );
        };
        let client = async {
            let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
            if fragmented {
                for (opcode, bytes, last) in [
                    (Data::Text, MAX_REQUEST_BYTES / 2, false),
                    (Data::Continue, MAX_REQUEST_BYTES / 2, false),
                    (Data::Continue, 1, true),
                ] {
                    if socket
                        .send(Message::Frame(Frame::message(
                            vec![b'a'; bytes],
                            OpCode::Data(opcode),
                            last,
                        )))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            } else {
                let _ = socket
                    .send(Message::Text("a".repeat(MAX_REQUEST_BYTES + 1).into()))
                    .await;
            }
        };
        tokio::join!(server, client);
    }
}
