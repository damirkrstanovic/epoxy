# Website access and transport limits

The signing WebSocket accepts these exact browser Origins:

- `https://eporezi.purs.gov.rs`
- `https://test.purs.gov.rs`

Other, missing, duplicate and `null` Origin headers receive HTTP 403 before
any Epoxy protocol event or native certificate request. Paths, alternate ports,
HTTP and lookalike domains are not accepted. The JSON `GET_INFO.host` value is
untrusted display metadata, not an authorization check.

The configured endpoint must resolve entirely to loopback addresses. Inherited
systemd listeners must be listening TCP sockets bound to loopback; one descriptor
is supported. IPv4, IPv6 and IPv4-mapped IPv6 loopback are recognized. Public,
LAN and wildcard listeners are rejected before binding. Peer addresses are also
checked before the upgrade.

## Developer clients

For an intentional command-line client that cannot send an Origin, explicitly
opt in for that run:

```sh
epoxy --allow-missing-origin
```

Alternatively use `allow_missing_origin = true` in the TOML configuration.
`--allow-missing-origin=false` overrides a configured exception. The exception
admits only absent headers: a present unapproved Origin remains rejected. Normal
portal use and the local browser test launcher do not need this exception.

An Origin check protects the browser boundary. It does not authenticate local
processes: a non-browser client can forge an allowed Origin, as explained in
[RFC 6455 section 10.2](https://www.rfc-editor.org/rfc/rfc6455.html#section-10.2).
This change does not add a separate approval prompt for every signature or stop
an allowed page from handling a PIN entered on that page. The existing policy
uses a nonempty supplied PIN only for its request, otherwise asking natively;
it does not cache PINs.

## Resource limits

| Network operation | Limit |
| --- | --- |
| WebSocket HTTP upgrade | 5 seconds |
| First GET_INFO after ON_OPEN_EVENT is sent | 10 seconds |
| Waiting for the next application request | 600 seconds |
| Sending a response or closing the socket | 5 seconds |
| Incoming frame and aggregate message | 16 MiB each |
| WebSocket write buffer | 32 MiB |

The encoded JSON request counts toward the message limit. Base64 XML has
roughly 12 MiB available before other request fields. Tungstenite's existing
HTTP-header and handshake packet limits remain enabled. `max_connections` must
be greater than zero and retains its default of 10.

Network deadlines apply to network waits only. Native NSS/XMLSec work and PIN
callbacks are synchronous and are not interrupted by a whole-operation timer.
Blocking native work may defer when the runtime observes network deadlines.
These are transport bounds, not a hard execution or total-memory sandbox.

Ping/Pong frames remain supported; requests must be text. Binary requests are
rejected without echoing their contents. Upgrade rejection logs use fixed
categories such as `origin_unapproved` and `websocket_upgrade_timeout`, never
raw Origin values. Optional detailed protocol logging retains sensitive-data
masking and remains off by default.

## Verification and manual acceptance

Automated tests use temporary loopback sockets, isolated displays and private
D-Bus/NSS fixtures. They cover Origin rejection, the absent-only exception,
loopback listeners, incomplete/oversized upgrades, frame/fragment limits,
initial/idle/write deadlines, Ping/Pong and Close, binary rejection and lazy
GET_INFO. Desktop smoke clients supply an approved Origin with the default guard.

Real production and test portal login/signing still need user-operated acceptance
on this build. Check unusually large declarations and reconnect behavior after
10 minutes without a request. Neither the real token nor tax data is accessed
by the automated fixtures. If a real portal fails to connect, enable operational
debug logs and inspect the fixed rejection category before changing policy;
never paste a PIN or an unmasked payload into a report.
