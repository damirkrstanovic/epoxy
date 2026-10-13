use super::{
    cli::{Command, Requests},
    state::{Diagnostics, DiagnosticsStatus},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Component, Path, PathBuf},
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
fn open_directory_at(parent: &fs::File, name: &std::ffi::CStr) -> Result<fs::File> {
    // An owned parent descriptor anchors lookup; O_NOFOLLOW rejects link components.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .context("cannot open nonsymlink runtime directory component");
    }
    // openat returned a new descriptor, transferred exactly once to File ownership.
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}
fn validate_private_directory(directory: &fs::File) -> Result<()> {
    let metadata = directory
        .metadata()
        .context("runtime directory unavailable")?;
    ensure!(
        metadata.uid() == effective_uid() && metadata.mode() & 0o7777 == 0o700,
        "runtime directory must be owned by this user with mode 700"
    );
    Ok(())
}
fn private_dir(path: &Path) -> Result<fs::File> {
    ensure!(
        path.is_absolute(),
        "runtime directory must be an absolute path"
    );
    let mut directory = fs::File::open("/")?;
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => {
                let name = std::ffi::CString::new(name.as_bytes())
                    .context("invalid runtime directory component")?;
                directory = open_directory_at(&directory, &name)?;
            }
            _ => anyhow::bail!("runtime directory must not contain parent traversal"),
        }
    }
    validate_private_directory(&directory)?;
    Ok(directory)
}
fn endpoint(runtime: &Path, create: bool) -> Result<(fs::File, PathBuf)> {
    let root = private_dir(runtime)?;
    if create {
        // mkdirat creates only inside the verified root descriptor, even if ancestors move.
        let result = unsafe { libc::mkdirat(root.as_raw_fd(), c"epoxy".as_ptr(), 0o700) };
        if result != 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            anyhow::bail!("cannot create private diagnostics directory");
        }
    }
    let directory = match open_directory_at(&root, c"epoxy") {
        Ok(directory) => directory,
        Err(error)
            if !create
                && error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            anyhow::bail!("no Epoxy diagnostics endpoint; start Epoxy first");
        }
        Err(error) => return Err(error),
    };
    validate_private_directory(&directory)?;
    // Linux has no bindat: the process-owned /proc descriptor path anchors bind,
    // connect and cleanup to this verified directory. No user symlink is followed.
    let socket = PathBuf::from(format!(
        "/proc/self/fd/{}/control.sock",
        directory.as_raw_fd()
    ));
    Ok((directory, socket))
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

fn remaining(deadline: Instant) -> Result<Duration> {
    let duration = deadline.saturating_duration_since(Instant::now());
    ensure!(!duration.is_zero(), "control deadline exceeded");
    Ok(duration)
}
fn read_deadline(mut stream: &UnixStream, deadline: Instant) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        let available = buffer.len().min(MAX_MESSAGE + 1 - data.len());
        let count = match stream.read(&mut buffer[..available]) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.context("control read failed")?,
        };
        ensure!(count > 0, "incomplete control message");
        let newline = buffer[..count].iter().position(|b| *b == b'\n');
        let consumed = newline.map_or(count, |index| index + 1);
        data.extend_from_slice(&buffer[..consumed]);
        ensure!(data.len() <= MAX_MESSAGE, "oversized control message");
        if newline.is_some() {
            remaining(deadline)?;
            return Ok(data);
        }
    }
}
fn write_deadline(mut stream: &UnixStream, mut bytes: &[u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        let count = match stream.write(bytes) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.context("control write failed")?,
        };
        ensure!(count > 0, "incomplete control write");
        bytes = &bytes[count..];
    }
    remaining(deadline)?;
    Ok(())
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
fn handle(stream: UnixStream, diagnostics: &Diagnostics) -> Result<()> {
    let deadline = Instant::now() + CLIENT_TIMEOUT;
    same_user(peer_uid(&stream)?)?;

    let reply = match read_deadline(&stream, deadline)
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
    write_deadline(&stream, &bytes, deadline)
}

pub struct ControlServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    socket: PathBuf,
    _directory: fs::File,
    identity: (u64, u64),
}
impl ControlServer {
    pub fn start(diagnostics: Diagnostics, runtime: &Path) -> Result<Self> {
        let (directory, socket) = endpoint(runtime, true)?;
        let listener = UnixListener::bind(&socket).context(
            "cannot bind control socket; an existing endpoint is never removed automatically",
        )?;
        let metadata = fs::symlink_metadata(&socket)?;
        let mut server = Self {
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            socket,
            _directory: directory,
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
    let deadline = Instant::now() + Duration::from_secs(2);
    let (_directory, path) = endpoint(runtime, false)?;
    let metadata =
        fs::symlink_metadata(&path).context("no diagnostics endpoint; start Epoxy first")?;
    ensure!(
        metadata.file_type().is_socket()
            && metadata.uid() == effective_uid()
            && metadata.mode() & 0o7777 == 0o600,
        "invalid diagnostics socket owner or permissions"
    );
    let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
    let address = socket2::SockAddr::unix(&path)?;
    socket
        .connect_timeout(&address, remaining(deadline)?)
        .context("cannot connect to running Epoxy within control deadline")?;
    let stream: UnixStream = socket.into();
    same_user(peer_uid(&stream)?)?;

    let mut bytes = serde_json::to_vec(command)?;
    ensure!(bytes.len() < MAX_MESSAGE, "control command too large");
    bytes.push(b'\n');
    write_deadline(&stream, &bytes, deadline)?;
    match serde_json::from_slice::<Reply>(&read_deadline(&stream, deadline)?)
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
        let (diagnostics, subscriber) = subscriber("warn", std::io::sink).unwrap();
        tracing::subscriber::with_default(subscriber, || {
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
            assert!(tracing::enabled!(target: "epoxy::server", tracing::Level::DEBUG));
            // No status getter runs until after checking the actual subscriber filter.
            std::thread::sleep(Duration::from_millis(1250));
            assert!(!tracing::enabled!(target: "epoxy::server", tracing::Level::DEBUG));
            let state = request(&temp.0, &Command::Status {}).unwrap();
            assert!(!state.requests_masked);
            assert_eq!(state.filter, "warn");
        });
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
            let data = read_deadline(&stream, Instant::now() + Duration::from_secs(2)).unwrap();
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

    #[test]
    fn trickling_client_cannot_block_controls_or_automatic_expiry() {
        let temp = Temp::new();
        let (diagnostics, subscriber) = subscriber("warn", std::io::sink).unwrap();
        tracing::subscriber::with_default(subscriber, || {
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
            let mut stream = UnixStream::connect(temp.0.join("epoxy/control.sock")).unwrap();
            stream.write_all(b" ").unwrap();
            let trickle = std::thread::spawn(move || {
                for _ in 0..15 {
                    std::thread::sleep(Duration::from_millis(200));
                    if stream.write_all(b" ").is_err() {
                        break;
                    }
                }
            });
            std::thread::sleep(Duration::from_millis(100));
            let start = Instant::now();
            request(&temp.0, &Command::Status {}).unwrap();
            assert!(start.elapsed() < Duration::from_millis(1200));
            std::thread::sleep(Duration::from_millis(1250));
            assert!(!tracing::enabled!(target: "epoxy::server", tracing::Level::DEBUG));
            trickle.join().unwrap();
        });
    }
    #[test]
    fn saturated_backlog_cannot_hang_cli_connect() {
        use socket2::{Domain, SockAddr, Socket, Type};
        let temp = Temp::new();
        let directory = temp.0.join("epoxy");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("control.sock");
        let address = SockAddr::unix(&path).unwrap();
        let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
        listener.bind(&address).unwrap();
        listener.listen(0).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let mut fillers = Vec::new();
        let mut saturated = false;
        for _ in 0..32 {
            let socket = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
            socket.set_nonblocking(true).unwrap();
            if socket.connect(&address).is_err() {
                saturated = true;
                break;
            }
            fillers.push(socket);
        }
        assert!(saturated);
        let runtime = temp.0.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = sender.send(request(&runtime, &Command::Status {}));
        });
        let response = receiver
            .recv_timeout(Duration::from_millis(2500))
            .expect("CLI connect must be bounded");
        assert!(response.is_err());
        drop(listener);
        worker.join().unwrap();
    }
    #[test]
    fn runtime_paths_reject_intermediate_and_trailing_slash_links() {
        let temp = Temp::new();
        let runtime = temp.0.join("runtime");
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&temp.0, temp.0.join("link")).unwrap();
        std::os::unix::fs::symlink(&runtime, temp.0.join("runtime-link")).unwrap();
        let (diagnostics, _subscriber) = subscriber("info", std::io::sink).unwrap();
        assert!(ControlServer::start(diagnostics.clone(), &temp.0.join("link/runtime")).is_err());
        let trailing = PathBuf::from(format!("{}/runtime-link/", temp.0.display()));
        assert!(ControlServer::start(diagnostics, &trailing).is_err());
    }
    #[test]
    fn directory_replacement_cannot_redirect_cleanup() {
        let temp = Temp::new();
        let (diagnostics, _subscriber) = subscriber("info", std::io::sink).unwrap();
        let server = ControlServer::start(diagnostics, &temp.0).unwrap();
        let original = temp.0.join("epoxy-original");
        fs::rename(temp.0.join("epoxy"), &original).unwrap();
        fs::create_dir(temp.0.join("epoxy")).unwrap();
        fs::write(temp.0.join("epoxy/control.sock"), b"replacement").unwrap();
        drop(server);
        assert!(!original.join("control.sock").exists());
        assert_eq!(
            fs::read(temp.0.join("epoxy/control.sock")).unwrap(),
            b"replacement"
        );
    }

    #[test]
    fn missing_endpoint_explains_that_epoxy_must_be_started() {
        let temp = Temp::new();
        let error = request(&temp.0, &Command::Status {}).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("start Epoxy first"));
        assert!(!message.contains("nonsymlink"));
    }
}
