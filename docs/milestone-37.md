# Milestone 37 — Xbox HLE Completion (M37-MEGA)

**Status:** inventory and honesty audit complete; no Xbox API status changed
**Verification date:** 2026-11-30
**Evidence:** `docs/xbox-hle-inventory.md`,
`tests/xbox_hle_inventory_tests.rs` (7 tests), `cargo test --workspace`

## Outcome in one sentence

M37 delivers a complete, evidence-classified Xbox API inventory and tests
that prove all 21 Xbox entries are dispatched honestly — **no new Xbox API
was implemented, because none can be implemented without inventing a
signature, flag value, structure layout or return convention**.

## PART A — Git audit

- Branch `main`, remote `origin` (`github.com/baummarder/xHoly.git`).
- `git fetch origin` showed no divergence; HEAD `4b7e838` and all earlier
  M33–M36 commits were already present on `origin/main`.
- `git push origin main` reported `Everything up-to-date`. No force push,
  rebase or history rewrite was performed.
- `Install.md` and `.vscode/settings.json` were never staged or modified.

## PART B — Phase status (M37.1–M37.8)

| Phase | Status | Result |
|---|---|---|
| M37.1 API audit | **Complete** | Full 7-module / 21-function inventory in `docs/xbox-hle-inventory.md` with a/b/c/d evidence classes |
| M37.2 Memory HLE | **Audited, status unchanged** | `XMemAlloc`/`XMemFree` keep `PartiallyImplemented`; argument shape evidenced, attribute bits not; lifecycle/bounds tests added |
| M37.3 KernelX HLE | **Research-gated** | No verified Series X|S kernel ABI in repo; Xbox 360 Xenia ordinals excluded by ADR 0001; xHoly VM services stay separate |
| M37.4 Game Runtime HLE | **Research-gated** | `XGameRuntime`, `XTaskQueue`, `XAsync`, `XSystem` have no signature in the repo; module stays `Unsupported` |
| M37.5 GameInput HLE | **Research-gated** | `GameInputCreate` ordinal claim only; no vtable/struct layout; stays `Unsupported`; host input model (`src/input`) remains internal |
| M37.6 Storage / Platform | **Research-gated** | No `XStore`/`XPackage`/VFS evidence; no package data, no access-control bypass |
| M37.7 Graphics HLE | **Foundation exists, no API claim** | `hle/graphics.rs` seam only; `submit_draw`/`create_resource` always `Unsupported`; no guest device |
| M37.8 Integration / regression | **Complete** | 7 new tests dispatch all 21 entries through the single resolver/dispatcher and check M33–M36 regressions |

Deviations from the requested order: none. Phases B3–B7 are documented as
research-gated rather than implemented, because implementing them would
violate ADR 0001 and the project's no-invented-semantics rule.

## Inventory numbers (verified)

| State | Modules | Functions |
|---|---:|---:|
| Implemented | 0 | 0 |
| PartiallyImplemented | 1 (`xmem.dll`) | 2 (`XMemAlloc`, `XMemFree`) |
| Unsupported | 5 | 2 (`D3D12CreateDevice`, `GameInputCreate`) |
| Unknown | 1 (`xam.xex`) | 17 |

17 `xh_vm.*` services are **excluded** from this table.

## Evidence highlights

- Public Microsoft x64 calling convention:
  <https://learn.microsoft.com/en-us/cpp/build/x64-calling-convention>.
- Public GDK conceptual pages on `docs.xbox.com` (`/build/...`) and MIT
  `microsoft/DirectXTex` / `microsoft/Xbox-ATG-Samples`: XMem two-argument
  call shape and DLL roles only. Restricted `/reference/...` pages unused.
- ScarlettX README: target/DLL list and ordinal hints only; no code copied,
  licence unresolved.
- Xbox 360 Xenia ordinals (`xam_abi.rs`): legacy, feature-gated, excluded as
  Gen9 evidence by ADR 0001.
- `xam.rs` "plausible success values" and legacy `X_ERROR_*` HRESULTs are
  never imported into Gen9 code.

## Guest integration

`tests/xbox_hle_inventory_tests.rs` verifies:

- every one of the 21 Xbox entries dispatches through the single runtime;
- Unknown/Unsupported results never carry a value;
- resolver precedence holds for every symbol;
- XMem allocation/free with valid and invalid parameters reports honest
  faults and records raw attributes without decoding them;
- a guest calling Unknown/Unsupported imports stops with no fake `RAX`;
- VM services and Xbox APIs remain separate in one runtime;
- M33 boot, M34 trace schema, M35 compute guest and M36 multi-module guest
  are unchanged.

## PART C — README rework

`README.md` was fully restructured into 25 sections with an honest status
summary, a Mermaid architecture diagram that visually separates Xbox HLE
from xHoly VM services, real CLI/GUI commands, accurate paths and explicit
limitations. The generated progress block is byte-identical to
`ProgressReport::render_markdown()` (validated by unit test). No invented
badges, screenshots, benchmarks or compatibility claims were added.

## Progress accounting

No percentage was inflated. The M36 `hle-vm-services` task (xHoly VM scaffold,
not an Xbox API) was folded back into `hle-gen9-runtime`; the HLE category
therefore reflects only the real Xbox registry work. Overall remains
**60.2%**, HLE **75.0%**. `progress.json`, `src/progress.rs`,
`tests/cli_progress_tests.rs` and the README block share the date
`2026-11-30`.

## Limitations and next research gates

No retail boot, no Xbox kernel ABI, no MMU, no GPU translation, no GDK.
P2: obtain a citable public GameInput signature/vtable before implementing
`GameInputCreate`. P3: verify `d3d12_xs.dll` exports and build a real guest
device shim before touching graphics APIs. P4: all 17 Unknown entries stay
Unknown until verifiable public documentation exists.
