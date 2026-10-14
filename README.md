# epoxy

SmartBox-compatible open source tool for signing ePorezi tax forms.

The default desktop build includes native GTK controls and a system tray. Use
`epoxy --show-window` to open the controls immediately. See [desktop controls](epoxy/docs/desktop.md)
for tray fallback, PIN cancellation, diagnostics and verification.

## DISCLAIMER

This tool is provided as-is under the MIT License. Please note:

* This software is provided without warranty of any kind. Project contributors are not responsible for any tax liabilities, penalties, damages or other consequences that may arise from using this tool.

* This project is not affiliated with the Serbian Tax Administration (Poreska Uprava).

## Quick start

### Arch Linux

```bash
# Install PKCS#11 module(s) - example for Serbian ID card
paru -Syu srb-id-pkcs11-git
sudo systemctl enable pcscd.socket
sudo systemctl start pcscd.socket

# Add p11-kit proxy module to NSS
modutil -dbdir $HOME/.pki/nssdb -add "p11-kit" -libfile /usr/lib/libp11-kit.so -mechanisms FRIENDLY
modutil -dbdir $HOME/.pki/nssdb -list

# Install epoxy
paru -Syu epoxy-git
systemctl --user enable epoxy.socket
systemctl --user start epoxy.socket
```

## Build

```bash
# All features
cargo build

# Without GTK4, systemd support
cargo build --no-default-features
```

## Setup

Only Linux systems are currently supported.

### Install library dependencies

- `libnss3`
- `libxml2`
- `libxmlsec1`
- `gtk4` 4.4 or newer (for `ui-gtk` / default tray builds)

### Install PKCS#11 module

Install the module for your token.

#### 1. Serbian ID card

Unofficial module setup guide: https://github.com/ubavic/srb-id-pkcs11/blob/main/README.md

### Add module to NSS database

This step is required on Arch but can be optional on other distros. Some distros already come with NSS + p11-kit preconfigured.

`-mechanisms FRIENDLY` tells NSS not to prompt for PIN when querying certificates. It shouldn't be used if your token does not allow passwordless access to public objects.

**Option 1.** Add the PKCS#11 module directly

```bash
modutil -dbdir $HOME/.pki/nssdb -add "srb-id-pkcs11" -libfile /usr/lib/pkcs11/libsrb-id-pkcs11.so -mechanisms FRIENDLY
```

**Option 2.** Use p11-kit proxy
```bash
modutil -dbdir $HOME/.pki/nssdb -add "p11-kit" -libfile /usr/lib/libp11-kit.so -mechanisms FRIENDLY
```

## Usage

Run with default settings:

```bash
epoxy
```

Print help:

```bash
epoxy --help
```


### Systemd service

Systemd unit files can be found [here](epoxy/systemd).

```
systemctl --user enable epoxy.socket
systemctl --user start epoxy.socket
```

### ePorezi login and signing

For testing, use the [ePorezi test environment](https://test.purs.gov.rs). The OLZ tax form is the simplest to start with.

Epoxy uses a nonempty page-provided PIN once for that request; otherwise it asks
through its native prompt. Entering a PIN on the page exposes it to the page's
scripts. Where the portal permits an empty PIN, leave it empty to use native
entry. Epoxy does not cache PINs between requests. See [PIN behavior](epoxy/docs/request-pins.md).

![demo](assets/demo.gif)

## Website access

By default, the signing WebSocket accepts only the production and test ePorezi
website origins and listens only on loopback. See [website access and transport
limits](epoxy/docs/browser-boundary.md) for developer access and troubleshooting.

## Runtime diagnostics

Operational logging omits credentials and certificate identity. Detailed masked
request/response tracing is off by default and can be changed without restarting:

```sh
epoxy diagnostics enable --level debug --requests masked --ttl 300
epoxy diagnostics status
epoxy diagnostics disable
```

See [runtime diagnostics](epoxy/docs/diagnostics.md) for masking, expiry, the
required private `XDG_RUNTIME_DIR`, and troubleshooting.

## Security and licensing

Epoxy disables ordinary core dumps/debug attachment at startup and wipes its
owned PIN buffers after use or cancellation. See [process protection](epoxy/docs/process-protection.md)
and [PIN memory lifetime and limits](epoxy/docs/pin-memory.md).

See the [security review and trust boundaries](epoxy/docs/security-review.md)
and [third-party licensing / distribution obligations](THIRD_PARTY.md).
