//! Parse a smart-HTTP `git-receive-pack` (push) request body — the WRITE-side
//! wire glue (WP1 of the receive-pack wave; see
//! `docs/plan/2026-06-22-receive-pack-wave-design.md`).
//!
//! This module is **pure and fail-closed**: no I/O, no CAS, no event log, and NO
//! behaviour change to the serve handler (W2 wires it in). It only turns the raw
//! POST body a `git` client sends into structured commands + the packfile bytes.
//!
//! The body is git's receive-pack request framing:
//!   * a list of pkt-line **commands**, each `<old-oid> SP <new-oid> SP <ref>`,
//!     where the FIRST command carries a trailing `NUL <space-separated caps>`;
//!   * a flush-pkt (`0000`) terminating the command list;
//!   * the raw **packfile** bytes (a `PACK…` stream) — EMPTY for a delete-only push.
//!
//! It mirrors the manual pkt-line decode in [`crate::git`] (a 4-hex big-endian
//! length prefix that INCLUDES the 4 prefix bytes, `0000` = flush) rather than
//! pulling a library, to match the existing wire code and keep the decode
//! auditable.

/// The all-zero oid git uses to mean "no such tip": an `old_oid` of all-zero is a
/// ref **create**; a `new_oid` of all-zero is a ref **delete**.
pub const ZERO_OID: &str = "0000000000000000000000000000000000000000";

/// One ref-mutation command a push asks to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveCommand {
    /// The tip the pusher believes the ref holds now — 40-hex, or [`ZERO_OID`] for
    /// a create (the ref is asserted absent).
    pub old_oid: String,
    /// The tip the ref should hold after the push — 40-hex, or [`ZERO_OID`] for a
    /// delete.
    pub new_oid: String,
    /// The full ref name, e.g. `refs/heads/main`.
    pub ref_name: String,
}

impl ReceiveCommand {
    /// A create — the ref is asserted to not exist yet.
    #[must_use]
    pub fn is_create(&self) -> bool {
        self.old_oid == ZERO_OID
    }
    /// A delete — the ref should be removed (no pack bytes are required for it).
    #[must_use]
    pub fn is_delete(&self) -> bool {
        self.new_oid == ZERO_OID
    }
}

/// A parsed receive-pack request: the commands, the capabilities the client
/// advertised (from the first command), and the raw packfile that followed the
/// flush (empty for a delete-only push).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveRequestWire {
    /// The ref commands, in wire order (≥1; a body that flushes with no command is
    /// [`RecvParseError::NoCommands`]).
    pub commands: Vec<ReceiveCommand>,
    /// The capability tokens the client sent after the NUL on the first command.
    pub capabilities: Vec<String>,
    /// The packfile bytes after the command-list flush. Empty iff every command is
    /// a delete; a non-delete with no pack is rejected by [`Self::require_pack`].
    pub pack: Vec<u8>,
}

impl ReceiveRequestWire {
    /// True if at least one command needs objects (i.e. is not a pure delete).
    #[must_use]
    pub fn needs_pack(&self) -> bool {
        self.commands.iter().any(|c| !c.is_delete())
    }

    /// Fail-closed check that a non-delete push actually delivered a packfile that
    /// starts with the git `PACK` magic. (A create/update with no pack — or junk
    /// where the pack should be — is a malformed push, not a silent no-op.)
    pub fn require_pack(&self) -> Result<(), RecvParseError> {
        if self.needs_pack() && !self.pack.starts_with(b"PACK") {
            return Err(RecvParseError::MissingPack);
        }
        Ok(())
    }
}

/// Why a receive-pack body could not be parsed. Every variant is fatal — the
/// caller commits nothing and answers a fail-closed error to the client.
#[derive(Debug, PartialEq, Eq)]
pub enum RecvParseError {
    /// A pkt-line length prefix is not 4 ASCII hex digits.
    BadLengthPrefix,
    /// A pkt-line claims more bytes than the body holds (truncated stream).
    Truncated,
    /// The command list flushed with zero commands — nothing to apply.
    NoCommands,
    /// A command line was not `<old-oid> SP <new-oid> SP <ref>` with valid oids.
    BadCommand(String),
    /// A non-delete push carried no `PACK…` stream after the flush.
    MissingPack,
}

impl std::fmt::Display for RecvParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadLengthPrefix => write!(f, "malformed pkt-line length prefix"),
            Self::Truncated => write!(f, "truncated pkt-line stream"),
            Self::NoCommands => write!(f, "no ref commands in the push"),
            Self::BadCommand(s) => write!(f, "malformed ref command: {s}"),
            Self::MissingPack => write!(f, "push delivered no packfile for a non-delete update"),
        }
    }
}

/// True iff `s` is exactly 40 lowercase hex digits (a git SHA-1 oid). [`ZERO_OID`]
/// satisfies this, so creates/deletes parse.
fn is_sha1_hex(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Parse one command payload `<old> SP <new> SP <ref>` (caps already stripped),
/// tolerating a trailing `\n`.
fn parse_command(payload: &[u8]) -> Result<ReceiveCommand, RecvParseError> {
    let text = std::str::from_utf8(payload)
        .map_err(|_| RecvParseError::BadCommand("non-UTF-8 command".to_string()))?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let mut parts = text.splitn(3, ' ');
    let (Some(old), Some(new), Some(ref_name)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(RecvParseError::BadCommand(format!(
            "expected '<old> <new> <ref>', got {text:?}"
        )));
    };
    if !is_sha1_hex(old) || !is_sha1_hex(new) {
        return Err(RecvParseError::BadCommand(format!("bad oid in {text:?}")));
    }
    if ref_name.is_empty() || !ref_name.starts_with("refs/") {
        return Err(RecvParseError::BadCommand(format!(
            "bad ref name in {text:?}"
        )));
    }
    Ok(ReceiveCommand {
        old_oid: old.to_string(),
        new_oid: new.to_string(),
        ref_name: ref_name.to_string(),
    })
}

/// Parse a full receive-pack request body into commands, capabilities, and the
/// trailing packfile. Fail-closed on any framing or command error.
pub fn parse_receive_pack_body(body: &[u8]) -> Result<ReceiveRequestWire, RecvParseError> {
    let mut commands = Vec::new();
    let mut capabilities = Vec::new();
    let mut i = 0usize;

    loop {
        if i + 4 > body.len() {
            // Ran out of bytes before a flush terminated the command list.
            return Err(RecvParseError::Truncated);
        }
        let len_hex = &body[i..i + 4];
        let len_str = std::str::from_utf8(len_hex).map_err(|_| RecvParseError::BadLengthPrefix)?;
        let len =
            usize::from_str_radix(len_str, 16).map_err(|_| RecvParseError::BadLengthPrefix)?;

        // A flush (`0000`) — and defensively any special marker < 4 — ends the
        // command list. The remaining bytes are the packfile.
        if len < 4 {
            i += 4;
            break;
        }
        if i + len > body.len() {
            return Err(RecvParseError::Truncated);
        }
        let mut payload = &body[i + 4..i + len];
        i += len;

        // The FIRST command carries `NUL <caps>`; split it off before parsing.
        let nul = if commands.is_empty() {
            payload.iter().position(|&b| b == 0)
        } else {
            None
        };
        if let Some(nul) = nul {
            let caps_raw = &payload[nul + 1..];
            capabilities = std::str::from_utf8(caps_raw)
                .unwrap_or("")
                .split_ascii_whitespace()
                .map(str::to_string)
                .collect();
            payload = &payload[..nul];
        }
        commands.push(parse_command(payload)?);
    }

    if commands.is_empty() {
        return Err(RecvParseError::NoCommands);
    }

    Ok(ReceiveRequestWire {
        commands,
        capabilities,
        pack: body[i..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    /// Frame `data` as a pkt-line (4-hex length prefix including itself).
    fn pkt(out: &mut Vec<u8>, data: &[u8]) {
        out.extend_from_slice(format!("{:04x}", data.len() + 4).as_bytes());
        out.extend_from_slice(data);
    }
    fn flush(out: &mut Vec<u8>) {
        out.extend_from_slice(b"0000");
    }

    #[test]
    fn create_with_caps_and_pack_parses() {
        let mut body = Vec::new();
        let mut first = format!("{ZERO_OID} {A} refs/heads/main").into_bytes();
        first.push(0);
        first.extend_from_slice(b"report-status side-band-64k agent=git/2.43");
        pkt(&mut body, &first);
        flush(&mut body);
        body.extend_from_slice(b"PACKfake-pack-bytes");

        let got = parse_receive_pack_body(&body).expect("parses");
        assert_eq!(got.commands.len(), 1);
        assert_eq!(got.commands[0].ref_name, "refs/heads/main");
        assert!(got.commands[0].is_create());
        assert!(!got.commands[0].is_delete());
        assert!(got.capabilities.contains(&"report-status".to_string()));
        assert_eq!(got.pack, b"PACKfake-pack-bytes");
        got.require_pack().expect("pack present");
    }

    #[test]
    fn caps_only_on_first_of_multiple_commands() {
        let mut body = Vec::new();
        let mut first = format!("{A} {B} refs/heads/main").into_bytes();
        first.push(0);
        first.extend_from_slice(b"report-status");
        pkt(&mut body, &first);
        pkt(
            &mut body,
            format!("{ZERO_OID} {B} refs/heads/feature").as_bytes(),
        );
        flush(&mut body);
        body.extend_from_slice(b"PACKxyz");

        let got = parse_receive_pack_body(&body).expect("parses");
        assert_eq!(got.commands.len(), 2);
        assert_eq!(got.capabilities, vec!["report-status".to_string()]);
        assert_eq!(got.commands[1].ref_name, "refs/heads/feature");
    }

    #[test]
    fn delete_only_push_needs_no_pack() {
        let mut body = Vec::new();
        let mut first = format!("{A} {ZERO_OID} refs/heads/stale").into_bytes();
        first.push(0);
        first.extend_from_slice(b"report-status");
        pkt(&mut body, &first);
        flush(&mut body);
        // no pack bytes follow

        let got = parse_receive_pack_body(&body).expect("parses");
        assert!(got.commands[0].is_delete());
        assert!(!got.needs_pack());
        assert!(got.pack.is_empty());
        got.require_pack().expect("delete needs no pack");
    }

    #[test]
    fn non_delete_without_pack_is_missing_pack() {
        let mut body = Vec::new();
        let mut first = format!("{ZERO_OID} {A} refs/heads/main").into_bytes();
        first.push(0);
        first.extend_from_slice(b"report-status");
        pkt(&mut body, &first);
        flush(&mut body);
        // no PACK stream — malformed for a create

        let got = parse_receive_pack_body(&body).expect("frames parse");
        assert_eq!(got.require_pack(), Err(RecvParseError::MissingPack));
    }

    #[test]
    fn empty_command_list_is_no_commands() {
        let mut body = Vec::new();
        flush(&mut body);
        assert_eq!(
            parse_receive_pack_body(&body),
            Err(RecvParseError::NoCommands)
        );
    }

    #[test]
    fn bad_oid_is_bad_command() {
        let mut body = Vec::new();
        let mut first =
            b"deadbeef 2222222222222222222222222222222222222222 refs/heads/main".to_vec();
        first.push(0);
        first.extend_from_slice(b"report-status");
        pkt(&mut body, &first);
        flush(&mut body);
        body.extend_from_slice(b"PACKz");
        assert!(matches!(
            parse_receive_pack_body(&body),
            Err(RecvParseError::BadCommand(_))
        ));
    }

    #[test]
    fn malformed_shape_is_bad_command() {
        let mut body = Vec::new();
        let mut first = format!("{A} refs/heads/main").into_bytes(); // missing a field
        first.push(0);
        pkt(&mut body, &first);
        flush(&mut body);
        assert!(matches!(
            parse_receive_pack_body(&body),
            Err(RecvParseError::BadCommand(_))
        ));
    }

    #[test]
    fn ref_outside_refs_namespace_is_rejected() {
        let mut body = Vec::new();
        let mut first = format!("{ZERO_OID} {A} HEAD").into_bytes();
        first.push(0);
        pkt(&mut body, &first);
        flush(&mut body);
        body.extend_from_slice(b"PACKz");
        assert!(matches!(
            parse_receive_pack_body(&body),
            Err(RecvParseError::BadCommand(_))
        ));
    }

    #[test]
    fn truncated_length_is_truncated() {
        // claims 0020 (32 bytes) but only a few follow
        let body = b"0020short".to_vec();
        assert_eq!(
            parse_receive_pack_body(&body),
            Err(RecvParseError::Truncated)
        );
    }

    #[test]
    fn non_hex_length_prefix_is_bad_prefix() {
        let body = b"zzzz....".to_vec();
        assert_eq!(
            parse_receive_pack_body(&body),
            Err(RecvParseError::BadLengthPrefix)
        );
    }

    #[test]
    fn no_flush_before_eof_is_truncated() {
        let mut body = Vec::new();
        let mut first = format!("{ZERO_OID} {A} refs/heads/main").into_bytes();
        first.push(0);
        pkt(&mut body, &first);
        // no flush, no pack — stream just ends
        assert_eq!(
            parse_receive_pack_body(&body),
            Err(RecvParseError::Truncated)
        );
    }
}
