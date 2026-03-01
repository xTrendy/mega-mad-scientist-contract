# Localnet Debug Runbook (Cosmic Mad Scientist)

## 1) Purpose

This document records:
- how the local-chain rehearsal environment was designed,
- every major failure observed during setup and rehearsal,
- what was changed to fix or mitigate each issue,
- the current root-cause thesis,
- and the faster workflow we will use next to map issues without blocking on each one serially.

This is a local testing runbook, not a production deployment guide.

## 1.1) Current Status Snapshot

- Environment-side hardening is implemented (toolchain guards, size checks, first-block waits, artifact fallback paths).
- CW721 version alignment is now explicit:
  - contract deps: `cw721 = 0.21.0`, `cw721-base = 0.21.0`,
  - default prebuilt artifact in localnet script: `public-awesome/cw-nfts v0.21.0`.
- Auction artifact handling is now explicit:
  - preferred: optimizer-built `artifacts/mega_mad_scientist.wasm`,
  - fallback: local Rust build path (may still be rejected by validators with strict wasm feature support).
- Documentation alignment updates are now applied:
  - execute/query message tables were updated to match `src/msg.rs`.
- Diagnostic matrix runner is still proposed (not yet implemented).

## 2) What We Are Trying To Do

Goal: run a full, realistic end-to-end rehearsal of the contract on a local `wasmd` network.

Current rehearsal path:
1. Start a 2-node local chain (`localnet_two_node_e2e.sh`).
2. Upload CW721 + auction wasm.
3. Instantiate Standard and Cosmic collections.
4. Create auction by sending Cosmic NFT with hook message.
5. Place bids via Standard NFT sends.
6. Finalize auction, verify winner/loser behavior.
7. Exercise swap staging + claim behavior.

Important scope statement:
- This process is local-only by default.
- It does not touch Cosmos testnet or mainnet unless manually pointed to remote RPC.

## 3) Environment Design (As Implemented)

Scripts:
- `localnet_two_node_e2e.sh`: one-command orchestrator
  - boots 2 local nodes (node0 validator + node1 full node),
  - waits for RPC and first block on both nodes,
  - runs E2E script against node0,
  - tears down nodes unless `KEEP_RUNNING=true`.
- `localnet_e2e.sh`: on-chain scenario runner
  - stores and instantiates contracts,
  - runs auction + bid + finalize + swap flow assertions.

Primary environment knobs:
- `WASM_RUST_TOOLCHAIN` (default now `stable`)
- `RESET_LOCALNET`
- `CW721_WASM` (manual override path)
- `CW721_RELEASE_URL` (prebuilt cw721 source URL)
- `CW721_RELEASE_WASM_PATH` (cache path for downloaded cw721 wasm)

## 4) Chronological Failure Log and Fixes

### A) Missing local chain binary
- Symptom:
  - `ERROR: required command not found: wasmd`
- Cause:
  - `wasmd` not installed / not on `PATH`.
- Mitigation:
  - installed and verified `wasmd`.

### B) Go toolchain incompatibility while building `wasmd`
- Symptom:
  - build failure in `bytedance/sonic` with newer Go.
- Cause:
  - upstream dependency mismatch with Go 1.26.
- Mitigation:
  - used supported Go line (1.25.x) and rebuilt.

### C) Shell `LDFLAGS` leaking into `go install`
- Symptom:
  - link args unexpectedly injected into `wasmd` build.
- Cause:
  - global shell env contaminating build command.
- Mitigation:
  - rebuilt with `env -u LDFLAGS make install`.

### D) Genesis/account mismatch on multi-node bootstrap
- Symptom:
  - `failed to validate account in genesis ... does not have a balance`.
- Cause:
  - genesis/gentx assumptions were inconsistent for two validators in this script flow.
- Mitigation:
  - simplified to one validator (node0) + one full node (node1) with shared genesis.

### E) RPC started but chain not ready
- Symptom:
  - `WasmApp is not ready; please wait for first block: invalid height`.
- Cause:
  - script proceeded before first committed block.
- Mitigation:
  - added explicit `wait_first_block` checks for both nodes.

### F) Missing CW721 artifact path
- Symptom:
  - `CW721 wasm not found`.
- Cause:
  - script required path, but artifact not present.
- Mitigation:
  - added auto-resolution and auto-build path in two-node script.

### G) CW721 wasm exceeded local upload limit
- Symptom:
  - `uncompress wasm archive: max 819200 bytes: exceeds limit`.
- Cause:
  - built cw721 artifact too large for default local wasm upload config.
- Mitigation:
  - size-optimized release build flags and hard size checks.

### H) Wasm validator compatibility failure (`bulk memory support is not enabled`)
- Symptom:
  - static wasm validation fails while storing contract.
- Cause:
  - artifact contains wasm features local validator rejects.
- Mitigation attempts:
  1. added conservative wasm feature flags in `RUSTFLAGS`,
  2. pinned Rust toolchain to older version,
  3. then reverted to modern toolchain because dependencies require `edition2024`,
  4. added toolchain compatibility guard (requires cargo >= 1.85),
  5. added prebuilt cw721 release download fallback (MVP-compatible artifact) as primary path.

### I) Edition mismatch after pinning toolchain too old
- Symptom:
  - `feature edition2024 is required` when using cargo 1.81.
- Cause:
  - upstream dependency (`rmp`) requires edition 2024 support.
- Mitigation:
  - default toolchain now `stable`, plus explicit guard and clear error message.

### J) Missing logs during debugging
- Symptom:
  - `tail: .localnet/...: No such file or directory`
- Cause:
  - command run from wrong working directory (`~` instead of project directory).
- Mitigation:
  - documented absolute log locations.

### K) CW721 contract-version drift between Rust deps and downloaded wasm
- Symptom:
  - localnet script used `cw721_base.wasm v0.19.0` while the contract migrated to `cw721/cw721-base v0.21.0`.
- Cause:
  - fallback download URL was pinned earlier for validator compatibility and not updated after dependency migration.
- Mitigation:
  - updated localnet default release artifact to `public-awesome/cw-nfts v0.21.0`.
  - retained override controls (`CW721_WASM`, `CW721_RELEASE_URL`) for emergency compatibility testing.

### L) Auction wasm still rejected on strict validators (`bulk memory support is not enabled`)
- Symptom:
  - `cw721` stores successfully, but storing `mega_mad_scientist.wasm` fails with static validation bulk-memory error.
- Cause:
  - local Rust build output can still contain wasm features rejected by the running validator.
- Mitigation:
  - two-node script now prefers optimizer-built auction artifact (`artifacts/mega_mad_scientist.wasm`),
  - script auto-attempts Docker optimizer build when Docker is available,
  - script falls back to Rust build only when optimizer artifact is unavailable.

## 5) Root-Cause Thesis

Primary instability has been in local rehearsal infrastructure, not core auction business logic.

Specifically:
- toolchain and runtime compatibility drift (`wasmd`, rust/cargo editions, wasm feature support),
- local bootstrap orchestration race conditions (first-block readiness),
- artifact path/size/format constraints,
- temporary dependency/artifact version drift during migration windows.

Contract logic itself has already passed local Rust test and lint/audit gates in prior runs; failures here are predominantly deployment/execution-environment level.

## 6) What We Changed in Code/Automation

In `localnet_two_node_e2e.sh`:
- one-validator + one full-node topology,
- first-block waits,
- CW721 resolution hardening,
- wasm size enforcement,
- default `WASM_RUST_TOOLCHAIN=stable`,
- cargo edition-capability guard,
- prebuilt CW721 release artifact download path:
  - default URL: `https://github.com/public-awesome/cw-nfts/releases/download/v0.21.0/cw721_base.wasm`.
- auction artifact resolution:
  - prefer `artifacts/mega_mad_scientist.wasm`,
  - auto-attempt Docker optimizer build (`cosmwasm/optimizer:0.17.0`),
  - fallback to Rust build path if optimizer artifact is unavailable.

In `localnet_e2e.sh`:
- compatibility build function for auction wasm,
- default `WASM_RUST_TOOLCHAIN=stable`,
- cargo edition-capability guard,
- conservative wasm-oriented build flags.

In docs (`README.md`, `LAUNCH_CHECKLIST.md`):
- updated commands to use `WASM_RUST_TOOLCHAIN=stable`,
- documented two-node harness behavior and CW721 artifact behavior,
- updated execute/query message documentation to match current contract API.

## 7) Current Run Commands (Known-Good Workflow)

From repo root:

```bash
cd "/Users/trendy/Mega Mad Scientists Smart Contract/mega-mad-scientist-contract"
rustup update stable
rm -rf .localnet/two-node
WASM_RUST_TOOLCHAIN=stable RESET_LOCALNET=true ./localnet_two_node_e2e.sh
```

Optional explicit optimizer build (recommended when local validator rejects Rust-built auction wasm):

```bash
cd "/Users/trendy/Mega Mad Scientists Smart Contract/mega-mad-scientist-contract"
docker run --rm -v "$(pwd)":/code \
  --mount type=volume,source=cosmwasm_target_cache,target=/target \
  --mount type=volume,source=cosmwasm_registry_cache,target=/usr/local/cargo/registry \
  cosmwasm/optimizer:0.17.0

AUCTION_WASM=artifacts/mega_mad_scientist.wasm \
FORCE_AUCTION_REBUILD=false \
WASM_RUST_TOOLCHAIN=stable \
RESET_LOCALNET=true \
./localnet_two_node_e2e.sh
```

If needed, force explicit CW721 artifact:

```bash
cd "/Users/trendy/Mega Mad Scientists Smart Contract/mega-mad-scientist-contract"
mkdir -p .cache
curl -L --fail https://github.com/public-awesome/cw-nfts/releases/download/v0.21.0/cw721_base.wasm -o .cache/cw721_base.v0.21.0.wasm
WASM_RUST_TOOLCHAIN=stable CW721_WASM="$PWD/.cache/cw721_base.v0.21.0.wasm" RESET_LOCALNET=true ./localnet_two_node_e2e.sh
```

Log files:
- `.localnet/two-node/logs/node0.log`
- `.localnet/two-node/logs/node1.log`

## 8) How We Move Faster Next (Without Faking Passes)

We should not fake successful chain steps (example: pretending `wasm store` passed), because downstream results become invalid.

Faster, accurate plan:
1. Add a diagnostic runner that executes independent checks with continue-on-error. (proposed)
2. Run suites in parallel/sequence where dependency-safe:
   - unit tests, integration tests, clippy, audit, stress/fuzz, localnet e2e.
3. Report a single matrix:
   - `PASS`, `FAIL`, `BLOCKED`, with exact failing stage and log path.
4. Keep strict mode for final launch sign-off:
   - all critical suites must pass without continue-on-error.

This gives full issue mapping quickly while preserving truthful signal.

## 9) Risk Status Snapshot

- High confidence:
  - Most recent blockers are infra/runtime compatibility, not obvious contract-flow regressions.
- Medium risk:
  - Local validator feature set still needs consistent artifact compatibility for every uploaded wasm.
  - Remaining contract/artifact integration mismatches can still appear after dependency upgrades.
- Remaining task:
  - finish one clean full localnet E2E pass using optimizer-compatible auction artifact, then freeze toolchain/script versions.
