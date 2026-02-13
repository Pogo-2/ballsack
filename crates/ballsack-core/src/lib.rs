//! ballsack-core — shared library for QUIC + SFU-style E2EE group calls.
//!
//! # Architecture (Clean Architecture)
//!
//! - **domain**: protocol types, identifiers, value objects (no I/O).
//! - **application**: use cases + port traits.
//! - **adapters**: QUIC (Quinn), crypto (X25519 + ChaCha20-Poly1305),
//!   media (Opus jitter buffer, video chunking).

pub mod adapters;
pub mod application;
pub mod domain;
