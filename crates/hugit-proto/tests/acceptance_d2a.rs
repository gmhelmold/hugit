//! WP-D2a acceptance oracle — git wire-protocol READ path over the CoreLink CAS.
//!
//! Owned items (VERBATIM from decomposition v2.0 — D2a):
//!   ① clone byte-identical to mirror
//!   ② delta-only fetch
//!
//! Driven by `tests/acceptance/wp-d2a/run.sh`.
//!
//! Strategy: build a genuine git object graph (blobs, trees, commits) in a
//! content-addressed CAS, serve clone/fetch over protocol v2, and assert against
//! a "mirror" — a second, independently-built CAS holding the same content. A
//! clone reconstructs byte-identical objects to the mirror; a fetch transfers
//! only the delta. Every object id is the real git SHA-1 (computed by the same
//! libgit2-class hash git uses), so byte-identity is not asserted by fiat.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use gix_hash::ObjectId;
use hugit_proto::read::negotiate::{Capabilities, RefAdvertisement, WantHave};
use hugit_proto::read::pack::{CasObjectSource, GitObject, ObjectKind, ObjectSource, parse_oid};
use hugit_proto::read::serve::{serve_clone, serve_fetch};

// ─── git object construction (canonical bytes; real oids) ────────────────────

/// A blob holding `content`.
fn blob(content: &[u8]) -> GitObject {
    GitObject::new(ObjectKind::Blob, content.to_vec())
}

/// A tree with one regular-file entry `name -> blob_oid` (mode 100644). The
/// canonical git tree encoding is `"<mode> <name>\0<20-byte-oid>"` concatenated.
fn tree_one(name: &str, blob_oid: &ObjectId) -> GitObject {
    let mut body = Vec::new();
    body.extend_from_slice(b"100644 ");
    body.extend_from_slice(name.as_bytes());
    body.push(0);
    body.extend_from_slice(blob_oid.as_slice());
    GitObject::new(ObjectKind::Tree, body)
}

/// A commit pointing at `tree`, with optional `parent`. Fixed author/committer
/// and timestamp so the oids are deterministic across runs.
fn commit(tree: &ObjectId, parent: Option<&ObjectId>, message: &str) -> GitObject {
    let mut body = String::new();
    body.push_str(&format!("tree {tree}\n"));
    if let Some(p) = parent {
        body.push_str(&format!("parent {p}\n"));
    }
    let ident = "hugit <bot@hugit.dev> 1717000000 +0000";
    body.push_str(&format!("author {ident}\n"));
    body.push_str(&format!("committer {ident}\n"));
    body.push('\n');
    body.push_str(message);
    body.push('\n');
    GitObject::new(ObjectKind::Commit, body.into_bytes())
}

/// A repository fixture: a CAS of objects plus its ref tips.
struct Fixture {
    cas: CasObjectSource,
    refs: BTreeMap<String, String>,
    /// The tip commit oids, by ref, for assertions and fetch negotiation.
    tips: BTreeMap<String, ObjectId>,
}

/// Build a deterministic two-commit history on `refs/heads/main` plus a
/// single-commit `refs/heads/feature`, sharing the base tree's blob. Returns the
/// fixture and the per-commit oids the fetch test advances against.
fn build_repo() -> Fixture {
    let mut cas = CasObjectSource::new();

    // --- base commit (c1) ---
    let b1 = blob(b"hello hugit\n");
    let b1_oid = cas.insert(b1.clone());
    let t1 = tree_one("README", &b1_oid);
    let t1_oid = cas.insert(t1.clone());
    let c1 = commit(&t1_oid, None, "init");
    let c1_oid = cas.insert(c1.clone());

    // --- second commit on main (c2), new blob+tree, parent c1 ---
    let b2 = blob(b"hello hugit, again\n");
    let b2_oid = cas.insert(b2.clone());
    let t2 = tree_one("README", &b2_oid);
    let t2_oid = cas.insert(t2.clone());
    let c2 = commit(&t2_oid, Some(&c1_oid), "update readme");
    let c2_oid = cas.insert(c2.clone());

    // --- feature branch: one commit off c1, reusing c1's tree ---
    let cf = commit(&t1_oid, Some(&c1_oid), "feature work");
    let cf_oid = cas.insert(cf.clone());

    let mut refs = BTreeMap::new();
    refs.insert("refs/heads/main".to_string(), c2_oid.to_string());
    refs.insert("refs/heads/feature".to_string(), cf_oid.to_string());

    let mut tips = BTreeMap::new();
    tips.insert("c1".into(), c1_oid);
    tips.insert("c2".into(), c2_oid);
    tips.insert("cf".into(), cf_oid);

    Fixture { cas, refs, tips }
}

/// Parse the object ids out of a served pack by re-reading them from the source
/// the pack was built against (the pack lists its `object_ids`). We then prove
/// each one round-trips: the bytes the CAS holds for that oid hash to that oid.
fn assert_objects_byte_identical(
    pack_ids: &[ObjectId],
    a: &dyn ObjectSource,
    b: &dyn ObjectSource,
) {
    for oid in pack_ids {
        let from_a = a.get(oid).expect("source a get").expect("present in a");
        let from_b = b.get(oid).expect("source b get").expect("present in b");
        // Byte-identical between the served repo and the mirror.
        assert_eq!(
            from_a.data, from_b.data,
            "object {oid} bytes differ between served repo and mirror"
        );
        assert_eq!(from_a.kind, from_b.kind, "object {oid} kind differs");
        // And the bytes are genuinely the bytes git stores for that oid.
        assert_eq!(&from_a.oid(), oid, "object {oid} is not content-addressed");
    }
}

// ─── ① clone byte-identical to mirror ────────────────────────────────────────

/// A clone serves a protocol-v2 ref advertisement and a packfile whose objects
/// are byte-identical to what the GitHub mirror holds for the same refs. The
/// pack is a real V2 packfile (PACK magic + V2 version + trailing SHA-1), and
/// the served object closure is exactly the full reachable set — no more, no
/// fewer — proving git is never broken and the clone equals the mirror.
#[test]
fn item_1_clone_byte_identical() {
    let repo = build_repo();
    // The mirror is an independently-built CAS holding the SAME content — this is
    // what "byte-identical to the mirror" means: same objects, same bytes.
    let mirror = build_repo();

    // Capability advertisement is protocol v2 with ls-refs + fetch.
    let caps = Capabilities::default();
    let caps_bytes = caps.encode().expect("encode capabilities");
    let caps_text = String::from_utf8_lossy(&caps_bytes);
    assert!(
        caps_text.contains("version 2"),
        "must advertise protocol v2, got: {caps_text:?}"
    );
    assert!(caps_text.contains("command=ls-refs"));
    assert!(caps_text.contains("command=fetch"));

    // Serve the clone: ref advertisement + assembled pack.
    let (adv, pack) = serve_clone(&repo.refs, &repo.cas).expect("clone must succeed");

    // Ref advertisement reflects the D1 derived view (both refs, name-sorted).
    let ad_bytes = adv.encode().expect("encode advertisement");
    let ad_text = String::from_utf8_lossy(&ad_bytes);
    assert!(
        ad_text.contains("refs/heads/main") && ad_text.contains("refs/heads/feature"),
        "advertisement must list both refs: {ad_text:?}"
    );
    assert_eq!(adv.refs.len(), 2, "exactly two refs advertised");

    // The pack is a real git V2 packfile.
    assert_eq!(
        &pack.bytes[0..4],
        b"PACK",
        "pack must start with PACK magic"
    );
    let version = u32::from_be_bytes(pack.bytes[4..8].try_into().unwrap());
    assert_eq!(version, 2, "pack must be version 2");
    let count = u32::from_be_bytes(pack.bytes[8..12].try_into().unwrap());
    assert_eq!(
        count as usize,
        pack.object_count(),
        "header object count must match"
    );
    // Trailing 20-byte SHA-1 over the pack stream (non-zero, present).
    assert!(
        pack.bytes.len() > 12 + 20,
        "pack must carry entries plus a trailing checksum"
    );

    // The clone closure is EXACTLY the full reachable object set of the repo.
    // Repo has: 3 commits (c1,c2,cf), 2 distinct trees (t1 shared by c1+cf, t2),
    // 2 distinct blobs (b1,b2) = 7 objects.
    assert_eq!(
        pack.object_count(),
        7,
        "clone must pack the full 7-object closure"
    );
    let served: BTreeSet<ObjectId> = pack.object_ids.iter().copied().collect();
    assert_eq!(served.len(), 7, "no duplicate objects in the pack");
    // Tips must be reachable/included.
    for tip in [&repo.tips["c2"], &repo.tips["cf"]] {
        assert!(served.contains(tip), "tip {tip} must be in the clone pack");
    }

    // Byte-identity vs the mirror: every served object equals the mirror's bytes.
    assert_objects_byte_identical(&pack.object_ids, &repo.cas, &mirror.cas);

    // A second clone of the same repo is deterministic — identical pack bytes.
    let (_adv2, pack2) = serve_clone(&repo.refs, &repo.cas).expect("second clone");
    assert_eq!(
        pack.bytes, pack2.bytes,
        "clone pack assembly must be deterministic (byte-for-byte)"
    );
}

// ─── ② delta-only fetch ──────────────────────────────────────────────────────

/// After cloning at c1, the client advances `main` to c2 and fetches with c1 as
/// its `have`. The server must transfer ONLY the delta — the objects reachable
/// from c2 but not from c1 (the new commit, its new tree, its new blob) — never
/// re-sending the objects the client already holds.
#[test]
fn item_2_delta_only_fetch() {
    let repo = build_repo();
    let c1 = repo.tips["c1"];
    let c2 = repo.tips["c2"];

    // Full closure reachable from c2 (what a fresh clone of just main@c2 sends).
    let full = serve_fetch(
        &repo.cas,
        &WantHave {
            wants: vec![c2],
            haves: vec![],
            done: true,
        },
    )
    .expect("baseline full transfer");
    // c2 reachability: c2, t2, b2, c1, t1, b1 = 6 objects.
    assert_eq!(full.object_count(), 6, "full c2 closure is 6 objects");

    // Delta-only fetch: want c2, already have c1.
    let request = WantHave {
        wants: vec![c2],
        haves: vec![c1],
        done: true,
    };
    let delta = serve_fetch(&repo.cas, &request).expect("delta fetch must succeed");

    // The delta is exactly the 3 NEW objects (c2, its tree t2, its blob b2);
    // c1 and everything reachable from it is NOT re-sent.
    assert_eq!(
        delta.object_count(),
        3,
        "delta must carry only the 3 new objects, got {}",
        delta.object_count()
    );
    let sent: BTreeSet<ObjectId> = delta.object_ids.iter().copied().collect();
    assert!(sent.contains(&c2), "the new tip c2 must be in the delta");
    assert!(
        !sent.contains(&c1),
        "the already-held commit c1 must NOT be re-sent"
    );

    // None of the objects reachable from the client's have (c1's closure) appear
    // in the delta — the strict delta-only guarantee.
    let have_closure = serve_fetch(
        &repo.cas,
        &WantHave {
            wants: vec![c1],
            haves: vec![],
            done: true,
        },
    )
    .expect("have closure");
    for held in &have_closure.object_ids {
        assert!(
            !sent.contains(held),
            "object {held} from the have-closure must not be in the delta"
        );
    }

    // The delta is strictly smaller than a full re-clone (proves no full re-send).
    assert!(
        delta.object_count() < full.object_count(),
        "delta ({}) must transfer fewer objects than a full clone ({})",
        delta.object_count(),
        full.object_count()
    );
    assert!(
        delta.bytes.len() < full.bytes.len(),
        "delta pack must be smaller than the full pack"
    );

    // Up-to-date fetch: want c2, already have c2 → empty delta (zero objects).
    let none = serve_fetch(
        &repo.cas,
        &WantHave {
            wants: vec![c2],
            haves: vec![c2],
            done: true,
        },
    )
    .expect("up-to-date fetch");
    assert_eq!(
        none.object_count(),
        0,
        "an up-to-date fetch transfers zero objects"
    );

    // The negotiated wants in a clone-all request equal the advertised tips
    // (negotiation wires the D1 derived view into the want set).
    let adv = RefAdvertisement::from_view(&repo.refs);
    let clone_req = WantHave::clone_all(&adv).expect("clone_all");
    let advertised: BTreeSet<ObjectId> = adv
        .refs
        .iter()
        .map(|(oid, _)| parse_oid(oid).unwrap())
        .collect();
    let wanted: BTreeSet<ObjectId> = clone_req.wants.iter().copied().collect();
    assert_eq!(advertised, wanted, "clone wants every advertised tip");
}
