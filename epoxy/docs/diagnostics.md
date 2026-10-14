# Runtime diagnostics

Epoxy logs operational events to stderr at `info` by default: operation name,
status, duration and an application-generated correlation ID. It omits PINs,
portal sessions, certificate identities and document contents. `--log debug`
and `--log trace` change operational verbosity but do **not** enable payload
tracing.

## Control the running instance

```sh
epoxy diagnostics status
epoxy diagnostics enable --level debug --requests masked --ttl 300
epoxy diagnostics disable
```

Start Epoxy before using these commands. If the control directory is absent,
the CLI reports `no Epoxy diagnostics endpoint; start Epoxy first`.

These commands contact the running Epoxy process. They do not start another
signing server, initialize NSS or open a PIN dialog. They also work while a
native PIN prompt is waiting. Later tray controls can use the same controller.

`enable` accepts `error`, `warn`, `info`, `debug` and `trace`, with a lifetime of
1–3600 seconds (default 300). `--requests` accepts `off` (default) or `masked`.
Masked wire events have their own target, `epoxy::diagnostics`, enabled at info
when requested; choosing an error/warn operational level still allows explicitly
requested wire events. Enabling again replaces the expiry deadline. Disabling
or expiry restores the original startup filter and turns payload tracing off.
Restarting always starts with payload tracing off.

## What a masked trace contains

A masked trace preserves known protocol operations, numeric status/IDs and
known container structure at approved locations for that operation and
request/response shape. A familiar field name in an unknown location is masked. It replaces PINs, session credentials, identity and
unknown fields/values. XML is represented by its encoded byte length, not its
contents. Unknown JSON field names are replaced by a count so credentials in a
field name cannot escape masking. No unmasked mode exists.

Malformed JSON logs omission metadata only. Input over 64 KiB, nesting deeper
than 16, more than 128 visited JSON nodes, or sanitized output over 16 KiB is
omitted rather than dumped. Raw input and parser error snippets are never the
fallback. Wire logging happens before request decoding discards the browser
PIN field; the PIN remains ignored for card authentication.

Operational error messages use fixed categories and, for NSS authentication
failures, numeric native error/status codes. Native dialogs may display the
selected certificate and signing purpose; those details do not enter operation
logs. Detailed native errors and qualified-certificate compatibility still need
backend-specific investigation; generic portal error codes remain compatible.

## Private control socket

The service requires `XDG_RUNTIME_DIR` to identify an absolute, nonsymlink
runtime directory owned by the application user with permissions 700. Epoxy
creates `XDG_RUNTIME_DIR/epoxy` with mode 700 and `control.sock` with mode 600.
Every runtime path component is opened with no-follow directory traversal.
Creation, binding, connection and cleanup remain anchored to verified directory
descriptors through Linux `/proc/self/fd`; a mounted procfs is required.
Both client and server verify Linux socket peer credentials. Controls are not
available through the browser WebSocket, and no shared `/tmp` fallback exists.

Control messages are limited to 8 KiB. A stalled control client has a 500 ms
absolute deadline for the entire connection, including reads and writes;
trickling bytes cannot restart it. CLI connect, command write and response read
share an absolute two-second deadline. The independent
control thread checks expiry every 50 ms when idle, with bounded additional
delay while handling a client. Application native signing/PIN waits do not
block that thread.

Startup collision errors name the actual diagnostics Unix socket, not the portal
TCP port. A bounded same-user peer check reports the listener PID when available.
Quit that instance from its tray/launching terminal, or stop its user service, then
retry. Inspect Unix sockets with `ss -xlpn` (filter on `control.sock`), rather than
searching for TCP port 17165.

If a verified private socket refuses connections, the error explains that no
process is listening and gives stale-file recovery guidance. With Epoxy stopped,
remove only `$XDG_RUNTIME_DIR/epoxy/control.sock` before restarting. If the endpoint
cannot be verified, startup gives inspection guidance without recommending removal.
The probe sends no commands, changes no diagnostic policy and has a 200 ms bound.

An existing endpoint is never deleted automatically at startup. Normally the
socket is removed on shutdown. After an abnormal exit, first confirm that no
Epoxy instance is running; then inspect the owner/type of the private socket
and remove that specific stale socket before restarting. An active instance or
an unrelated endpoint must not be removed. Missing runtime directories,
unsafe permissions, inaccessible sockets and unsupported control options
produce errors instead of silently enabling another service.

## Capture logs

For a systemd user service:

```sh
journalctl --user -u epoxy.service -f
```

For a foreground instance, use stderr redirection to a private file:

```sh
umask 077
epoxy --pin-prompt tty 2>epoxy-diagnostics.log
```

Enable masked tracing from a second terminal, reproduce the issue, then disable
it. Examine logs before sharing them. This feature adds no file sink; retention
of redirected files or journal entries is controlled by you/the host.

Application masking does not sanitize operating-system core dumps or independent
native-library stderr/debug output. Do not enable PKCS#11/native-library raw
debug environments for ordinary operation. No PIN caching or convenience
session retention is added by diagnostics.
