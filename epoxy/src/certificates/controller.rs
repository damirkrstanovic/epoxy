use super::*;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogStatus {
    NotLoaded,
    Loading,
    Ready,
    Busy,
    NoCard,
    NativeUnavailable,
}
pub enum CertificateCommand {
    Refresh,
    Select {
        key: CertificateKey,
        remember: bool,
        generation: u64,
    },
    Clear {
        forget_saved: bool,
    },
}
#[derive(Clone)]
pub struct CertificateSnapshot {
    pub contexts: Vec<PublicContext>,
    pub generation: u64,
    pub revision: u64,
    pub selected: Option<CertificateKey>,
    pub remembered: bool,
    pub status: CatalogStatus,
    pub error: Option<SelectionError>,
}
struct State {
    model: CatalogModel,
    contexts: Vec<PublicContext>,
    status: CatalogStatus,
    error: Option<SelectionError>,
    pending: bool,
    busy: bool,
    closed: bool,
    remembered: bool,
}
#[derive(Clone)]
pub struct CertificateController {
    state: Arc<Mutex<State>>,
    sender: mpsc::Sender<CertificateCommand>,
}
impl CertificateController {
    pub fn new() -> (Self, mpsc::Receiver<CertificateCommand>) {
        let (sender, rx) = mpsc::channel(1);
        (
            Self {
                sender,
                state: Arc::new(Mutex::new(State {
                    model: CatalogModel::default(),
                    contexts: vec![],
                    status: CatalogStatus::NotLoaded,
                    error: None,
                    pending: false,
                    busy: false,
                    closed: false,
                    remembered: false,
                })),
            },
            rx,
        )
    }
    pub fn snapshot(&self) -> CertificateSnapshot {
        let s = self.state.lock().unwrap();
        CertificateSnapshot {
            contexts: s.contexts.clone(),
            generation: s.model.generation(),
            revision: s.model.revision(),
            selected: s.model.selected_key().cloned(),
            remembered: s.remembered,
            status: if s.busy {
                CatalogStatus::Busy
            } else {
                s.status
            },
            error: s.error,
        }
    }
    pub fn request_refresh(&self) -> Result<(), SelectionError> {
        self.enqueue(CertificateCommand::Refresh)
    }
    pub fn request_select(
        &self,
        key: CertificateKey,
        remember: bool,
        generation: u64,
    ) -> Result<(), SelectionError> {
        self.enqueue(CertificateCommand::Select {
            key,
            remember,
            generation,
        })
    }
    pub fn request_clear(&self, forget_saved: bool) -> Result<(), SelectionError> {
        self.enqueue(CertificateCommand::Clear { forget_saved })
    }
    fn enqueue(&self, command: CertificateCommand) -> Result<(), SelectionError> {
        let mut s = self.state.lock().unwrap();
        let result = (|| {
            if s.closed {
                return Err(SelectionError::Unavailable);
            }
            if s.busy {
                return Err(SelectionError::Busy);
            }
            if matches!(command, CertificateCommand::Refresh) && s.pending {
                return Ok(());
            }
            if s.pending {
                return Err(SelectionError::Busy);
            }
            if let CertificateCommand::Select {
                ref key,
                generation,
                ..
            } = command
            {
                if generation != s.model.generation() {
                    return Err(SelectionError::Stale);
                }
                s.model.unique(key)?;
            }
            self.sender
                .try_send(command)
                .map_err(|_| SelectionError::Unavailable)?;
            s.pending = true;
            s.status = CatalogStatus::Loading;
            Ok(())
        })();
        s.error = result.err();
        result
    }
    pub fn publish(&self, contexts: Vec<PublicContext>) {
        let mut s = self.state.lock().unwrap();
        s.model.replace_entries(
            contexts
                .iter()
                .flat_map(|c| c.certificates.clone())
                .collect(),
        );
        s.status = if contexts.is_empty() {
            CatalogStatus::NoCard
        } else {
            CatalogStatus::Ready
        };
        s.contexts = contexts;
        s.pending = false;
        s.error = None;
    }
    pub fn finish(&self, result: Result<(), SelectionError>) {
        let mut s = self.state.lock().unwrap();
        s.pending = false;
        s.error = result.err();
        s.status = if s.contexts.is_empty() {
            CatalogStatus::NotLoaded
        } else {
            CatalogStatus::Ready
        };
    }
    pub fn unavailable(&self) {
        let mut s = self.state.lock().unwrap();
        s.status = CatalogStatus::NativeUnavailable;
        s.pending = false;
        s.error = Some(SelectionError::Unavailable);
    }
    pub fn commit_selection(&self, key: Option<CertificateKey>, remembered: bool) {
        let mut s = self.state.lock().unwrap();
        s.model.set_selection(key);
        s.remembered = remembered;
    }
    /// Called by the serialized native dispatcher after publishing a fresh catalog.
    /// A failed write must leave the previous selection and revision unchanged.
    pub fn commit_choice(
        &self,
        key: CertificateKey,
        remember: bool,
        store: &PreferenceStore,
    ) -> Result<(), SelectionError> {
        let mut s = self.state.lock().unwrap();
        if s.closed {
            return Err(SelectionError::Unavailable);
        }
        s.model.unique(&key)?;
        if remember {
            store.save(&key)?;
        }
        s.model.set_selection(Some(key));
        s.remembered = remember;
        Ok(())
    }
    pub fn clear_choice(
        &self,
        forget_saved: bool,
        store: &PreferenceStore,
    ) -> Result<(), SelectionError> {
        let mut s = self.state.lock().unwrap();
        if s.closed {
            return Err(SelectionError::Unavailable);
        }
        if forget_saved {
            store.forget()?;
        }
        s.model.set_selection(None);
        s.remembered = false;
        Ok(())
    }
    pub fn native_operation(&self) -> NativeOperation {
        self.state.lock().unwrap().busy = true;
        NativeOperation(self.clone())
    }
    pub fn close(&self) {
        let mut s = self.state.lock().unwrap();
        s.closed = true;
        s.pending = false;
        s.status = CatalogStatus::NativeUnavailable;
    }
}
pub struct NativeOperation(CertificateController);
impl Drop for NativeOperation {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().busy = false;
    }
}

impl CertificateSnapshot {
    pub fn authorize_alias(
        &self,
        key: &CertificateKey,
        revision: u64,
    ) -> Result<(), SelectionError> {
        if revision != self.revision {
            return Err(SelectionError::Stale);
        }
        if let Some(selected) = &self.selected {
            if selected != key {
                return Err(SelectionError::Missing);
            }
        }
        Ok(())
    }
}
