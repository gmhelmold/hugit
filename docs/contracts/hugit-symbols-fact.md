# Contract: the `hugit-symbols` outline fact

**Status:** grounded fact contract (KungFu). Every claim cites `file:line` in
`crates/hugit-symbols` (and its two call sites) at the time of writing.

## The fact (one sentence)

`hugit_symbols::outline_blob(lang, source) -> Vec<SymbolItem>` is a **pure,
total, deterministic, bounded** function from one blob's bytes to a flat,
source-ordered outline of its declarations — the local "semantic index" fact the
serve and CLI layers map onto the wire.

## Signature and shape

```rust
pub fn outline_blob(lang: Lang, source: &[u8]) -> Vec<SymbolItem>;

pub struct SymbolItem {
    pub kind: SymbolKind, // domain enum, never a raw string on this side
    pub name: String,
    pub line: u32,        // 1-based line of the declaration's start
}
```

- `crates/hugit-symbols/src/lib.rs:144` — `pub fn outline_blob(lang: Lang, source: &[u8]) -> Vec<SymbolItem>`
- `crates/hugit-symbols/src/lib.rs:115-121` — `SymbolItem { kind, name, line }`
  ("Maps 1:1 onto the four wire fields the serve layer fills"; `line` is 1-based,
  `lib.rs:119-120`).

The crate is "**Pure + headless: no I/O, no network, no serde**" — it returns a
plain domain type the cli/serve layers map onto the frozen
`hugit_http_contracts::blob::OutlineItemVm` wire shape
(`crates/hugit-symbols/src/lib.rs:1-10`).

## The `kind` vocabulary — produced in exactly one place

`SymbolKind` is a domain enum (never a raw string) so the wire vocabulary is
emitted in **one** place: `SymbolKind::as_wire_str`
(`crates/hugit-symbols/src/lib.rs:46-47, 86-108`). The full vocabulary and its
exact wire tokens (`crates/hugit-symbols/src/lib.rs:88-106`):

| `SymbolKind` | wire token | | `SymbolKind` | wire token |
|---|---|---|---|---|
| `Fn`        | `fn`     | | `Function`    | `function`    |
| `Struct`    | `struct` | | `Method`      | `method`      |
| `Enum`      | `enum`   | | `Constructor` | `constructor` |
| `Trait`     | `trait`  | | `Class`       | `class`       |
| `Impl`      | `impl`   | | `Interface`   | `interface`   |
| `Mod`       | `mod`    | | `Namespace`   | `namespace`   |
| `Const`     | `const`  | | `Field`       | `field`       |
| `Static`    | `static` | | `Variable`    | `variable`    |
| `Macro`     | `macro`  | | `Constant`    | `constant`    |
| `TypeAlias` | `type`   | | `Module`      | `module`      |

The vocabulary is **frozen** — the source comment at
`crates/hugit-symbols/src/lib.rs:51-58` warns the Rust-era block is "frozen — do
not change a token, it is a wire break", and the deliberate distinctions are
documented: `Fn`→`fn` vs `Function`→`function` (Rust emits `fn`, the curly-brace
family emits `function`) and `Mod`→`mod` (Rust) vs `Module`→`module` (Ruby).

## Supported languages (extension → `Lang`)

`lang_for_ext` is the single source of truth for extension→language, shared by
serve/cli for their display label so the file labeled (e.g.) "rust" is the exact
file the parser parses, no drift (`crates/hugit-symbols/src/lang.rs:1-5,45-49`).
The mapping (`crates/hugit-symbols/src/lang.rs:50-63`):

| extensions | `Lang` | display label (`Lang::as_str`) |
|---|---|---|
| `rs` | `Rust` | `rust` |
| `ts` `mts` `cts` | `TypeScript` | `typescript` |
| `tsx` | `Tsx` | `tsx` |
| `js` `jsx` `mjs` `cjs` | `JavaScript` | `javascript` |
| `py` `pyi` | `Python` | `python` |
| `go` | `Go` | `go` |
| `java` | `Java` | `java` |
| `c` `h` | `C` | `c` |
| `cc` `cpp` `cxx` `hpp` `hh` `hxx` | `Cpp` | `cpp` |
| `rb` | `Ruby` | `ruby` |
| anything else | `None` | — (caller emits an empty outline) |

- `crates/hugit-symbols/src/lang.rs:11-25` — the `Lang` enum (10 variants).
- `crates/hugit-symbols/src/lang.rs:29-42` — `Lang::as_str` display labels.
- `crates/hugit-symbols/src/lang.rs:50-63` — `lang_for_ext` (case-insensitive bare
  extension; `_ => None`, line 63 — an unknown extension yields no language,
  "honest, never fabricated", `lang.rs:48-49`).

## Totality — empty outline, never a panic

The function is **total**: every degenerate input returns an **empty outline**,
never a panic, never an error. The contract is stated at
`crates/hugit-symbols/src/lib.rs:18-22` ("Total: non-UTF-8, binary,
unknown-language, or unparseable input returns an empty outline — never a
panic") and enforced at:

1. **Oversized input** — `source.len() > MAX_INPUT_BYTES` returns `Vec::new()`
   (`crates/hugit-symbols/src/lib.rs:146-148`). `MAX_INPUT_BYTES = 2 * 1024 * 1024`
   (2 MiB) — a hard bound against a hostile multi-GB blob OOMing the parser
   (`crates/hugit-symbols/src/lib.rs:40-43`).
2. **Non-UTF-8 / binary** — `std::str::from_utf8(source)` failure returns
   `Vec::new()` (`crates/hugit-symbols/src/lib.rs:151-153`).
3. **Parser/query/classifier panic** — the parse path runs inside
   `std::panic::catch_unwind(AssertUnwindSafe(…))` and `.unwrap_or_default()`s to
   an empty outline (`crates/hugit-symbols/src/lib.rs:159-162`) — "never letting a
   parser panic escape to the caller".
4. **A grammar query that fails to compile** caches `None` and degrades to empty
   (`crates/hugit-symbols/src/lib.rs:176, 181-184` — "A query that fails to compile
   caches `None` (degrade to empty — the totality contract)").
5. **A classifier that drops a match / an empty name** is skipped, not emitted
   (`crates/hugit-symbols/src/lib.rs:281-286`).

## Determinism

The same bytes always yield the same outline
(`crates/hugit-symbols/src/lib.rs:17`). Order is **deterministic**: by source
position (declaration start byte), then `kind` wire-token, then `name` to break
ties (`crates/hugit-symbols/src/lib.rs:292-299`). The emitted `line` is 1-based
(`crates/hugit-symbols/src/lib.rs:288`).

## The two consumers (this fact, mapped onto the wire)

This pure fact is consumed verbatim by both surfaces — neither re-derives the
vocabulary or the language table:

- **Serve** — `crates/hugit-serve/src/handlers/blob.rs:223-233`: `compute_outline`
  selects the language via `hugit_symbols::lang_for_ext` (line 227), calls
  `hugit_symbols::outline_blob` (line 230), and maps each item onto
  `hugit_http_contracts::blob::OutlineItemVm` with `kind: item.kind.as_wire_str()`
  (line 233). Symbol names are scrubbed at the read boundary
  (`crates/hugit-serve/src/handlers/blob.rs:14`); the VM's `active` cursor flag is
  set by the server, not the parser (`crates/hugit-symbols/src/lib.rs:112-114`).
- **CLI** — `crates/hugit-cli/src/symbol/mod.rs:5-6, 224-228`: the `hugit symbol`
  verb derives the language from `hugit_symbols::lang_for_ext` and parses with
  `hugit_symbols::outline_blob`, emitting stable JSON.

## Where bulk fact-derivation belongs

`outline_blob` is cheap per blob, but parsing is not free — `Query::new` is "tens
of ms — worst for the large C++ grammar", which is why the engine caches each
compiled query in a per-language `OnceLock` (`crates/hugit-symbols/src/lib.rs:168-186`;
the comment records a real `.rs` blob hot-path cost of ~6 s → ~0.5 s after caching).

On the single-threaded lazy-CAS engine, even a result-count-bounded read is a
latency hazard because each object is a synchronous R2 fetch that blocks the whole
accept loop. Therefore **bulk fact-derivation (outlining many blobs / a whole
repo) is KungFu-workload work — run it off the engine's read path, not as a heavy
synchronous engine read.** The engine's per-blob `compute_outline` on a single
requested file is fine (one blob, query cached); fanning the outliner across a
repo to build an index is the AC-memoized wrapper's job, exactly as
`crates/hugit-symbols/src/lib.rs:11-14` describes ("a content-addressed semantic
index keyed by tree-hash, parsed once per subtree across all tenants — the
AC-memoized wrapper that wraps THIS pure function later"). This pairs with the
read-by-sha contract's same rule (`docs/contracts/read-by-sha.md`).

## Honest caveats

- The grander "content-addressed semantic index, parsed once per subtree across
  all tenants" is **not built** — this crate is "strictly the local
  parse-blob-to-outline core" (`crates/hugit-symbols/src/lib.rs:11-15`). The
  AC-memoized bulk wrapper is a future surface, not a current one.
- The vocabulary/extension counts above are exact at the time of writing; the
  crate is explicitly extensible (add a `Lang` variant + an arm in `lang_for_ext`
  + a query + a classifier arm — `crates/hugit-symbols/src/lang.rs:7-9`). The
  durable contract is *purity + totality + determinism + one-place vocabulary*,
  not the exact language count.
