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
use epoxy::certificates::{
    CatalogModel, CertificateKey, ContextKey, Eligibility, PublicCertificate, SelectionError,
};
use futures::{SinkExt, StreamExt, TryStreamExt, future};
use libxml::{
    parser::{Parser, ParserOptions, XmlParseError},
    tree::Document,
};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tracing::{info, warn};
use tungstenite::{Message, Utf8Bytes};

use crate::{
    nss::{Nss, PinContext, PinInfo, PinPrompt, SecItem},
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

#[derive(Default)]
pub struct Session {
    terminals: Vec<ContextKey>,
    certs: std::collections::BTreeMap<usize, CertificateRecord>,
    next_alias: usize,
}
struct CertificateRecord {
    key: CertificateKey,
    der: SecItem,
    label: String,
    revision: u64,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub allow_soft_tokens: bool,
    pub selection_path: std::path::PathBuf,
}

pub struct Server<F> {
    config: ServerConfig,
    diagnostics: Diagnostics,
    desktop: DesktopController,
    pin_prompt: Box<dyn PinPrompt>,
    nss_xmlsec: LazyCell<Result<(Nss, XmlSec)>, F>,
    xml_parser: Parser,
}

impl<F> Server<F>
where
    F: FnOnce() -> Result<(Nss, XmlSec)>,
{
    pub fn new(
        config: ServerConfig,
        pin_prompt: Box<dyn PinPrompt>,
        nss_xmlsec: LazyCell<Result<(Nss, XmlSec)>, F>,
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

    pub fn certificate_command(&self, command: epoxy::certificates::CertificateCommand) {
        use epoxy::certificates::{CertificateCommand, native::public_catalog};
        let controller = self.desktop.certificates();
        if self.desktop.shutdown_requested().is_cancelled() {
            controller.close();
            return;
        }
        let _busy = controller.native_operation();
        let Ok((nss, _)) = &*self.nss_xmlsec else {
            controller.unavailable();
            return;
        };
        match command {
            CertificateCommand::Refresh => match public_catalog(nss, self.config.allow_soft_tokens)
            {
                Ok(contexts) => controller.publish(contexts),
                Err(_) => controller.unavailable(),
            },
            CertificateCommand::Select {
                key,
                remember,
                generation,
            } => {
                let result = (|| {
                    if controller.snapshot().generation != generation {
                        return Err(SelectionError::Stale);
                    }
                    let contexts = public_catalog(nss, self.config.allow_soft_tokens)?;
                    let model = CatalogModel::from_entries(
                        contexts
                            .iter()
                            .flat_map(|c| c.certificates.clone())
                            .collect(),
                    );
                    model.unique(&key)?;
                    controller.publish(contexts);
                    controller.commit_choice(
                        key,
                        remember,
                        &epoxy::certificates::PreferenceStore::new(
                            self.config.selection_path.clone(),
                        ),
                    )?;
                    Ok(())
                })();
                controller.finish(result);
            }
            CertificateCommand::Clear { forget_saved } => {
                let result = controller.clear_choice(
                    forget_saved,
                    &epoxy::certificates::PreferenceStore::new(self.config.selection_path.clone()),
                );
                controller.finish(result);
            }
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
            let _native = self.desktop.certificates().native_operation();
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
        let Ok((nss, _)) = &*self.nss_xmlsec else {
            return ResultResponse::error(responses::GetTerminalsErrorCode::GenericError);
        };

        let tokens = nss.get_tokens();
        let Some(tokens) = tokens else {
            return ResultResponse::error(responses::GetTerminalsErrorCode::GenericError);
        };

        let selection = self.desktop.certificates().snapshot().selected;
        let mut valid_tokens = Vec::new();
        for token in tokens
            .iter()
            .filter(|t| (self.config.allow_soft_tokens || t.is_hw()) && !t.is_internal())
        {
            let Ok(context) = token.context_key() else {
                continue;
            };
            if selection.as_ref().is_none_or(|s| s.context == context) {
                valid_tokens.push((context, token.name()));
            }
        }
        if selection.is_some() && valid_tokens.len() != 1 {
            valid_tokens.clear();
        }
        session.terminals.clear();
        let mut terminals = Vec::new();
        for (index, (key, name)) in valid_tokens.into_iter().enumerate() {
            session.terminals.push(key);
            terminals.push(responses::Terminal { id: index, name });
        }

        ResultResponse::success(responses::GetTerminals { terminals })
    }

    fn get_certificates(
        &self,
        session: &mut Session,
        request: requests::GetCertificates,
    ) -> ResultResponse<responses::GetCertificates, responses::GetCertificatesErrorCode> {
        let Ok((nss, _)) = &*self.nss_xmlsec else {
            return ResultResponse::error(responses::GetCertificatesErrorCode::GenericError);
        };

        session.certs.clear();
        let snapshot = self.desktop.certificates().snapshot();
        let context_key = session.terminals.get(request.terminal_id);
        let Some(context_key) = context_key else {
            return ResultResponse::error(responses::GetCertificatesErrorCode::GenericError);
        };

        if snapshot
            .selected
            .as_ref()
            .is_some_and(|s| &s.context != context_key)
        {
            return ResultResponse::success(responses::GetCertificates {
                certificates: vec![],
            });
        }
        let Ok(token) = nss.find_context(context_key) else {
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

        let mut valid_certs = Vec::new();
        for cert in certs.iter() {
            let Ok(der) = cert.der() else {
                continue;
            };
            let Ok(metadata) = PublicCertificate::from_der(context_key.clone(), der.as_ref())
            else {
                continue;
            };
            if metadata.eligibility != Eligibility::Eligible {
                continue;
            }
            if snapshot
                .selected
                .as_ref()
                .is_none_or(|s| s == &metadata.key)
            {
                valid_certs.push((der, metadata));
            }
        }
        if snapshot.selected.is_some() && valid_certs.len() != 1 {
            valid_certs.clear();
        }
        let mut certificates = Vec::new();
        for (der, metadata) in valid_certs {
            let alias = session.next_alias;
            let Some(next) = alias.checked_add(1) else {
                return ResultResponse::error(responses::GetCertificatesErrorCode::GenericError);
            };
            session.next_alias = next;
            certificates.push(responses::Certificate {
                alias,
                name: metadata.holder.clone(),
            });
            session.certs.insert(
                alias,
                CertificateRecord {
                    key: metadata.key,
                    der,
                    label: metadata.holder,
                    revision: snapshot.revision,
                },
            );
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

        let Some(record) = session.certs.get(&request.certificate.alias) else {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        };
        let snapshot = self.desktop.certificates().snapshot();
        if snapshot
            .authorize_alias(&record.key, record.revision)
            .is_err()
        {
            warn!(
                error_kind = "stale_certificate_selection",
                "Reload or reselect certificates in the portal"
            );
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        }
        let Ok((nss, xmlsec)) = &*self.nss_xmlsec else {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        };
        let Ok(token) = nss.find_context(&record.key.context) else {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        };
        let Ok(metadata) =
            PublicCertificate::from_der(record.key.context.clone(), record.der.as_ref())
        else {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        };
        if metadata.key != record.key || metadata.eligibility != Eligibility::Eligible {
            return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
        }

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
            let pin_context = self.get_pin_context(Some(record.label.clone()), reason);
            let Ok(_logout_guard) = nss.authenticate(&pin_context, &token) else {
                warn!(
                    error_kind = "authentication_failed",
                    "token authentication failed"
                );
                return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
            };

            // Re-read from the selected live slot after authentication. A global
            // certificate cache must not keep a removed/replaced card eligible.
            let Some(live_certs) = nss.list_certificates(&token) else {
                return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
            };
            let live = CatalogModel::from_entries(
                live_certs
                    .iter()
                    .filter_map(|c| {
                        PublicCertificate::from_der(
                            record.key.context.clone(),
                            c.der().ok()?.as_ref(),
                        )
                        .ok()
                    })
                    .collect(),
            );
            if live.unique(&record.key).is_err() {
                return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
            }

            let Ok(signer) = XmlSigner::with_cert(xmlsec, &record.der) else {
                return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
            };

            if !signer.matches_token(&token) {
                warn!(
                    error_kind = "key_context_mismatch",
                    "Signing key does not belong to the selected PIN context"
                );
                return ResultResponse::error(responses::GetSignedXmlErrorCode::GenericError);
            }
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
    fn public_catalog_runs_on_native_thread_and_controls_remain_responsive() {
        use epoxy::certificates::{
            CatalogStatus, CertificateCommand, CertificateController, SelectionError,
        };
        let (catalog, mut rx) = CertificateController::new();
        catalog.request_refresh().unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker_catalog = catalog.clone();
        let worker = std::thread::spawn(move || {
            struct NoPin;
            impl PinPrompt for NoPin {
                fn prompt_pin(&self, _: &PinInfo, _: String) -> Option<String> {
                    panic!("public catalog must not prompt")
                }
            }
            let native_thread = std::thread::current().id();
            let (diagnostics, _subscriber) =
                crate::diagnostics::subscriber("info", std::io::sink).unwrap();
            let desktop = DesktopController::with_certificates(diagnostics.clone(), worker_catalog);
            let server = Server::new(
                ServerConfig {
                    allow_soft_tokens: false,
                    selection_path: "/synthetic/unused/selection.json".into(),
                },
                Box::new(NoPin),
                LazyCell::new(move || {
                    started_tx.send(std::thread::current().id()).unwrap();
                    release_rx.recv().unwrap();
                    Err(anyhow!("synthetic unavailable backend"))
                }),
                diagnostics,
                desktop,
            );
            assert!(matches!(rx.try_recv(), Ok(CertificateCommand::Refresh)));
            server.certificate_command(CertificateCommand::Refresh);
            native_thread
        });
        let executing_thread = started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(catalog.snapshot().status, CatalogStatus::Busy);
        assert_eq!(catalog.request_clear(false), Err(SelectionError::Busy));
        release_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), executing_thread);
        assert_eq!(catalog.snapshot().status, CatalogStatus::NativeUnavailable);
    }
    #[test]
    fn changed_selection_rejects_a_real_session_alias_before_any_pin() {
        struct NoPin;
        impl PinPrompt for NoPin {
            fn prompt_pin(&self, _: &PinInfo, _: String) -> Option<String> {
                panic!("stale alias must not prompt")
            }
        }
        let (catalog, _) = epoxy::certificates::CertificateController::new();
        let (diagnostics, _subscriber) =
            crate::diagnostics::subscriber("info", std::io::sink).unwrap();
        let desktop = DesktopController::with_certificates(diagnostics.clone(), catalog.clone());
        let server = Server::new(
            ServerConfig {
                allow_soft_tokens: false,
                selection_path: "/synthetic/unused/selection.json".into(),
            },
            Box::new(NoPin),
            LazyCell::new(|| panic!("stale alias must not initialize native libraries")),
            diagnostics,
            desktop,
        );
        let context = ContextKey {
            module_name: "Synthetic module".into(),
            module_library: None,
            token_uri: b"synthetic:a".to_vec(),
        };
        let bytes = include_bytes!("../tests/fixtures/eligible.der").to_vec();
        let key = PublicCertificate::from_der(context, &bytes).unwrap().key;
        let mut session = Session::default();
        session.certs.insert(
            17,
            CertificateRecord {
                key: key.clone(),
                der: SecItem::from_der_bytes(bytes).unwrap(),
                label: "Synthetic".into(),
                revision: 0,
            },
        );
        catalog.commit_selection(Some(key), false);
        let response = server.get_signed_xml(
            &session,
            requests::GetSignedXml {
                certificate: requests::Certificate {
                    alias: 17,
                    name: "Synthetic".into(),
                },
                pin: String::new(),
                xml: b"<synthetic/>".to_vec(),
            },
        );
        assert!(matches!(response, ResultResponse::Error(_)));
    }
    #[test]
    fn get_info_does_not_initialize_native_libraries() {
        struct NoPin;
        impl PinPrompt for NoPin {
            fn prompt_pin(&self, _: &PinInfo, _: String) -> Option<String> {
                panic!("information does not prompt")
            }
        }
        let (diagnostics, _subscriber) =
            crate::diagnostics::subscriber("info", std::io::sink).unwrap();
        let desktop = DesktopController::new(diagnostics.clone());
        let server = Server::new(
            ServerConfig {
                allow_soft_tokens: false,
                selection_path: "/synthetic/unused/selection.json".into(),
            },
            Box::new(NoPin),
            LazyCell::new(|| panic!("information does not initialize NSS")),
            diagnostics,
            desktop,
        );
        assert!(matches!(server.get_info(), ResultResponse::Success(_)));
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
                selection_path: "/synthetic/unused/selection.json".into(),
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
