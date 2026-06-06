//! Smart-HTTP **protocol v2** negotiation: capability advertisement, ref
//! advertisement, and `want`/`have` negotiation.
//!
//! Protocol v2 is fixed by contract (not a choice). The flow this module frames:
//!
//! 1. **Capability advertisement** — the server announces `version 2` and the
//!    commands it supports (`ls-refs`, `fetch`) plus object-format.
//! 2. **Ref advertisement** (`ls-refs`) — the server lists refs the client may
//!    `want`. Refs are the **D1 event-log derived view** ([`RefView`]); this
//!    layer reads them, it never holds primary ref state.
//! 3. **want/have negotiation** (`fetch`) — the client sends `want <oid>` for
//!    tips it lacks and `have <oid>` for commits it already holds; the server
//!    transfers only the difference (the spine of delta-only fetch, D2 item ②).
//!
//! pkt-line framing rides `gix-packetline` (a libgit2-class library); the wire
//! format is not reimplemented here.

use std::collections::BTreeMap;

use gix_hash::ObjectId;
use gix_packetline::{PacketLineRef, blocking_io::encode};

use crate::read::pack::parse_oid;

/// The protocol-v2 version token the server advertises.
pub const PROTOCOL_VERSION: &str = "version 2";

/// A read-only view of a repository's refs — the **D1 event-log derived view**.
///
/// The read path negotiates a clone/fetch against this view; it is consumed,
/// never mutated. [`hugit_refstore::RefState`] is the production implementation
/// (see the blanket impl below), so the projection layer reads the exact ref
/// state the event log projects.
pub trait RefView {
    /// Iterate `(ref_name, target_oid_hex)` pairs in canonical (name-sorted)
    /// order.
    fn refs(&self) -> Vec<(String, String)>;
}

impl RefView for hugit_refstore::RefState {
    fn refs(&self) -> Vec<(String, String)> {
        self.iter()
            .map(|(name, target)| (name.to_string(), target.to_string()))
            .collect()
    }
}

/// A simple in-memory [`RefView`] for tests and non-refstore callers.
impl RefView for BTreeMap<String, String> {
    fn refs(&self) -> Vec<(String, String)> {
        self.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

/// The capabilities the server advertises in a protocol-v2 handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// The supported commands (e.g. `ls-refs`, `fetch`).
    pub commands: Vec<String>,
    /// The object format (always `sha1` for this core).
    pub object_format: String,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            commands: vec!["ls-refs".into(), "fetch".into()],
            object_format: "sha1".into(),
        }
    }
}

impl Capabilities {
    /// Encode the protocol-v2 capability advertisement as pkt-lines:
    /// `version 2`, one line per command, `object-format=...`, then flush.
    pub fn encode(&self) -> Result<Vec<u8>, NegotiationError> {
        let mut out = Vec::new();
        write_text(&mut out, PROTOCOL_VERSION)?;
        for cmd in &self.commands {
            write_text(&mut out, &format!("command={cmd}"))?;
        }
        write_text(&mut out, &format!("object-format={}", self.object_format))?;
        encode::flush_to_write(&mut out).map_err(NegotiationError::io)?;
        Ok(out)
    }
}

/// A protocol-v2 ref advertisement (the result of an `ls-refs` command).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefAdvertisement {
    /// `(oid_hex, ref_name)` pairs in canonical (name-sorted) order.
    pub refs: Vec<(String, String)>,
}

impl RefAdvertisement {
    /// Build the advertisement from a [`RefView`] (the D1 derived view).
    pub fn from_view(view: &dyn RefView) -> Self {
        let mut refs: Vec<(String, String)> = view
            .refs()
            .into_iter()
            .map(|(name, oid)| (oid, name))
            .collect();
        refs.sort_by(|a, b| a.1.cmp(&b.1));
        Self { refs }
    }

    /// Encode as pkt-lines: `<oid> <ref>` per ref, then flush. This is the
    /// `ls-refs` response body.
    pub fn encode(&self) -> Result<Vec<u8>, NegotiationError> {
        let mut out = Vec::new();
        for (oid, name) in &self.refs {
            write_text(&mut out, &format!("{oid} {name}"))?;
        }
        encode::flush_to_write(&mut out).map_err(NegotiationError::io)?;
        Ok(out)
    }

    /// The tip oids advertised (what a clone resolves to "want everything").
    pub fn tip_oids(&self) -> Result<Vec<ObjectId>, NegotiationError> {
        self.refs
            .iter()
            .map(|(oid, _)| parse_oid(oid).map_err(|_| NegotiationError::BadOid(oid.clone())))
            .collect()
    }
}

/// The parsed `want`/`have` sets of a protocol-v2 `fetch` request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WantHave {
    /// Object ids the client wants (tips it lacks).
    pub wants: Vec<ObjectId>,
    /// Object ids the client already has (its negotiation frontier).
    pub haves: Vec<ObjectId>,
    /// Whether the client signalled `done` (negotiation may conclude).
    pub done: bool,
}

impl WantHave {
    /// Build a clone request: want every advertised tip, have nothing.
    pub fn clone_all(adv: &RefAdvertisement) -> Result<Self, NegotiationError> {
        Ok(Self {
            wants: adv.tip_oids()?,
            haves: Vec::new(),
            done: true,
        })
    }

    /// Encode this request as protocol-v2 `fetch` command pkt-lines:
    /// `command=fetch`, delimiter, `want <oid>` / `have <oid>` lines, optional
    /// `done`, then flush. This is what a client sends; the server parses it
    /// with [`WantHave::parse`].
    pub fn encode(&self) -> Result<Vec<u8>, NegotiationError> {
        let mut out = Vec::new();
        write_text(&mut out, "command=fetch")?;
        encode::delim_to_write(&mut out).map_err(NegotiationError::io)?;
        for oid in &self.wants {
            write_text(&mut out, &format!("want {oid}"))?;
        }
        for oid in &self.haves {
            write_text(&mut out, &format!("have {oid}"))?;
        }
        if self.done {
            write_text(&mut out, "done")?;
        }
        encode::flush_to_write(&mut out).map_err(NegotiationError::io)?;
        Ok(out)
    }

    /// Parse a protocol-v2 `fetch` request from its pkt-line bytes.
    ///
    /// Recognizes `want <oid>`, `have <oid>`, and `done`; the `command=fetch`
    /// header, delimiter and flush packets are tolerated and skipped.
    pub fn parse(bytes: &[u8]) -> Result<Self, NegotiationError> {
        let mut wh = WantHave::default();
        for line in decode_lines(bytes)? {
            let Some(data) = line_data(&line) else {
                continue; // flush / delim / response-end
            };
            let text = std::str::from_utf8(data)
                .map_err(|_| NegotiationError::Malformed("non-utf8 pkt-line".into()))?
                .trim_end_matches('\n');
            if let Some(oid) = text.strip_prefix("want ") {
                wh.wants
                    .push(parse_oid(oid.trim()).map_err(|_| NegotiationError::BadOid(oid.into()))?);
            } else if let Some(oid) = text.strip_prefix("have ") {
                wh.haves
                    .push(parse_oid(oid.trim()).map_err(|_| NegotiationError::BadOid(oid.into()))?);
            } else if text == "done" {
                wh.done = true;
            }
            // `command=fetch` and any unknown args are ignored (forward-compatible).
        }
        Ok(wh)
    }
}

/// Errors raised during protocol-v2 negotiation.
#[derive(Debug, thiserror::Error)]
pub enum NegotiationError {
    /// pkt-line framing I/O failed.
    #[error("pkt-line io error: {0}")]
    Io(String),
    /// A request was structurally malformed.
    #[error("malformed request: {0}")]
    Malformed(String),
    /// An advertised or requested oid was not a valid git oid.
    #[error("invalid object id: {0}")]
    BadOid(String),
}

impl NegotiationError {
    fn io(e: std::io::Error) -> Self {
        NegotiationError::Io(e.to_string())
    }
}

/// Write one text pkt-line (a trailing newline is added by the encoder).
fn write_text(out: &mut Vec<u8>, text: &str) -> Result<(), NegotiationError> {
    encode::text_to_write(text.as_bytes(), out).map_err(NegotiationError::io)?;
    Ok(())
}

/// Decode all pkt-lines from a buffer into owned variants.
fn line_data(line: &OwnedLine) -> Option<&[u8]> {
    match line {
        OwnedLine::Data(d) => Some(d),
        OwnedLine::Flush | OwnedLine::Delimiter | OwnedLine::ResponseEnd => None,
    }
}

/// An owned pkt-line (decoupled from the borrowing decoder).
enum OwnedLine {
    Data(Vec<u8>),
    Flush,
    Delimiter,
    ResponseEnd,
}

fn decode_lines(mut bytes: &[u8]) -> Result<Vec<OwnedLine>, NegotiationError> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        match gix_packetline::decode::streaming(bytes)
            .map_err(|e| NegotiationError::Malformed(e.to_string()))?
        {
            gix_packetline::decode::Stream::Complete {
                line,
                bytes_consumed,
            } => {
                out.push(match line {
                    PacketLineRef::Data(d) => OwnedLine::Data(d.to_vec()),
                    PacketLineRef::Flush => OwnedLine::Flush,
                    PacketLineRef::Delimiter => OwnedLine::Delimiter,
                    PacketLineRef::ResponseEnd => OwnedLine::ResponseEnd,
                });
                bytes = &bytes[bytes_consumed..];
            }
            gix_packetline::decode::Stream::Incomplete { bytes_needed } => {
                return Err(NegotiationError::Malformed(format!(
                    "truncated pkt-line: {bytes_needed} more bytes needed"
                )));
            }
        }
    }
    Ok(out)
}
