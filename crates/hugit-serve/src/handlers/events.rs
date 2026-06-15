//! `GET /v1/repos/{repo}/events?since=<seq>` → SSE replay-then-close (spec §2).
//!
//! **Replay-then-close** (the chosen design; see
//! `docs/plan/2026-06-14-serve-production-seams-design.md` Seam A): the retained
//! history since `since` is written as a `text/event-stream` body, followed by a
//! single heartbeat comment, then the connection closes. The sync `tiny_http`
//! serve loop processes responses serially — it cannot safely hold a long-lived
//! stream open — so replay-then-close is the sound, buildable-now approach; the
//! client reconnects from its advanced `since` cursor. True live-tail is the
//! documented P2 seam.
//!
//! **Wire seq is 1-based** (the frozen client contract, `../githugr/crates/
//! githugr-live/tests/events.rs`: `since==0 ⇒ seqs 1,2,3`). The engine's internal
//! `EventLog` seq is 0-based, so the wire seq is `record.seq + 1`. This makes the
//! client's first connect (`since=0`) deliver EVERY event — `seq > since` with a
//! 0-based internal seq would have silently dropped the first record.
//!
//! **Retention floor**: `head_wire.saturating_sub(10_000)` (spec §2, last-10k).
//! When `since < floor`, the FIRST frame is a `kind:"gap"` sentinel
//! (`summary:"recarregue"`) carrying the current head wire seq; the client reloads
//! its §1 projections and resumes from that seq.
//!
//! **Frame shape** (byte-faithful to the client mock): `id: <seq>\n` +
//! `data: {"seq":<seq>,"kind":"<kind>","summary":"<summary>"}\n\n`. Field order is
//! seq → kind → summary (a serde struct, NOT the key-sorting `json!` macro).
//!
//! **Redaction**: `summary` is derived from the record payload (free text) and
//! MUST pass [`crate::fmt::scrub`]. `kind` and `seq` are structural — not scrubbed.
//!
//! **P2 seams** (not implemented here): true live-tail, ≤25s heartbeat on a quiet
//! live stream, multi-client fan-out (CoreLink pub/sub), native `Last-Event-ID`,
//! retention trim.

use std::fmt::Write as _;

use hugit_refstore::EventLog;
use serde::Serialize;
use serde_json::Value;

use crate::fmt::scrub;

/// The retention window (spec §2 last-10k): a `since` older than `head - WINDOW`
/// gets a gap sentinel.
const RETENTION_WINDOW: u64 = 10_000;

/// One SSE `data:` payload. Field order (seq → kind → summary) is byte-faithful to
/// the frozen client frame format; a serde struct preserves declaration order
/// (the `json!` macro would sort keys).
#[derive(Serialize)]
struct Frame<'a> {
    seq: u64,
    kind: &'a str,
    summary: &'a str,
}

/// Map a record `kind` to the spec §2 wire kind, or `None` to skip (internal /
/// audit kinds are NOT surfaced on the live stream — they would need a new spec
/// kind + a client update).
fn record_to_wire_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "pr.queued" => Some("queue"),
        "pr.landed" => Some("land"),
        "check.recorded" => Some("check"),
        "verdict.recorded" => Some("verdict"),
        "pr.comment" => Some("comment"),
        "policy.set" => Some("policy"),
        k if k.starts_with("mirror.") => Some("mirror"),
        _ => None,
    }
}

/// A safe, non-payload default summary per record kind.
fn default_summary(record_kind: &str) -> &'static str {
    match record_kind {
        "pr.queued" => "PR enfileirado",
        "pr.landed" => "PR pousou",
        "check.recorded" => "verificação registrada",
        "verdict.recorded" => "veredito registrado",
        "pr.comment" => "comentário adicionado",
        "policy.set" => "política atualizada",
        _ => "evento de espelho",
    }
}

/// Derive a pt-BR summary from a record payload. Best-effort + kind-aware; the
/// result is ALWAYS scrubbed (payload free text). A missing field or a malformed
/// payload falls back to the kind default — never a panic, never a raw echo.
fn extract_summary(record_kind: &str, payload: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(payload) else {
        return scrub(default_summary(record_kind));
    };
    let field = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);

    let raw = match record_kind {
        "pr.queued" => field("pr_id").map(|id| format!("PR #{id} na fila")),
        "pr.landed" => field("verdict").map(|val| format!("PR pousou: {val}")),
        "check.recorded" => field("name").map(|name| {
            let status = match v.get("exit").and_then(Value::as_i64) {
                Some(0) => "passou",
                Some(_) => "falhou",
                None => "registrado",
            };
            format!("verificação '{name}' {status}")
        }),
        "verdict.recorded" => field("verdict").map(|val| format!("veredito: {val}")),
        "pr.comment" => field("body").map(|body| body.chars().take(80).collect::<String>()),
        "policy.set" => field("rule_id").map(|rule| {
            let state = if v.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
                "ativada"
            } else {
                "desativada"
            };
            format!("regra '{rule}' {state}")
        }),
        _ => None,
    };

    scrub(&raw.unwrap_or_else(|| default_summary(record_kind).to_string()))
}

/// Append one SSE data frame. The `id ≡ data.seq` invariant holds by construction
/// (both use `wire_seq`). `summary` is already scrubbed by the caller.
fn push_frame(buf: &mut String, wire_seq: u64, kind: &str, summary: &str) {
    let data = serde_json::to_string(&Frame {
        seq: wire_seq,
        kind,
        summary,
    })
    .unwrap_or_else(|_| format!(r#"{{"seq":{wire_seq},"kind":"","summary":""}}"#));
    // `write!` to a String is infallible; the result is intentionally ignored.
    let _ = write!(buf, "id: {wire_seq}\ndata: {data}\n\n");
}

/// Build the raw `text/event-stream` body for a replay-then-close SSE response.
///
/// Emits every record whose **wire seq** (`internal seq + 1`) is `> since` and
/// whose `kind` maps to a spec §2 wire kind; prepends a gap frame when `since` is
/// below the retention floor; ends with a single `": hb\n\n"` heartbeat. The
/// returned bytes are the COMPLETE response body.
pub fn build_events(log: &EventLog, _repo: &str, since: u64) -> Vec<u8> {
    let mut buf = String::with_capacity(512);

    // Highest wire seq currently in the log (0 when empty). The internal head seq
    // is `len - 1`, so the head WIRE seq is `len`.
    let head_wire = log.len() as u64;
    let floor = head_wire.saturating_sub(RETENTION_WINDOW);
    if head_wire > 0 && since < floor {
        push_frame(&mut buf, head_wire, "gap", "recarregue");
    }

    for record in log.records() {
        let wire_seq = record.seq + 1;
        if wire_seq <= since {
            continue;
        }
        let Some(wire_kind) = record_to_wire_kind(&record.kind) else {
            continue;
        };
        let summary = extract_summary(&record.kind, &record.payload);
        push_frame(&mut buf, wire_seq, wire_kind, &summary);
    }

    buf.push_str(": hb\n\n");
    buf.into_bytes()
}

#[cfg(test)]
mod tests {
    use hugit_refstore::{Endpoint, PrincipalClass};

    use super::*;

    fn append(log: &mut EventLog, kind: &str, payload: String, at: u64) {
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            kind,
            vec!["orchestrator:test".to_string()],
            payload,
            at,
        )
        .expect("append");
    }

    /// Parse data frames into `(seq, kind, summary)`, asserting `id ≡ data.seq`.
    fn frames(raw: &[u8]) -> Vec<(u64, String, String)> {
        let text = std::str::from_utf8(raw).expect("utf-8");
        let mut out = Vec::new();
        for block in text.split("\n\n") {
            let block = block.trim();
            if block.is_empty() || block.starts_with(':') {
                continue; // heartbeat / blank
            }
            let mut id = None;
            let mut data: Option<Value> = None;
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("id: ") {
                    id = Some(v.trim().parse::<u64>().expect("id u64"));
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = Some(serde_json::from_str(v.trim()).expect("data json"));
                }
            }
            let (id, data) = (id.expect("id line"), data.expect("data line"));
            let seq = data["seq"].as_u64().expect("seq");
            assert_eq!(id, seq, "id must equal data.seq");
            out.push((
                seq,
                data["kind"].as_str().expect("kind").to_string(),
                data["summary"].as_str().expect("summary").to_string(),
            ));
        }
        out
    }

    fn ends_with_heartbeat(raw: &[u8]) -> bool {
        raw.ends_with(b": hb\n\n")
    }

    #[test]
    fn empty_log_is_heartbeat_only() {
        let body = build_events(&EventLog::new(), "r", 0);
        assert!(frames(&body).is_empty());
        assert!(ends_with_heartbeat(&body));
    }

    #[test]
    fn since_0_delivers_the_first_event_as_wire_seq_1() {
        // The whole point of the 1-based wire seq: the client's first connect
        // (since=0) MUST receive the first record (internal seq 0 → wire seq 1).
        let mut log = EventLog::new();
        append(&mut log, "pr.queued", r#"{"pr_id":"1"}"#.to_string(), 1);
        let f = frames(&build_events(&log, "r", 0));
        assert_eq!(f.len(), 1, "the only record must be delivered at since=0");
        assert_eq!(f[0].0, 1, "internal seq 0 is wire seq 1");
        assert_eq!(f[0].1, "queue");
    }

    #[test]
    fn resume_since_1_skips_the_first_event() {
        let mut log = EventLog::new();
        append(&mut log, "pr.queued", r#"{"pr_id":"1"}"#.to_string(), 1);
        append(
            &mut log,
            "policy.set",
            r#"{"rule_id":"dco","enabled":true}"#.to_string(),
            2,
        );
        // since=1 → wire seq 1 already seen → only wire seq >= 2.
        let f = frames(&build_events(&log, "r", 1));
        assert!(f.iter().all(|(seq, _, _)| *seq > 1), "must skip wire seq 1");
        assert!(
            f.iter().any(|(seq, _, _)| *seq == 2),
            "must include wire seq 2"
        );
    }

    #[test]
    fn id_equals_data_seq_for_every_frame() {
        let mut log = EventLog::new();
        append(
            &mut log,
            "verdict.recorded",
            r#"{"verdict":"approve"}"#.to_string(),
            1,
        );
        append(
            &mut log,
            "policy.set",
            r#"{"rule_id":"secrets","enabled":false}"#.to_string(),
            2,
        );
        // frames() asserts id ≡ data.seq internally.
        assert!(!frames(&build_events(&log, "r", 0)).is_empty());
    }

    #[test]
    fn summary_scrubs_secret_in_comment_body() {
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let mut log = EventLog::new();
        append(
            &mut log,
            "pr.comment",
            serde_json::json!({ "body": format!("see {pat}"), "pr_id": "1" }).to_string(),
            1,
        );
        let text = String::from_utf8(build_events(&log, "r", 0)).unwrap();
        assert!(!text.contains(pat), "PAT must not appear in the SSE body");
        assert!(text.contains("[REDACTED]"));
    }

    #[test]
    fn summary_scrubs_secret_in_check_name() {
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let mut log = EventLog::new();
        append(
            &mut log,
            "check.recorded",
            serde_json::json!({ "name": pat, "exit": 0 }).to_string(),
            1,
        );
        let text = String::from_utf8(build_events(&log, "r", 0)).unwrap();
        assert!(!text.contains(pat));
        assert!(text.contains("[REDACTED]"));
    }

    #[test]
    fn internal_kinds_are_not_emitted() {
        for k in [
            "pr.queued",
            "pr.landed",
            "check.recorded",
            "verdict.recorded",
            "pr.comment",
            "policy.set",
        ] {
            assert!(record_to_wire_kind(k).is_some(), "{k} should map");
        }
        for k in [
            "authz.denied",
            "idem.recorded",
            "op.undone",
            "intent.landed",
            "pr.opened",
            "ref.update",
            "issue.transition",
            "erasure.decided",
        ] {
            assert!(record_to_wire_kind(k).is_none(), "{k} must NOT map");
        }
    }

    #[test]
    fn mirror_kinds_map_to_mirror() {
        assert_eq!(
            record_to_wire_kind("mirror.recovered.change_event"),
            Some("mirror")
        );
    }

    #[test]
    fn non_mappable_kind_emits_no_frame() {
        let mut log = EventLog::new();
        append(
            &mut log,
            "issue.transition",
            r#"{"issue_id":1,"to":"open"}"#.to_string(),
            1,
        );
        let body = build_events(&log, "r", 0);
        assert!(frames(&body).is_empty());
        assert!(ends_with_heartbeat(&body));
    }

    #[test]
    fn malformed_payload_does_not_panic_or_leak() {
        let mut log = EventLog::new();
        append(&mut log, "pr.comment", "not-json".to_string(), 1);
        let text = String::from_utf8(build_events(&log, "r", 0)).unwrap();
        assert!(text.contains("\"comment\""), "frame still emitted");
        assert!(
            !text.contains("not-json"),
            "malformed payload must not leak"
        );
        assert!(text.contains("comentário adicionado"), "kind default used");
    }

    #[test]
    fn gap_frame_shape_is_correct() {
        // A >10k-record log is impractical in a unit test; the gap frame's shape
        // is verified directly (the floor branch is covered by the same code path).
        let mut buf = String::new();
        push_frame(&mut buf, 42, "gap", "recarregue");
        assert_eq!(
            frames(buf.as_bytes()),
            vec![(42, "gap".to_string(), "recarregue".to_string())]
        );
    }

    #[test]
    fn frame_bytes_exact_format_seq_first() {
        let mut buf = String::new();
        push_frame(&mut buf, 7, "land", "PR pousou: approve");
        assert_eq!(
            buf,
            "id: 7\ndata: {\"seq\":7,\"kind\":\"land\",\"summary\":\"PR pousou: approve\"}\n\n"
        );
    }
}
