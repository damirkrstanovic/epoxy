use super::{
    cli::{Command, Requests},
    state::{Diagnostics, DiagnosticsStatus},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::fd::AsRawFd,
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_MESSAGE: usize = 8192;
const CLIENT_TIMEOUT: Duration = Duration::from_millis(500);

fn effective_uid() -> u32 {
    // geteuid has no pointer arguments or failure case.
    unsafe { libc::geteuid() }
}
fn private_dir(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).context("runtime directory unavailable")?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "runtime directory must not be a symlink"
    );
    ensure!(
        metadata.uid() == effective_uid() && metadata.mode() & 0o7777 == 0o700,
        "runtime directory must be owned by this user with mode 700"
    );
    Ok(())
}
fn endpoint(runtime: &Path, create: bool) -> Result<PathBuf> {
    ensure!(
        runtime.is_absolute(),
        "runtime directory must be an absolute path"
    );
    private_dir(runtime)?;
    let directory = runtime.join("epoxy");
    if create {
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => anyhow::bail!("cannot create private diagnostics directory"),
        }
    }
    private_dir(&directory)?;
    Ok(directory.join("control.sock"))
}
fn peer_uid(stream: &UnixStream) -> Result<u32> {
    let mut credential: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // Both pointers refer to correctly sized live buffers; getsockopt fills them.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credential as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    ensure!(
        result == 0 && length as usize == std::mem::size_of::<libc::ucred>(),
        "cannot verify control peer"
    );
    Ok(credential.uid)
}
fn same_user(uid: u32) -> Result<()> {
    ensure!(
        uid == effective_uid(),
        "control peer must be the application user"
    );
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "result", content = "value")]
enum Reply {
    Ok(DiagnosticsStatus),
    Error(String),
}

fn read_line(stream: &UnixStream) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    BufReader::new(stream.take((MAX_MESSAGE + 1) as u64))
        .read_until(b'\n', &mut data)
        .context("control read failed")?;
    ensure!(
        data.len() <= MAX_MESSAGE && data.last() == Some(&b'\n'),
        "invalid or oversized control message"
    );
    Ok(data)
}
fn apply(diagnostics: &Diagnostics, command: Command) -> Result<DiagnosticsStatus> {
    match command {
        Command::Status {} => (),
        Command::Enable {
            level,
            requests,
            ttl,
        } => diagnostics.enable(level, matches!(requests, Requests::Masked), ttl)?,
        Command::Disable {} => diagnostics.disable()?,
    }
    diagnostics.status()
}
fn handle(mut stream: UnixStream, diagnostics: &Diagnostics) -> Result<()> {
    same_user(peer_uid(&stream)?)?;
    stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
    stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
    let reply = match read_line(&stream)
        .ok()
        .and_then(|data| serde_json::from_slice::<Command>(&data).ok())
    {
        Some(command) => match apply(diagnostics, command) {
            Ok(status) => Reply::Ok(status),
            Err(_) => Reply::Error("command rejected or diagnostics unavailable".into()),
        },
        None => Reply::Error("invalid or oversized control message".into()),
    };
    let mut bytes = serde_json::to_vec(&reply)?;
    if bytes.len() >= MAX_MESSAGE {
        bytes = serde_json::to_vec(&Reply::Error("status exceeds control message limit".into()))?;
    }
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    Ok(())
}

pub struct ControlServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    socket: PathBuf,
    identity: (u64, u64),
}
impl ControlServer {
    pub fn start(diagnostics: Diagnostics, runtime: &Path) -> Result<Self> {
        let socket = endpoint(runtime, true)?;
        let listener = UnixListener::bind(&socket).context(
            "cannot bind control socket; an existing endpoint is never removed automatically",
        )?;
        let metadata = fs::symlink_metadata(&socket)?;
        let mut server = Self {
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            socket,
            identity: (metadata.dev(), metadata.ino()),
        };
        fs::set_permissions(&server.socket, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stop = server.stop.clone();
        server.thread = Some(
            thread::Builder::new()
                .name("epoxy-diagnostics".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        if diagnostics.expire(Instant::now()).is_err() {
                            break;
                        }
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let _ = handle(stream, &diagnostics);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(50))
                            }
                            Err(_) => break,
                        }
                    }
                })
                .context("cannot start diagnostics controller")?,
        );
        Ok(server)
    }
}
impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if fs::symlink_metadata(&self.socket).is_ok_and(|m| {
            m.file_type().is_socket()
                && m.uid() == effective_uid()
                && (m.dev(), m.ino()) == self.identity
        }) {
            let _ = fs::remove_file(&self.socket);
        }
    }
}

pub fn runtime_dir() -> Result<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .context("XDG_RUNTIME_DIR is required for private diagnostics controls")
}
pub fn request(runtime: &Path, command: &Command) -> Result<DiagnosticsStatus> {
    let path = endpoint(runtime, false)?;
    let metadata =
        fs::symlink_metadata(&path).context("no diagnostics endpoint; start Epoxy first")?;
    ensure!(
        metadata.file_type().is_socket()
            && metadata.uid() == effective_uid()
            && metadata.mode() & 0o7777 == 0o600,
        "invalid diagnostics socket owner or permissions"
    );
    let mut stream = UnixStream::connect(&path).context("cannot connect to running Epoxy")?;
    same_user(peer_uid(&stream)?)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
    let mut bytes = serde_json::to_vec(command)?;
    ensure!(bytes.len() < MAX_MESSAGE, "control command too large");
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    match serde_json::from_slice::<Reply>(&read_line(&stream)?)
        .map_err(|_| anyhow::anyhow!("invalid control response"))?
    {
        Reply::Ok(status) => Ok(status),
        Reply::Error(_) => anyhow::bail!("diagnostics command rejected"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{
        cli::Requests,
        state::{Level, subscriber},
    };
    use std::{
        fs,
        os::unix::{fs::PermissionsExt, net::UnixStream},
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "epoxy-control-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn control_round_trip_modes_and_clean_shutdown() {
        let temp = Temp::new();
        let (diagnostics, _subscriber) = subscriber("warn", std::io::sink).unwrap();
        let server = ControlServer::start(diagnostics, &temp.0).unwrap();
        let socket = temp.0.join("epoxy/control.sock");
        assert_eq!(
            fs::metadata(temp.0.join("epoxy"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            !request(&temp.0, &Command::Status {})
                .unwrap()
                .requests_masked
        );
        assert!(
            request(
                &temp.0,
                &Command::Enable {
                    level: Level::Debug,
                    requests: Requests::Masked,
                    ttl: 300
                }
            )
            .unwrap()
            .requests_masked
        );
        assert!(
            !request(&temp.0, &Command::Disable {})
                .unwrap()
                .requests_masked
        );
        drop(server);
        assert!(!socket.exists());
    }
    #[test]
    fn refuses_insecure_directory_symlink_and_existing_endpoint() {
        let temp = Temp::new();
        let (diagnostics, _subscriber) = subscriber("info", std::io::sink).unwrap();
        fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ControlServer::start(diagnostics.clone(), &temp.0).is_err());
        fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&temp.0, temp.0.join("epoxy")).unwrap();
        assert!(ControlServer::start(diagnostics.clone(), &temp.0).is_err());
        fs::remove_file(temp.0.join("epoxy")).unwrap();
        fs::create_dir(temp.0.join("epoxy")).unwrap();
        fs::set_permissions(temp.0.join("epoxy"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(temp.0.join("epoxy/control.sock"), b"unrelated").unwrap();
        assert!(ControlServer::start(diagnostics, &temp.0).is_err());
        assert_eq!(
            fs::read(temp.0.join("epoxy/control.sock")).unwrap(),
            b"unrelated"
        );
    }
    #[test]
    fn expiry_and_control_continue_while_other_work_is_blocked() {
        let temp = Temp::new();
        let (diagnostics, _subscriber) = subscriber("warn", std::io::sink).unwrap();
        let _server = ControlServer::start(diagnostics, &temp.0).unwrap();
        request(
            &temp.0,
            &Command::Enable {
                level: Level::Debug,
                requests: Requests::Masked,
                ttl: 1,
            },
        )
        .unwrap();
        // Mimics a synchronous native callback holding the application's execution thread.
        std::thread::sleep(Duration::from_millis(1150));
        let state = request(&temp.0, &Command::Status {}).unwrap();
        assert!(!state.requests_masked);
        assert_eq!(state.filter, "warn");
    }
    #[test]
    fn stalled_client_cannot_block_controls_indefinitely() {
        let temp = Temp::new();
        let (diagnostics, _subscriber) = subscriber("warn", std::io::sink).unwrap();
        let _server = ControlServer::start(diagnostics, &temp.0).unwrap();
        let _stalled = UnixStream::connect(temp.0.join("epoxy/control.sock")).unwrap();
        let start = std::time::Instant::now();
        assert!(
            !request(&temp.0, &Command::Status {})
                .unwrap()
                .requests_masked
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn credential_validator_rejects_another_user() {
        assert!(same_user(effective_uid()).is_ok());
        assert!(same_user(effective_uid().wrapping_add(1)).is_err());
    }
    #[test]
    fn malformed_oversized_and_invalid_commands_never_echo_credentials() {
        let temp = Temp::new();
        let (diagnostics, _subscriber) = subscriber("warn", std::io::sink).unwrap();
        let _server = ControlServer::start(diagnostics, &temp.0).unwrap();
        for input in [
            "{\"command\":\"synthetic-secret\"}\n".to_string(),
            "{\"command\":\"enable\",\"level\":\"debug\",\"requests\":\"masked\",\"ttl\":0}\n"
                .to_string(),
            "{\"command\":\"status\",\"pin\":\"synthetic-secret\"}\n".to_string(),
            format!("{}\n", "synthetic-secret".repeat(600)),
        ] {
            let mut stream = UnixStream::connect(temp.0.join("epoxy/control.sock")).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream.write_all(input.as_bytes()).unwrap();
            let data = read_line(&stream).unwrap();
            assert!(!String::from_utf8_lossy(&data).contains("synthetic-secret"));
            assert!(matches!(
                serde_json::from_slice::<Reply>(&data).unwrap(),
                Reply::Error(_)
            ));
            assert!(
                !request(&temp.0, &Command::Status {})
                    .unwrap()
                    .requests_masked
            );
        }
    }
    #[test]
    fn cleanup_never_deletes_replacement_endpoint() {
        let temp = Temp::new();
        let (diagnostics, _subscriber) = subscriber("info", std::io::sink).unwrap();
        let server = ControlServer::start(diagnostics, &temp.0).unwrap();
        let socket = temp.0.join("epoxy/control.sock");
        fs::remove_file(&socket).unwrap();
        fs::write(&socket, b"replacement").unwrap();
        drop(server);
        assert_eq!(fs::read(&socket).unwrap(), b"replacement");
    }
}
