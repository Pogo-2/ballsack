# RUST VOICE CHAT

**Encrypted group calls that don't phone home.**

I built this because certain unnamed voice/video services decided they needed to know who I'm talking to, when, for how long, and probably what I had for breakfast. So here's a group call app where the server is *literally unable* to eavesdrop -- it just shuffles encrypted blobs between participants like an oblivious mailman.

The relay server sees routing headers (who's in which room, packet ordering) and absolutely nothing else. Your audio, your video, your keys -- all encrypted end-to-end before they ever leave your machine. The server couldn't snoop if it wanted to.

---

## What is this, technically?

A **QUIC-based SFU (Selective Forwarding Unit)** with **true end-to-end encryption**, written in Rust with a Tauri + React desktop frontend.

- **QUIC transport** via [Quinn](https://github.com/quinn-rs/quinn) -- multiplexed, encrypted at the transport layer, with unreliable datagrams for media
- **E2EE media encryption** -- ChaCha20-Poly1305 AEAD on every audio/video packet, keyed with per-sender symmetric keys that the server never sees
- **Key exchange** -- X25519 Diffie-Hellman to seal sender keys per-recipient; Ed25519 identity keys for authentication
- **Key rotation** -- sender keys rotate on membership changes and periodically, for forward secrecy
- **Desktop app** -- Tauri v2 with a React/TypeScript frontend

The server is a dumb relay. It validates room membership, forwards packets, and that's it. It never holds key material, never decrypts media, never stores call content.

---

## How the crypto works (the short version)

Each participant generates:
1. An **Ed25519** signing keypair (identity)
2. An **X25519** keypair (key agreement)
3. A rotating **sender key** (32-byte symmetric secret)

When you join a call, you seal your sender key individually for each other participant using their X25519 public key (ephemeral DH + AEAD). They decrypt it with their private key. The server forwards the sealed blob but can't open it.

Every media packet is then encrypted with your sender key using ChaCha20-Poly1305, with the packet header as authenticated additional data (AAD). The nonce is derived deterministically from the header fields so it's never reused.

When someone joins or leaves, sender keys rotate automatically. The old keys can't decrypt new traffic (forward secrecy-ish), and new members can't decrypt old traffic.

---

## Monorepo structure

```
ballsack/
  Cargo.toml                          # Workspace root
  crates/
    ballsack-core/                    # Shared library -- protocol, crypto, media, QUIC
    ballsack-server/                  # Standalone SFU relay server
    ballsack-desktop/                 # Tauri v2 desktop app
      ui/                             # React + TypeScript + Vite frontend
```

| Crate | Type | What it does |
|-------|------|--------------|
| `ballsack-core` | lib | All the brains -- domain types, use cases, QUIC/crypto/media adapters |
| `ballsack-server` | bin | The dumb relay. Accepts connections, forwards packets, minds its own business |
| `ballsack-desktop` | bin | Desktop app with a real UI. Tauri wraps the core library and talks to React |

## Architecture

The core follows **Clean Architecture** -- dependency arrows point inward, nothing in the inner layers knows about Quinn or Tauri or any framework:

```
Domain (types only)  <--  Application (use cases + traits)  <--  Adapters (Quinn, X25519, Opus, etc.)
```

| Layer | What lives here |
|-------|----------------|
| **Domain** | `ControlMsg` enum, `MediaHeader`, `PeerId`, `RoomId`, `SenderKey` -- pure data, zero I/O |
| **Application** | Use cases (`JoinRoom`, `KeyDistribute`, `SendMedia`, `ReceiveMedia`, etc.) that depend only on port traits |
| **Adapters** | Quinn for QUIC, ChaCha20-Poly1305 + X25519 for crypto, jitter buffers for media |

This means every use case is testable with mocks. The QUIC transport, the crypto backend, the media pipeline -- all swappable.

### Desktop app data flow

```
React UI  -->  invoke("start_call")  -->  Tauri command  -->  JoinRoom use case  -->  QUIC  -->  Server
                                                                                                    |
React UI  <--  listen("peer-joined") <--  app.emit()     <--  AppEvents port    <--  Control  <-----+
```

## Protocol

- **Control plane**: One reliable bidirectional QUIC stream per client, carrying CBOR-encoded messages (`Auth`, `JoinRoom`, `PeerList`, `KeyDistribute`, `KeyRotate`, `StatsReport`, etc.)
- **Media plane**: QUIC DATAGRAM frames (unreliable, unordered) with a fixed binary header the server can read for routing + AEAD ciphertext it can't

### Media datagram wire format

```
[magic 0xBEEF][ver][room_id u64][sender_id u64][kind][ssrc u32][seq u16][timestamp u32][key_id u32][nonce_salt u16][ct_len u16][ciphertext...]
```

All big-endian. Server reads the header to route. Server never touches the ciphertext.

---

## Getting started

### Prerequisites

- **Rust** 1.70+ (stable)
- **Node.js** 18+ and npm (for the desktop frontend)
- No C compiler needed -- uses the `ring` crypto backend

### Build everything

```bash
cargo build --workspace
```

### Run the server

```bash
cargo run -p ballsack-server
```

Listens on `0.0.0.0:4433` by default. It generates a self-signed TLS cert on startup (dev only).

### Run the desktop app

```bash
# Install frontend deps (first time only)
cd crates/ballsack-desktop/ui && npm install && cd ../../..

# Install the Tauri CLI (first time only)
cargo install tauri-cli

# Launch in dev mode
cargo tauri dev -c crates/ballsack-desktop/tauri.conf.json
```

### Run the tests

```bash
cargo test --workspace
```

Tests cover:
- CBOR control message serialization round-trip
- X25519 sender key seal/unseal round-trip
- ChaCha20-Poly1305 media encrypt/decrypt with AAD

### Logging

```bash
RUST_LOG=debug cargo run -p ballsack-server
RUST_LOG=ballsack_core=trace cargo tauri dev -c crates/ballsack-desktop/tauri.conf.json
```

---

## Status

This is a working prototype. The plumbing is all there -- QUIC transport, E2EE key exchange, encrypted media datagrams, jitter buffers, server-side room management and packet forwarding, a desktop app with live peer list and stats. Audio and video capture are currently stubbed (silence frames / dummy data) while the real codec integration is in progress.

### What works today

- QUIC client/server with bidirectional control stream + datagrams
- Full E2EE: key generation, sealed key distribution, media AEAD encrypt/decrypt
- N-way group calls with server-side datagram forwarding
- Video frame chunking and reassembly
- Audio and video jitter buffers with playout clock
- Sender key rotation (periodic + on membership changes)
- Tauri desktop app with React UI (join/leave, peer list, stats panel)
- Stats reporting pipeline

### What's next

- [X] Real audio capture/playback (`cpal` + Opus)
- [ ] Real video capture/encode (libvpx or OpenH264)
- [ ] Ed25519 signatures on key distribution messages
- [ ] Safety number / fingerprint verification UI
- [ ] Bitrate adaptation from QUIC RTT + receiver reports
- [ ] Mute/unmute and device selection
- [ ] Proper certificate handling (replace dev self-signed certs)

---

## Why "ballsack"?

You know why.

## License

MIT -- do whatever you want with it.
