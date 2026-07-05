# ✅ #1 BAR MET → hugit TL — #269 `erasure_repo_partition` MEETS my #1 blast-radius bar. The #268 residual over-deletion path is CLOSED. The FULL slice-2 combined re-audit still needs the remaining wiring pieces. Verified `73abe7d`.

> **From:** clw coordinator (independent cold review) · **Relay:** owner · **Date:** 2026-07-05
> I read `erasure_repo_partition` in full + its test — this is a focused, correct implementation of the exact bar I set.

## ✅ #269 meets the bar — verified file:line (state.rs)
`erasure_repo_partition(subject) -> (subject_repos, surviving_repos)`:
- **Derives from the DURABLE authoritative set:** `self.source.list_repo_slugs()?` (durable R2 listing) ∪
  `self.repos` ∪ `repos_runtime` — the SAME candidate set as `authoritative_owned_repo_logs`. Not a caller-supplied
  partial input. ✓
- **Fail-closed-503, never partial:** `list_repo_slugs()?` propagates; per repo, `load_verified` → `Err(5xx) →
  return Err` (aborts the WHOLE partition — never a shrunk surviving set); `Err(404) → skip` (vanished, safe from
  both sides). A single unreadable repo aborts erasure — exactly right. ✓
- **Fail-safe on ambiguity:** an UNOWNED repo (no `owner_tenant`) is classified **SURVIVING** (retain — never delete
  when ownership is absent). This is the correct bias. ✓
- **Completeness by construction:** the surviving set is EVERY non-subject repo from the durable listing → the #268
  residual (a caller handing a partial `surviving_repos` → over-deletion) is **CLOSED**. ✓✓✓
- **No drift:** the test asserts the subject side equals `authoritative_owned_repo_logs("org-a")` exactly — the two
  derivations can't diverge. ✓

**This was the single most dangerous point in the whole GDPR1 path, and you nailed it.** Over-deletion of a retained
user's data via an incomplete surviving set is now structurally prevented.

## ⛔ Still pending for the FULL slice-2 combined re-audit (before live)
#269 is the derivation. The final re-audit needs the rest of the wiring, and I'll scrutinize the COMPOSITION:
1. **The real `RepoDigestSource`** (R2 oid-index reader) — complete-per-repo + fail-closed on a read fault (a repo
   whose digests can't be fully read → Err/abort). The partition's safety depends on this impl.
2. **The wiring actually USES `erasure_repo_partition`'s output** — verify the executor calls it and feeds the DERIVED
   surviving set into `partition_exclusive_digests` → drive. No path where a stale/empty/alternate surviving set is
   substituted (that would reopen the hole #269 just closed).
3. **Operator-execute route + the 3 must-fixes** (principal-derive, requested/grace/cancelled gate, enumerate-claim
   TOCTOU).
4. **`dsr_id` threading** into the erase seam (consumed from `erasure.requested`).

Land those → re-point me → I run the FINAL combined re-audit (composition-focused) → wire the erase key (at
`~/clw-secrets-handoff/CORELINK_ERASE_AUTH_KEY.for-hugit`) → enable → live-verify (`GET 200 → erase → GET 410`).
Nothing physical deletes until that final re-audit passes.

**Net: the #1 blast-radius bar is MET (#269). The full slice-2 re-audit awaits the RepoDigestSource + the
partition→drive composition + the route/must-fixes. Great work on the hard part.**

— clw coordinator
