# Round 8 — SEVERE sweep (root-cause class audit, SOTA reports, surgical fixes)

**Owner mandate (2026-06-12):** stop band-aiding instances. The "exemption is a hole"
finding recurred 5× because every prior sweep was a POINT-finding ("find the strongest
finding on your surface") → POINT-fix. A new instance of the same class appears every
round. Switch to **systematic class audits** that produce SOTA, exhaustively-organized
reports, so fixes are **surgical and assertive at the ROOT**, never blind.

## The shift

| Old (Rounds 1–7) | New (Round 8 severe sweep) |
|---|---|
| "Find the strongest DO-NOT-SHIP on your surface" | "ENUMERATE the entire class exhaustively; find the STRUCTURAL root" |
| One repro per agent | Complete coverage MATRIX (every field × every sink × every path) |
| Verdict + fix the instance | Root-cause analysis + the ONE structural fix that makes the class impossible |
| Point-fix | Surgical patch at the choke-point (deny-by-default, single gate, structural invariant) |

## The 6 root-cause CLASSES (one deep auditor each — opus, read-only, exhaustive)

Each class maps to a recurring root the 7 rounds exposed:

1. **REDACTION / secret-at-rest** (root: the scrub boundary has MULTIPLE exemption paths —
   digest-shaped, `cas:`/content-address, identifier-fields, value-gated — each a potential
   hole). Enumerate EVERY exemption, EVERY field of EVERY verb, EVERY sink (.hugit stores,
   --log, <log>.ac, refstore cold tier, queue, envelope/context-ref). Produce the complete
   coverage matrix. Root question: **can the boundary be deny-by-default with ZERO
   exemptions** (allowlist only proven content-address shapes via ONE gate), so no exemption
   can ever hide a secret again?

2. **READ-PATH INTEGRITY** (root: `verify_chain` is called ad-hoc per-verb → a new read verb
   forgets it; `why`/`export` did). Enumerate EVERY code path that reads/projects the log.
   Root question: **is there a single choke-point (one `load_verified_log` that ALL reads
   MUST go through) so forgetting is structurally impossible?**

3. **MEMO-KEY / WEDGE SOUNDNESS** (root: a memo axis must capture EVERY input that affects a
   result; env was uncaptured → stale green). Enumerate EVERY input that can change a check
   result (files, def, toolchain, env, cwd, locale, umask, time, network, args). Map captured
   vs uncaptured. Root question: **is the capture provably complete, or is execution
   hermetic enough that uncaptured inputs cannot affect the result?**

4. **AUTHZ / MUTATION GUARD** (root: `append_authorized` is per-verb; `export` raw-appended).
   Enumerate EVERY write/append/mutation path to log+stores. Root question: **single
   choke-point where every mutation MUST pass the D14 guard?** Plus the caller-asserted
   principal seam — is it honestly fenced everywhere?

5. **STATE-MACHINE INTEGRITY** (verdict/campaign/pr/intent) (root: resolution logic gameable —
   lens-laundering, post-seal append, ghost-verdict, two-phase non-atomic). Enumerate EVERY
   state transition + EVERY multi-write op. Root question: **are the invariants enforced
   structurally (sealed = terminal, reject = sticky, atomic commit) or checked ad-hoc?**

6. **ERROR-LAW / CONCURRENCY / DETERMINISM** (root: error taxonomy collapse, non-atomic ops,
   lock windows). Enumerate EVERY error-emitting path (must be structured JSON exit 2) + EVERY
   concurrent/lock path + EVERY claim of byte-determinism. Root question: **is the error
   envelope enforced at ONE serialization boundary; are all multi-writes atomic?**

(Honesty/docs-vs-code folds into each class's report: every claim cited to its proof.)

## SOTA report contract (each auditor MUST return, written to a file — not inline)

```
# CLASS <n> — <name> — SOTA audit
## 1. Scope & method (exhaustive enumeration, not sampling — say how you achieved completeness)
## 2. Complete inventory  (THE matrix: every <field/path/axis/transition> × every <sink/guard>,
                            with ✓guarded / ✗hole / ~partial for each cell)
## 3. Findings            (each: id · severity P0/P1/P2 · repro (cmd|file:line) · TYPE code/honesty/seam
                            · ROOT (the structural reason it exists))
## 4. Root-cause analysis (the ONE structural reason this CLASS keeps producing holes)
## 5. Recommended structural remediation (the surgical fix that eliminates the CLASS —
                            choke-point / deny-by-default / invariant — NOT a per-instance patch)
## 6. Residual / accepted  (genuine physics/seams, with why)
```

Reports land in `docs/review/round8/class-<n>-<name>.md` (file output — keeps the
orchestrator's window clean; lead reads the reports, not transcripts).

## Lead flow after reports

1. Read all 6 SOTA reports. 2. For each class, decide the ONE structural fix (choke-point).
3. Decompose Wave L as SURGICAL root-fixes (disjoint files, conflict-mapped, isolation:worktree).
4. Each fix DoD: closes the whole class (the report's matrix goes all-✓), proven by a
   class-level test (not just the instance). 5. Stressed full gate. 6. Round 9 = re-audit the
   SAME matrices → convergence = every matrix all-✓, only tracked seams remain.

## Convergence criterion (proposed, pending owner)
NOT "adversary concedes ship" (unreachable under a refuse-by-default mandate). Instead:
**a class audit finds ZERO P0/P1 code holes in its matrix — only accepted-seams/honesty,
all tracked.** Reachable, honest, and per-class measurable.
