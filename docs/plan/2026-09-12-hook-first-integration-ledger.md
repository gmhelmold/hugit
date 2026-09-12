# Hook-first integration ledger

Integration classification, not merge result. Source payload excludes ledger seed
commit/path: 91 commits, 65 paths. Refs captured 2026-09-12.

```text
BASE	origin/main	58ee1629a4bfec9ef73a90beb56be7b009bad7ee
SOURCE	feat/hook-first-automation	2eef714e65599754a230f23061d187ff4dd09411
COMMIT_RANGE	origin/main..feat/hook-first-automation
PATH_RANGE	origin/main...feat/hook-first-automation
COMMIT_COUNT	91
PATH_COUNT	65
```

## Commit ledger

Format: `SHA<TAB>SUBJECT<TAB>TARGET_DOMAIN<TAB>DISPOSITION`. `SQUASH` means
fold into named domain's rewrite; `DROP` means no surviving independent delta.

```text
f6ac15175fa8c7bac50428680b320d0213392768	feat(cli): add hook attachment health	hook-health	REWRITE
cada5ff9496d36024a990c14eb59efb2a56a839e	fix(cli): migrate hook state to git runtime	runtime-store	REWRITE
d199528783e731392f1ea131e98e3af03a16a746	fix(cli): bind legacy runtime prefix before append	runtime-store	SQUASH
3b293a7b9eab7762222a0b6812d0c5f8efdd1722	fix(cli): guard runtime migration before append	runtime-store	SQUASH
9a183975837ee8816e56b18fa87f704d0ebed8ef	fix(cli): centralize runtime log mutation gate	runtime-store	SQUASH
209fbdb58592fc55b0d6b6dc424095f8701f1dc4	fix(cli): harden hook runtime bootstrap	runtime-store	SQUASH
73f81c52976e025e7d6e940f6e4681fd315896c0	fix(cli): bootstrap template hook runtime	runtime-store	SQUASH
4e6c8bebb1575df661f49134e11be0074fbec43f	fix(mcp): resolve runtime logs from repository	mcp-runtime	REWRITE
48e14a4b5a4c3a00f77d24d35aaca52c289c9ff0	fix(mcp): preserve land-status log resolution	mcp-runtime	SQUASH
949f137ae792c5c8fbcb609af6f4ddf3f2aa9961	fix(cli): fall back to legacy log	runtime-store	SQUASH
17ccc9dc864a4a0252e1ddb0542182c39992877b	fix(mcp): fall back to non-git legacy logs	mcp-runtime	SQUASH
8f94e6a250d4f5faac72c53d91f6e04ac4db05cb	fix(mcp): delegate absent non-git logs	mcp-runtime	SQUASH
e71898c23ce69ffc8fe269e98cf34bb93232d6ef	fix(mcp): preserve runtime migration honesty	mcp-runtime	SQUASH
6f5e4c54e1bbc2ec7ebb1078e944b5ff05d12f5a	fix(mcp): preserve relative log paths	mcp-runtime	SQUASH
c9f8b997482a87070cd47184d7c790aba1f797d0	fix(mcp): delegate Git runtime capture defaults	mcp-runtime	SQUASH
b250dd80ed246344a236121b9fa5938ef86635bf	fix(mcp): qualify capture verification result	mcp-runtime	SQUASH
1cceb9af26c9222e26bdae6962e0260a400c0a9d	fix(mcp): prove capture confirmation causally	mcp-runtime	SQUASH
61cc09bcdf9aa134b796bb6a0e3ff99f5836548c	fix(mcp): report capture dispatch only	mcp-runtime	SQUASH
1008898bbb75b704efae74f36a151db63e64c273	fix(docs): state MCP capture dispatch contract	docs-runtime	REWRITE
48e404efeb5c6d50856bd8a864ebd2a7f125a866	feat(capture): add durable receipt recovery	durable-capture	REWRITE
2e10dd0cadde073c8503329daada51f8468a5225	fix(capture): harden durable receipt drain	durable-capture	SQUASH
9f112f25d949aec10e8fbd70b6bc1b307d2d04fa	fix(capture): fail closed outside Unix	durable-capture	SQUASH
cc80602a75e659e758541539eb1f0fb98f584124	fix(capture): bind drain lifecycle to claimed receipt	durable-capture	SQUASH
c33240bac664f29e125c77362d016925fa599322	fix(capture): harden durable drain completion	durable-capture	SQUASH
da028a8236ca83332e47e892885a776bd95650cb	fix(capture): bind receipt cleanup to exact marker	durable-capture	SQUASH
87ca77c919662e414352fb536367e4269d4ae5e1	fix(capture): bound receipt completion cleanup	durable-capture	SQUASH
5fe7bd8f4600937266cf2dd10ebf5c09f6684436	docs(plan): freeze receipt contract	durable-capture	KEEP
51b4a0291a049064e35af0aa62ad51f7a3fa7fbb	feat(capture): add durable receipt drain	durable-capture	SQUASH
31df335b984720c8742048c75902207af035c746	fix(cli): harden hook capture inputs	durable-capture	SQUASH
2e44653458a7c477e7d46a6fea5372bf37c2c547	fix(cli): close capture input review P1	durable-capture	SQUASH
1631f38123a7a03399be674a3125dcc5d3ef58ee	test(capture): pin receipt scrub boundary	durable-capture	SQUASH
cc3d909bfd4da723af064ea09412e4041a7d6d0f	fix(capture): scrub dead-letter receipts	durable-capture	SQUASH
5693ea2eeccd793d6ef4b59c379bd383c48dafaa	fix(capture): harden dead-letter redaction	durable-capture	SQUASH
171678f7789d14485671a024021fd313d73ba615	fix(capture): constrain dead-letter filenames	durable-capture	SQUASH
5f4a4837f16a63463a5f71879b176c237d7185b2	feat(capture): harden hook receipt inputs	durable-capture	DROP
da2ce8e59a1e05b9f98ef9ced8b3d736b712d7f1	feat(capture): add truthful Git coverage	durable-capture	REWRITE
561e50976ecda4ff4d11a0e98f537a7351a626c6	fix(capture): converge receipt pairing	durable-capture	SQUASH
7d43d12a95d3a12f81b57178c95e3dbaaccf9452	fix(capture): persist checkout transaction evidence	durable-capture	SQUASH
6c56195d93dd1e4167f855abeedf1fc4198e095f	fix(capture): probe hook capability and reconcile receipts	durable-capture	SQUASH
bbe63df930c7b1b57669154441606ae56168e81b	test(capture): assert platform capability matrix	durable-capture	SQUASH
fd6483acbce3405ce0bd050462255ed03a19ed58	test(capture): stabilize capability matrix probe	durable-capture	SQUASH
72eca3caa518ec0aa5060d4f4589d553f98d77c7	fix(policy): preserve serialized change payloads	policy-boundary	REWRITE
669afef367de1f601f6f80281ae09563d704dd8d	feat(capture): report truthful Git coverage	durable-capture	DROP
7fcca5d06d8132b0bb6574a0d5e6bf143a17e48f	docs(plan): split projection work	projection	KEEP
050c2b56a1105d9e8f11278fd254b7517cfea907	feat(capture): add bounded projection runner	projection	REWRITE
095798f8b36ae44c6605356f92efb0174df96650	fix(capture): harden projection authority	projection	SQUASH
e609745783c373f274d8ad93a7bf767f1733fc4d	fix(capture): reset busy drain count	projection	SQUASH
e2bcfe8d780c1912d5fcda05b4456dff83bc9ab8	fix(capture): serialize projection completion	projection	SQUASH
795feb430e4cb0d4798aead2c6784b74a62ddbd0	fix(capture): guard drain status ownership	projection	SQUASH
2b7cdbe7b6e34d02997abe54c5a0dd96d68904aa	fix(capture): claim projection work briefly	projection	SQUASH
be24db2cc7693aec0c9427845b541c00578e297c	fix(capture): fence stale projector effects	projection	SQUASH
169fbcae14d40aeb31c490f41d0aa5e1f5674416	fix(capture): fence closed projection declaration	projection	SQUASH
ae57bac5641b71817db3a400fa883b88ed08176a	feat(capture): add projection runner contract	projection	DROP
45e471a6524ae99aa2997790f472a0d49d3d0431	fix(capture): retain projection completion ledger	projection	SQUASH
d1468bc0a5f5c4d231370ceae71c4ce55bc29d96	fix(capture): harden projector recovery	projection	SQUASH
1ab2e38ec09c6eb132583505e0d6181451852a3f	fix(capture): support projection fanout	projection	DROP
d1a14a1b62fc2c67149312509eff4ba9617ca5de	feat(dock): project physical worktree lifecycle	projection	REWRITE
be8cf95b7b4d37cd84077f7d0856027dd1cfeea0	feat(capture): project receipt provenance	projection	SQUASH
00e351fb240dd7c88a4a0bd4b7157e26556567d3	test(capture): pin derived projector facts	projection	SQUASH
95a1ab0269aea3162efca0be2b30255a04c16e2e	test(capture): accept skipped fanout outcomes	projection	SQUASH
2c70fb0522ad759af385532f323883a7ef88aeb0	fix(dock): preserve lifecycle projection history	projection	SQUASH
3c7c940095fa8891ea4d60c443c4f44eb487e1e6	test(capture): name required drain projectors	projection	SQUASH
b0d139421251af5f142f59932782853f37526cd8	fix(dock): persist lifecycle reconcile projection	projection	SQUASH
f8fa8d0c73cc2008b745b5ba7044f60fbd2ceb41	fix(dock): reconcile owner worktree inventory	projection	SQUASH
dab05907b75148a259f0a58f182ff94263e192f3	feat(dock): project worktree lifecycle	projection	DROP
05a6cc02ecc2f51f70ecc53993467fdb0ce68818	feat(capture): project provenance facts	projection	DROP
c75766715dc9cc268e9810465fd611127c71f35f	feat(cli): surface source-linked projection views	projection	REWRITE
91ff808145ddb5eebf42121e85289a625bdc3f26	fix(cli): verify projection views	projection	SQUASH
582b00bd3b53b2be6a3f110add38b7235440e808	fix(cli): reject incomplete projection retries	projection	SQUASH
9bb0efdf57093b9cdf6978c694bfbf75e726b6bb	fix(cli): distinguish projection retries from pending	projection	SQUASH
e3b7986bdaf749bda47bacb497e8c303d796121c	feat(cli): project verified local views	projection	DROP
fad58ee446b3c078f54d174648f22d047b368137	fix(capture): finalize drain before receipt cleanup	durable-capture	SQUASH
c47b4d3750ae5c51b1a68826e19c5a49cb3a3f92	fix(capture): persist terminal status before cleanup	durable-capture	DROP
9d00c8a29fd4abd8be6a2f6a5e4b2e7199a6c176	test(capture): align jj evidence assertions	durable-capture	SQUASH
cc2a5424d96df227828adcbc59da3c8afd26854b	fix(capture): retain merge parent identities	durable-capture	SQUASH
2fcfd999b1e0223417c1e6b3986d5b6488a43b83	style: format merge capture checks	durable-capture	DROP
ed1c095cdaba6ec1b173409ab1ab0f67b08cb65a	test(fleet): retry concurrent capture drain	durable-capture	SQUASH
96d77a4d4c5d6d8ead90b3d5ab95a38955f208af	test(dock): allow concurrent capture drain	durable-capture	SQUASH
4541f07d597375333ae5355190b283fefd0b8b4b	test(capture): observe isolated checkout facts	durable-capture	SQUASH
f573f642b164f7aac817029bd9af687a104b321b	style: format capture isolation wait	durable-capture	DROP
b2967092185b296c03f6ff37ded63f8c0290923a	test(capture): allow checkout worker under load	durable-capture	SQUASH
ecba386528cedc247414d534b068cccb71128aff	test(capture): report rewrite drain evidence	durable-capture	SQUASH
447e804aabd2d03bbfb84f6f9568fb9279e1e89d	fix(capture): retry transient hook capability probes	durable-capture	SQUASH
6dfdc18f71f409f059d4ee97898fbb9d8118d832	test(pr): retry concurrent capture drain	durable-capture	SQUASH
342c932cf85c75cce359f1a6f4523f20216859ea	docs: lead hook-first onboarding	docs-runtime	REWRITE
ea2cc8c25ae324e1e5cce44b6afcd95e9822df52	feat(init): safely adopt managed hook dispatchers	hook-init	REWRITE
ba3c9e70ffb5d3b4962bda95fff9b125ef689b2d	fix(init): preserve adopted hook input	hook-init	SQUASH
df30a984c2d330164cac2e0423ba4fe7528841d8	fix(capture): retry busy detached drains	durable-capture	SQUASH
254fca50bcc3c7d7924acf3fc2cd0415791f94d1	test(capture): select durable commit capture	durable-capture	SQUASH
32e3c7cedee7433202c60a44e33e75ae4f69f657	test(mcp): await durable capture drain	mcp-runtime	SQUASH
2eef714e65599754a230f23061d187ff4dd09411	docs(cli): clarify hook-first health workflow	hook-health	SQUASH
```

## Path ledger

Format: `STATUS<TAB>PATH<TAB>TARGET_DOMAIN<TAB>DISPOSITION<TAB>NAMED_TEST_OR_PROOF`.

```text
M	.github/workflows/ci.yml	ci	DROP	ci.yml-current-matrix-review
M	CHANGELOG.md	release-notes	CONFLICT	changelog-current-release-review
M	Cargo.lock	workspace-deps	REWRITE	cargo-test-workspace-locked
M	Cargo.toml	workspace-deps	REWRITE	cargo-test-workspace-locked
M	README.md	docs-runtime	CONFLICT	readme-command-link-review
M	crates/hugit-cli/Cargo.toml	workspace-deps	REWRITE	cargo-test-p-hugit-cli
M	crates/hugit-cli/src/campaign/open.rs	runtime-store	REWRITE	acceptance-gitlocal-journey
M	crates/hugit-cli/src/campaign/world.rs	runtime-store	REWRITE	acceptance-gitlocal-journey
A	crates/hugit-cli/src/capture/capability.rs	durable-capture	REWRITE	acceptance-capture-capability-matrix
A	crates/hugit-cli/src/capture/drain.rs	durable-capture	REWRITE	acceptance-capture-durable-drain
M	crates/hugit-cli/src/capture/mod.rs	durable-capture	CONFLICT	acceptance-capture-durable-drain
A	crates/hugit-cli/src/capture/receipt.rs	durable-capture	REWRITE	acceptance-runtime-store-receipt
M	crates/hugit-cli/src/dock/close.rs	projection	REWRITE	acceptance-dock-reconcile
A	crates/hugit-cli/src/dock/lifecycle.rs	projection	REWRITE	acceptance-dock-reconcile
M	crates/hugit-cli/src/dock/mod.rs	projection	REWRITE	acceptance-dock-reconcile
M	crates/hugit-cli/src/dock/resolve.rs	runtime-store	REWRITE	acceptance-dock-resolver
M	crates/hugit-cli/src/fleet/mod.rs	projection	REWRITE	acceptance-projection-views
A	crates/hugit-cli/src/health.rs	hook-health	REWRITE	acceptance-wp8-surface
M	crates/hugit-cli/src/init/mod.rs	hook-init	CONFLICT	acceptance-capture-hook-adoption
M	crates/hugit-cli/src/intent/canonical_log.rs	runtime-store	REWRITE	acceptance-gitlocal-journey
M	crates/hugit-cli/src/lib.rs	hook-health	REWRITE	acceptance-wp8-surface
M	crates/hugit-cli/src/log_resolve.rs	runtime-store	REWRITE	acceptance-runtime-store
M	crates/hugit-cli/src/main.rs	hook-health	CONFLICT	acceptance-wp8-surface
M	crates/hugit-cli/src/policy/edit.rs	policy-boundary	REWRITE	acceptance-pr-commit
M	crates/hugit-cli/src/porcelain.rs	durable-capture	REWRITE	acceptance-capture-jj-checkout-merge
M	crates/hugit-cli/src/pr/filelock.rs	durable-capture	REWRITE	acceptance-pr-commit
M	crates/hugit-cli/src/pr/mod.rs	durable-capture	REWRITE	acceptance-pr-commit
A	crates/hugit-cli/src/projection.rs	projection	REWRITE	acceptance-projection-views
A	crates/hugit-cli/src/projection/context.rs	projection	REWRITE	acceptance-projection-views
A	crates/hugit-cli/src/projection/freshness.rs	projection	REWRITE	acceptance-projection-views
A	crates/hugit-cli/src/projection/provenance.rs	projection	REWRITE	acceptance-projection-views
A	crates/hugit-cli/src/projection/views.rs	projection	REWRITE	acceptance-projection-views
A	crates/hugit-cli/src/runtime_store.rs	runtime-store	REWRITE	acceptance-runtime-store
M	crates/hugit-cli/src/setup.rs	hook-init	CONFLICT	acceptance-capture-hook-adoption
M	crates/hugit-cli/src/watch/mod.rs	projection	REWRITE	acceptance-projection-views
M	crates/hugit-cli/tests/acceptance_capture.rs	durable-capture-tests	CONFLICT	cargo-test-p-hugit-cli-acceptance-capture
M	crates/hugit-cli/tests/acceptance_capture_jj_checkout_merge.rs	durable-capture-tests	SQUASH	cargo-test-p-hugit-cli-acceptance-capture-jj-checkout-merge
M	crates/hugit-cli/tests/acceptance_dock_coinage.rs	projection-tests	SQUASH	cargo-test-p-hugit-cli-acceptance-dock-coinage
M	crates/hugit-cli/tests/acceptance_dock_insights.rs	projection-tests	SQUASH	cargo-test-p-hugit-cli-acceptance-dock-insights
M	crates/hugit-cli/tests/acceptance_dock_reconcile.rs	projection-tests	SQUASH	cargo-test-p-hugit-cli-acceptance-dock-reconcile
M	crates/hugit-cli/tests/acceptance_dock_resolver.rs	projection-tests	SQUASH	cargo-test-p-hugit-cli-acceptance-dock-resolver
M	crates/hugit-cli/tests/acceptance_fleet_journey.rs	durable-capture-tests	SQUASH	cargo-test-p-hugit-cli-acceptance-fleet-journey
M	crates/hugit-cli/tests/acceptance_gitlocal_journey.rs	runtime-store-tests	SQUASH	cargo-test-p-hugit-cli-acceptance-gitlocal-journey
M	crates/hugit-cli/tests/acceptance_pr_commit.rs	durable-capture-tests	SQUASH	cargo-test-p-hugit-cli-acceptance-pr-commit
A	crates/hugit-cli/tests/acceptance_projection_views.rs	projection-tests	REWRITE	cargo-test-p-hugit-cli-acceptance-projection-views
M	crates/hugit-cli/tests/acceptance_rcli.rs	hook-health-tests	SQUASH	cargo-test-p-hugit-cli-acceptance-rcli
A	crates/hugit-cli/tests/acceptance_runtime_store.rs	runtime-store-tests	REWRITE	cargo-test-p-hugit-cli-acceptance-runtime-store
A	crates/hugit-cli/tests/acceptance_wp8_surface.rs	hook-health-tests	REWRITE	cargo-test-p-hugit-cli-acceptance-wp8-surface
A	crates/hugit-cli/tests/fixtures/wp8-capture-help.golden	hook-health-tests	REWRITE	acceptance-wp8-surface-golden
M	crates/hugit-ledger/src/fleet/mod.rs	durable-capture	REWRITE	acceptance-fleet-journey
M	crates/hugit-mcp/src/server.rs	mcp-runtime	REWRITE	cargo-test-p-hugit-mcp
M	crates/hugit-mcp/src/tools/capture.rs	mcp-runtime	REWRITE	cargo-test-p-hugit-mcp
M	crates/hugit-mcp/src/tools/land_status.rs	mcp-runtime	REWRITE	cargo-test-p-hugit-mcp
M	crates/hugit-mcp/src/tools/mod.rs	mcp-runtime	REWRITE	cargo-test-p-hugit-mcp
M	crates/hugit-refstore/src/replay/mod.rs	runtime-store	CONFLICT	cargo-test-p-hugit-refstore
M	crates/hugit-refstore/tests/acceptance_d1a.rs	runtime-store-tests	SQUASH	cargo-test-p-hugit-refstore-acceptance-d1a
M	docs/feature-ledger.md	docs-runtime	CONFLICT	docs-feature-ledger-review
M	docs/installation.md	docs-runtime	CONFLICT	docs-installation-review
A	docs/plan/2026-09-07-hook-first-cli.md	plan	KEEP	plan-hook-first-contract-review
A	docs/plan/wp-3-receipt-contract.md	durable-capture	KEEP	receipt-contract-review
M	docs/product/command-catalog.md	docs-runtime	REWRITE	command-catalog-review
M	docs/quickstart-golive.md	docs-runtime	CONFLICT	quickstart-golive-review
M	docs/quickstart-hooks.md	docs-runtime	CONFLICT	quickstart-hooks-review
M	docs/quickstart-local.md	docs-runtime	CONFLICT	quickstart-local-review
M	docs/review/2026-09-03-session-state-git-local.md	docs-runtime	REWRITE	review-state-claim-audit
```

## Conflict decisions

`git merge-tree --write-tree origin/main feat/hook-first-automation` found 13
content conflicts. `MANUAL` preserves current-base security/runtime work while
porting source behavior into target domains. Decision evidence is merge-tree
output captured during preflight; implementation proof is path-ledger proof.

```text
PATH	DECISION	TARGET_DOMAIN	RATIONALE	STATUS
CHANGELOG.md	DROP	release-notes	current release history wins	VERIFIED
README.md	MANUAL	docs-runtime	current install surface diverged	VERIFIED
crates/hugit-cli/src/capture/mod.rs	MANUAL	durable-capture	capture boundary diverged	VERIFIED
crates/hugit-cli/src/init/mod.rs	MANUAL	hook-init	current hook install changed	VERIFIED
crates/hugit-cli/src/main.rs	MANUAL	hook-health	CLI surface diverged	VERIFIED
crates/hugit-cli/src/setup.rs	MANUAL	hook-init	setup flow diverged	VERIFIED
crates/hugit-cli/tests/acceptance_capture.rs	MANUAL	durable-capture-tests	assertions must match rewritten capture	VERIFIED
crates/hugit-refstore/src/replay/mod.rs	MANUAL	runtime-store	current replay contract diverged	VERIFIED
docs/feature-ledger.md	MANUAL	docs-runtime	current feature claims win	VERIFIED
docs/installation.md	MANUAL	docs-runtime	current install flow diverged	VERIFIED
docs/quickstart-golive.md	MANUAL	docs-runtime	current go-live flow diverged	VERIFIED
docs/quickstart-hooks.md	MANUAL	docs-runtime	current hook flow diverged	VERIFIED
docs/quickstart-local.md	MANUAL	docs-runtime	current local flow diverged	VERIFIED
```

`CHANGELOG.md` is `DROP` path disposition and conflict decision. All 13
merge-tree conflict paths have a complete decision.

## Gates

```text
GATE	REQUIREMENT	STATUS	EVIDENCE
G0	base/source refs	VERIFIED	git-rev-parse: 58ee1629 / 83be5f42; payload source 2eef714e
G1	91 commit rows	VERIFIED	git-rev-list --count origin/main..feat/hook-first-automation = 92; excluding ledger seed = 91
G2	65 path rows	VERIFIED	git-diff --name-only origin/main...feat/hook-first-automation = 66; excluding ledger path = 65
G3	non-unresolved dispositions	VERIFIED	91 commit rows, 65 path rows; DROP rationale defined above
G4	conflicts decided	VERIFIED	merge-tree found 13 content conflicts; every path listed
G5	only approved ledger edit	VERIFIED	git-diff --check origin/main; intended tracked change is this file
G6	formatter	NOT_RUN	docs-only classification; no Rust source changed
G7	lint	NOT_RUN	docs-only classification; no Rust source changed
G8	tests	NOT_RUN	docs-only classification; path-specific integration proofs named above
G9	ledger/index review	VERIFIED	rows, refs, conflict output, trailers reviewed before commit
```
