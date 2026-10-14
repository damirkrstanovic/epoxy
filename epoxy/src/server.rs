use crate::desktop::{DesktopController, OperationKind};
use crate::diagnostics::Diagnostics;
use std::{
    cell::{Cell, LazyCell},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
static NEXT_CORRELATION: AtomicU64 = AtomicU64::new(1);

use anyhow::{Context, Error, Result, anyhow};
use futures::{SinkExt, StreamExt, TryStreamExt, future};
use libxml::{
    parser::{Parser, ParserOptions, XmlParseError},
    tree::Document,
};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tracing::{info, warn};
use tungstenite::{Message, Utf8Bytes};
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::{
    nss::{Nss, PinContext, PinInfo, PinPrompt, SecItem, TokenUri},
    proto::{
        requests::{self, Request},
        responses::{self, Response, ResultResponse, SuccessResponse},
    },
    xmlsec::{XmlSec, XmlSigner},
};

impl TryFrom<Response> for Message {
    type Error = Error;

    fn try_from(response: Response) -> Result<Message> {
        let text = Utf8Bytes::from(
            serde_json::to_string(&response).context("failed to serialize response")?,
        );

        let message = Message::Text(text);
        Ok(message)
    }
}

impl TryFrom<Message> for Request {
    type Error = Error;

    fn try_from(request: Message) -> Result<Request> {
        let text = request
            .into_text()
            .context("failed to convert WebSocket message to text")?;

        parse_request(&text)
    }
}

fn parse_request(text: &str) -> Result<Request> {
    serde_json::from_str(text).map_err(|e| {
        anyhow!(
            "invalid protocol request at line {} column {}",
            e.line(),
            e.column()
        )
    })
}

#[derive(Debug, Default)]
pub struct Session {
    terminals: Vec<TokenUri>,
    certs: Vec<(TokenUri, SecItem, String)>,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub allow_soft_tokens: bool,
}

pub struct Server<F> {
    config: ServerConfig,
    diagnostics: Diagnostics,
    desktop: DesktopController,
    pin_prompt: Box<dyn PinPrompt>,
    nss_xmlsec: LazyCell<(Nss, XmlSec), F>,
    xml_parser: Parser,
}

impl<F> Server<F>
where
    F: FnOnce() -> (Nss, XmlSec),
{
    pub fn new(
        config: ServerConfig,
        pin_prompt: Box<dyn PinPrompt>,
        nss_xmlsec: LazyCell<(Nss, XmlSec), F>,
        diagnostics: Diagnostics,
        desktop: DesktopController,
    ) -> Server<F> {
        Server {
            config,
            diagnostics,
            desktop,
            pin_prompt,
            nss_xmlsec,
            xml_parser: Parser::default(),
        }
    }

    pub async fn run(&self, ws_stream: WebSocketStream<TcpStream>) -> Result<()> {
        let _connection = self.desktop.connection_started();
        anyhow::ensure!(
            !self.desktop.shutdown_requested().is_cancelled(),
            "service shutting down"
        );
        let (tx, rx) = ws_stream.split();

        let context = Rc::new(Cell::new((0, Instant::now())));
        let operation_context = context.clone();
        let response_context = context.clone();
        let response_diagnostics = self.diagnostics.clone();
        let mut tx = Box::pin(tx.with(move |response: Response| {
            let (correlation_id, started) = response_context.get();
            info!(
                correlation_id,
                operation = response.operation(),
                status = response.status(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "protocol operation completed"
            );
            let message = Message::try_from(response);
            if let Ok(Message::Text(text)) = &message {
                response_diagnostics.wire("response", text, correlation_id);
            }
            future::ready(message)
        }));
        let request_diagnostics = self.diagnostics.clone();
        let mut rx = Box::pin(
            rx.try_take_while(|message| future::ok(!message.is_close()))
                .map_err(|_| anyhow!("WebSocket transport error"))
                .and_then(move |message| {
                    let correlation_id = NEXT_CORRELATION.fetch_add(1, Ordering::Relaxed);
                    context.set((correlation_id, Instant::now()));
                    let request = message
                        .into_text()
                        .map_err(|_| anyhow!("invalid WebSocket text message"))
                        .and_then(|text| {
                            request_diagnostics.wire("request", &text, correlation_id);
                            parse_request(&text)
                        });
                    match &request {
                        Ok(request) => info!(
                            correlation_id,
                            operation = request.operation(),
                            "protocol operation received"
                        ),
                        Err(_) => warn!(
                            correlation_id,
                            error_kind = "invalid_request",
                            "protocol request rejected"
                        ),
                    }
                    future::ready(request)
                }),
        );

        let session = self
            .handshake(&mut tx, &mut rx)
            .await
            .context("protocol handshake failed")?;

        self.handle_requests(tx, rx, session, operation_context)
            .await
            .context("request handler failed")
    }

    async fn handshake(
        &self,
        mut tx: impl SinkExt<Response, Error = Error> + Unpin,
        mut rx: impl TryStreamExt<Ok = Request, Error = Error> + Unpin,
    ) -> Result<Session> {
        let on_open_event = SuccessResponse::new(responses::OnOpenEvent::with_mocked_info());
        tx.send(Response::OnOpenEvent(on_open_event)).await?;

        let Some(first_req) = rx.try_next().await? else {
            return Err(anyhow!("connection closed before protocol handshake"));
        };

        let Request::GetInfo(get_info_req) = first_req else {
            return Err(anyhow!("first request must be GET_INFO"));
        };

        if get_info_req.sb_session.is_empty() {
            return Err(anyhow!("invalid first GET_INFO request"));
        }

        anyhow::ensure!(
            !self.desktop.shutdown_requested().is_cancelled(),
            "service shutting down"
        );
        let operation = self
            .desktop
            .operation_started(0, OperationKind::Information);
        let response = Response::GetInfo(self.get_info());
        operation.complete(response.status());
        tx.send(response).await?;

        Ok(Session::default())
    }

    async fn handle_requests(
        &self,
        mut tx: impl SinkExt<Response, Error = Error> + Unpin,
        mut rx: impl TryStreamExt<Ok = Request, Error = Error> + Unpin,
        mut session: Session,
        context: Rc<Cell<(u64, Instant)>>,
    ) -> Result<()> {
        while let Some(request) = rx.try_next().await? {
            anyhow::ensure!(
                !self.desktop.shutdown_requested().is_cancelled(),
                "service shutting down"
            );
            let kind = match &request {
                Request::GetSignedXml(_) => OperationKind::Signing,
                Request::GetCertificates(_) | Request::GetTerminals(_) => {
                    OperationKind::Enumeration
                }
                _ => OperationKind::Information,
            };
            let operation = self.desktop.operation_started(context.get().0, kind);
            let response = {
                let span = tracing::info_span!(
                    "operation",
                    correlation_id = context.get().0,
                    operation = request.operation()
                );
                let _entered = span.enter();
                match request {
                    Request::GetInfo(_) => Response::GetInfo(self.get_info()),
                    Request::GetProviders => Response::GetProviders(self.get_providers()),
                    Request::GetTerminals(_) => {
                        Response::GetTerminals(self.get_terminals(&mut session))
                    }
                    Request::GetCertificates(req) => {
                        Response::GetCertificates(self.get_certificates(&mut session, req))
                    }
                    Request::GetSignedXml(req) => {
                        Response::GetSignedXml(self.get_signed_xml(&session, req))
                    }
                }
            };
            operation.complete(response.status());
            tx.send(response).await?;
        }
        Ok(())
    }

    fn get_info(&self) -> ResultResponse<responses::GetInfo, responses::GetInfoErrorCode> {
        ResultResponse::success(responses::GetInfo {
            provider_id: None,
            terminal_id: None,
            certificate_id: None,
        })
    }

    fn get_providers(
        &self,
    ) -> ResultResponse<responses::GetProviders, responses::GetProvidersErrorCode> {
        let providers = vec![responses::Provider {
            id: 0,
            name: String::from("NSS"),
        }];
        ResultResponse::success(responses::GetProviders { providers })
    }

    fn get_terminals(
        &self,
        session: &mut Session,
    ) -> ResultResponse<responses::GetTerminals, responses::GetTerminalsErrorCode> {
        let (ref nss, _) = *self.nss_xmlsec;

        let tokens = nss.get_tokens();
        let Some(tokens) = tokens else {
            return ResultResponse::error(responses::GetTerminalsErrorCode::GenericError);
        };

        let valid_tokens = tokens.iter().filter_map(|token| {
            if (self.config.allow_soft_tokens || token.is_hw()) && !token.is_internal() {
                Some((token.uri(), token.name()))
            } else {
                None
            }
        });

        session.terminals.clear();
        let mut terminals = Vec::new();

        for (index, (uri, name)) in valid_tokens.enumerate() {
            session.terminals.push(uri);
            terminals.push(responses::Terminal { id: index, name });
        }

        ResultResponse::success(responses::GetTerminals { terminals })
    }

    fn get_certificates(
        &self,
        session: &mut Session,
        request: requests::GetCertificates,
    ) -> ResultResponse<responses::GetCertificates, responses::GetCertificatesErrorCode> {
        let (ref nss, _) = *self.nss_xmlsec;

        let token_uri = session.terminals.get(request.terminal_id);
        let Some(token_uri) = token_uri else {
            return ResultResponse::error(responses::GetCertificatesErrorCode::GenericError);
        };

        let token = nss.get_token(token_uri);
        let Some(token) = token else {
            return ResultResponse::error(responses::GetCertificatesErrorCode::GenericError);
        };

        // here PIN will be requested only if the token doesn't allow passwordless access to certs
        let pin_context = self.get_pin_context(None, String::from("List certificates"));
        let Ok(certs) = nss.get_certs_in_token(&pin_context, &token) else {
            warn!(
                error_kind = "authentication_failed",
                "token authentication failed"
            );
            return ResultResponse::error(responses::GetCertificatesErrorCode::GenericError);
        };
        let Some(certs) = certs else {
            return ResultResponse::error(responses::GetCertificatesErrorCode::GenericError);
        };

        let valid_certs = certs.iter().filter_map(|cert| {
            let der = cert.der();

            let (_, x509) = X509Certificate::from_der(der.as_ref()).ok()?;
            let common_name = x509.subject().iter_common_name().next()?;
            let label = common_name.as_str().ok()?.to_owned();

            // keep only signing certs
            let key_usage = x509.key_usage().ok()??.value;
            if !(key_usage.digital_signature() && key_usage.non_repudiation()) {
                return None;
            }

            if !x509.validity().is_valid() {
                warn!(
                    error_kind = "certificate_validity",
                    "certificate is expired or not yet active"
                );
                return None;
            }

            Some((der, label))
        });

        session.certs.clear();
        let mut certificates = Vec::new();

        for (index, (der, label)) in valid_certs.enumerate() {
            session.certs.push((token_uri.clone(), der, label.clone()));
            certificates.push(responses::Certificate {
                alias: index,
                name: label,
            });
        }

        ResultResponse::success(responses::GetCertificates { certificates })
    }

    fn get_signed_xml(
        &self,
        session: &Session,
        request: requests::GetSignedXml,
    ) -> ResultResponse<responses::GetSignedXml, responses::GetSignedXmlErrorCode> {
        if self.desktop.shutdown_requested().is_cancelled() {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        }
        let (ref nss, ref xmlsec) = *self.nss_xmlsec;

        let cert = session.certs.get(request.certificate.alias);
        let Some((token_uri, der, cert_label)) = cert else {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        };

        let Some(token) = nss.get_token(token_uri) else {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        };

        let Ok(mut document) = self.parse_xml(request.xml) else {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        };

        let reason = match try_identify_form(&document) {
            Some(EporeziFormKind::LoginForm) => String::from("Log in to ePorezi"),
            Some(EporeziFormKind::TaxForm(server_path)) => {
                format!("Sign form {}", server_path)
            }
            None => String::from("Sign form (unidentified)"),
        };

        info!("signature requested");
        {
            let pin_context = self.get_pin_context(Some(cert_label.clone()), reason);
            let Ok(_logout_guard) = nss.authenticate(&pin_context, &token) else {
                warn!(
                    error_kind = "authentication_failed",
                    "token authentication failed"
                );
                return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
            };

            let Ok(signer) = XmlSigner::with_cert(xmlsec, der) else {
                return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
            };

            if self.desktop.shutdown_requested().is_cancelled() {
                return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
            }
            let Ok(()) = signer.sign(&mut document) else {
                return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
            };
        }

        if self.desktop.shutdown_requested().is_cancelled() {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        }
        let xml = document.to_string().into_bytes();

        ResultResponse::success(responses::GetSignedXml { xml })
    }

    fn get_pin_context(&'_ self, cert: Option<String>, reason: String) -> PinContext<'_> {
        let pin_info = PinInfo { cert, reason };
        PinContext::new(&*self.pin_prompt, pin_info)
    }

    fn parse_xml(&self, xml: Vec<u8>) -> Result<Document, XmlParseError> {
        let opts = ParserOptions {
            recover: false,
            ..Default::default()
        };
        self.xml_parser.parse_string_with_options(xml, opts)
    }
}

enum EporeziFormKind {
    LoginForm,
    TaxForm(String),
}

fn try_identify_form(document: &Document) -> Option<EporeziFormKind> {
    let root = document.get_root_element()?;

    let login_form_nodes = root.findnodes("/*[local-name() = 'envelopaEPrijave']/timestamp");
    if let Ok(nodes) = login_form_nodes
        && nodes.len() == 1
    {
        return Some(EporeziFormKind::LoginForm);
    }

    let tax_form_nodes =
        root.findnodes("/*[local-name() = 'envelopaEPrijave']/deklaracijaZaglavlje/xmlPath");
    if let Ok(nodes) = tax_form_nodes {
        let node = nodes.first()?;
        let server_path = node.get_content();
        if !server_path.is_empty() {
            return Some(EporeziFormKind::TaxForm(server_path));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Self;
        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }
    #[test]
    fn shutdown_rejects_queued_signing_before_native_initialization() {
        struct NoPin;
        impl PinPrompt for NoPin {
            fn prompt_pin(&self, _: &PinInfo, _: String) -> Option<String> {
                panic!("cancelled work must not prompt")
            }
        }
        let (diagnostics, _subscriber) =
            crate::diagnostics::subscriber("info", std::io::sink).unwrap();
        let desktop = DesktopController::new(diagnostics.clone());
        let server = Server::new(
            ServerConfig {
                allow_soft_tokens: false,
            },
            Box::new(NoPin),
            LazyCell::new(|| panic!("cancelled work must not initialize NSS")),
            diagnostics,
            desktop.clone(),
        );
        desktop.request_quit();
        let response = server.get_signed_xml(
            &Session::default(),
            requests::GetSignedXml {
                certificate: requests::Certificate {
                    alias: 0,
                    name: String::new(),
                },
                pin: String::new(),
                xml: b"<synthetic/>".to_vec(),
            },
        );
        assert!(matches!(response, ResultResponse::Error(_)));
    }
    #[test]
    fn parsing_error_does_not_echo_invalid_operation() {
        let error = Request::try_from(Message::text(
            r#"{"operation":"synthetic-secret","input":{"pin":"synthetic-secret"}}"#,
        ))
        .unwrap_err();
        assert!(!format!("{error:#}").contains("synthetic-secret"));
    }
    #[test]
    fn protocol_conversions_do_not_emit_raw_data_at_trace_level() {
        let capture = Capture::default();
        let (_diagnostics, subscriber) =
            crate::diagnostics::subscriber("trace", capture.clone()).unwrap();
        tracing::subscriber::with_default(subscriber, || {
            let request = Request::try_from(Message::text(r#"{"operation":"GET_CERTIFICATES","input":{"terminalId":1,"pin":"synthetic-secret"}}"#)).unwrap();
            assert!(matches!(request, Request::GetCertificates(_)));
            let response =
                Response::GetCertificates(ResultResponse::success(responses::GetCertificates {
                    certificates: vec![responses::Certificate {
                        alias: 0,
                        name: "Synthetic Čertificate".into(),
                    }],
                }));
            let message = Message::try_from(response).unwrap();
            assert!(message.to_text().unwrap().contains("Synthetic Čertificate"));
            let response =
                Response::GetSignedXml(ResultResponse::success(responses::GetSignedXml {
                    xml: b"<secret/>".to_vec(),
                }));
            let _ = Message::try_from(response).unwrap();
            let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
            for secret in ["synthetic-secret", "Synthetic Čertificate", "PHNlY3JldC8+"] {
                assert!(!output.contains(secret));
            }
        });
    }

    #[test]
    fn websocket_flow_toggles_masked_tracing_without_loading_nss() {
        struct NoPin;
        impl PinPrompt for NoPin {
            fn prompt_pin(&self, _: &PinInfo, _: String) -> Option<String> {
                panic!("this protocol test must not request a PIN")
            }
        }
        let capture = Capture::default();
        let (diagnostics, subscriber) =
            crate::diagnostics::subscriber("trace", capture.clone()).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = Server::new(ServerConfig { allow_soft_tokens: false }, Box::new(NoPin), LazyCell::new(|| panic!("NSS must remain lazy")), diagnostics.clone(),DesktopController::new(diagnostics.clone()));
            let server_flow = async {
                let (stream, _) = listener.accept().await.unwrap();
                server.run(tokio_tungstenite::accept_async(stream).await.unwrap()).await.unwrap();
            };
            let client_flow = async {
                let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{address}")).await.unwrap();
                let opening = client.next().await.unwrap().unwrap();
                let opening: serde_json::Value = serde_json::from_str(opening.to_text().unwrap()).unwrap();
                assert_eq!(opening["operation"], "ON_OPEN_EVENT");
                let input = r#"{"operation":"GET_INFO","input":{"sbSession":"synthetic-secret","host":"synthetic-host","language":"sr","pin":"synthetic-secret"}}"#;
                client.send(Message::text(input)).await.unwrap();
                let response = client.next().await.unwrap().unwrap();
                let response: serde_json::Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
                assert_eq!(response["status"], 0);
                assert_eq!(server.desktop.snapshot().connections,1);
                assert!(!String::from_utf8_lossy(&capture.0.lock().unwrap()).contains("masked protocol trace"));
                diagnostics.enable(crate::diagnostics::Level::Debug, true, 300).unwrap();
                client.send(Message::text(r#"{"operation":"GET_PROVIDERS"}"#)).await.unwrap();
                let response = client.next().await.unwrap().unwrap();
                let response: serde_json::Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
                assert_eq!(response["payload"]["providers"][0]["name"], "NSS");
                client.send(Message::text(input)).await.unwrap();
                assert!(client.next().await.unwrap().is_ok());
                let traces = String::from_utf8_lossy(&capture.0.lock().unwrap()).matches("masked protocol trace").count();
                assert_eq!(traces, 4);
                diagnostics.disable().unwrap();
                client.send(Message::text(input)).await.unwrap();
                assert!(client.next().await.unwrap().is_ok());
                let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
                assert_eq!(output.matches("masked protocol trace").count(), traces);
                for sensitive in ["synthetic-secret", "synthetic-host"] { assert!(!output.contains(sensitive)); }
                assert!(output.contains("correlation_id"));
                assert!(output.contains("elapsed_ms"));
                client.close(None).await.unwrap();
            };
            tokio::join!(server_flow, client_flow);
            assert_eq!(server.desktop.snapshot().connections,0);
        })
        });
    }
}
