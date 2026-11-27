# Xbox HLE API Inventory — Milestone 37

**Status:** research inventory (read-only analysis; no invented APIs)
**Verification date:** 2026-11-30
**Authority:** `src/core/hle/mod.rs::with_known_entries()`,
`docs/adr/0001-target-architecture.md`

## 1. Scope and evidence policy

xHoly targets **Xbox Series X|S (Gen9)** only. ADR 0001, decision 6, is
normative: *"Xbox-ABI signatures, structure layouts and function numbers
may only come from verifiable sources. Own syscall numbers as substitutes
for real ABI values are not allowed. Sources that apply to another console
(e.g. Xenia for Xbox 360) are excluded."*

Allowed evidence in this repository:

- public GDK **conceptual** pages on `docs.xbox.com` (`/build/...`);
- `microsoft/DirectXTex` and `microsoft/Xbox-ATG-Samples` (MIT);
- the ScarlettX README for target/ordinal pointers only (no source copied;
  no LICENSE in the local copy, so no implementation was derived from it).

Explicitly **not** used as Gen9 evidence:

- access-restricted GDK `/reference/...` pages (including `d3d12_xs.h`);
- Xbox 360 Xenia ordinals in `src/core/kernel/xam_abi.rs` (feature-gated,
  legacy);
- `xam.rs` "plausible success values" (xHoly-internal placeholders);
- third-party reverse-engineering interfaces;
- identically named PC D3D12 APIs as proof of Xbox export equivalence.

Documentation classes used below:

| Class | Meaning |
|---|---|
| **(a)** | Publicly documented Microsoft interface (name/signature). |
| **(b)** | Reliably documented role, contract or call shape — **not** a full interface. |
| **(c)** | Experimental assumption / inferred value. |
| **(d)** | Unknown; no reliable public information in the repository. |

## 2. Module inventory (7 modules)

| Module | Role | State | Evidence |
|---|---|---|---|
| `xam.xex` | XAM user/profile services | Unknown | Xbox 360 Xenia ordinals; not verified for Series X\|S (ADR 0001) |
| `xgameruntime.dll` | Game Core runtime | Unsupported | ScarlettX README target; no xHoly implementation |
| `d3d12_xs.dll` | Xbox D3D12 graphics runtime | Unsupported | Public GDK role; no guest device shim or draw translation |
| `xmem.dll` | Xbox memory services | PartiallyImplemented | Public GDK call shape; attributes not decoded |
| `xg_xs.dll` | Texture layout / swizzle | Unsupported | MIT samples reference the DLL; xHoly implements none of it |
| `gameinput.dll` | GameInput controller API | Unsupported | ScarlettX README ordinal #1; no shim |
| `msvcp140.dll` | MSVC C++ runtime (host library) | Unsupported | Not an Xbox API; xHoly does not emulate a host CRT |

Tally: **0 Implemented, 1 PartiallyImplemented, 5 Unsupported, 1 Unknown**.

## 3. Function inventory (21 entries)

| # | Module!symbol | State | Class | Evidence / missing artefact |
|---:|---|---|---|---|
| 1 | `xam.xex!XamUserGetSigninState` | Unknown | (d) | Xbox 360 ordinal `0x210`; no Gen9 signature |
| 2 | `xam.xex!XamInputGetState` | Unknown | (d) | Xbox 360 ordinal `0x191`; no Gen9 signature |
| 3 | `xmem.dll!XMemAlloc` | PartiallyImplemented | (b)/(d) | 2-arg shape `(size, ULONGLONG attributes)` public; attribute bitfield layout restricted. Arity 2. |
| 4 | `xmem.dll!XMemFree` | PartiallyImplemented | (b)/(d) | `(pointer, attributes)` evidenced by MIT DirectXTex; attributes ignored. Arity 2. |
| 5 | `xmem.dll!XMemAllocDefault` | Unknown | (d) | No public interface documentation |
| 6 | `xmem.dll!XMemFreeDefault` | Unknown | (d) | No public interface documentation |
| 7 | `xmem.dll!XMemVirtualAlloc` | Unknown | (b)/(d) | Contract described publicly; no parameter list or flag values |
| 8 | `xmem.dll!XMemAllocatePhysicalPages` | Unknown | (b)/(d) | 64 KiB page semantics described; no signature |
| 9 | `xmem.dll!XMemFreePhysicalPages` | Unknown | (b)/(d) | Role described; no signature |
| 10 | `xmem.dll!XMemMapPhysicalPages` | Unknown | (b)/(d) | Mapping semantics described; no signature |
| 11 | `xmem.dll!XMemMapPhysicalPagesScatter` | Unknown | (b)/(d) | Name listed; no signature |
| 12 | `xmem.dll!XMemTransferMemory` | Unknown | (d) | No public documentation |
| 13 | `xmem.dll!XMemGetWorkingSetStatistics` | Unknown | (b)/(d) | "in-use footprint query"; no signature or struct layout |
| 14 | `xmem.dll!XMemIsToolsMemAvailable` | Unknown | (d) | No public documentation |
| 15 | `xg_xs.dll!XGCreateTextureComputer` | Unknown | (d) | Exact symbol undocumented; samples use `XGCreateTexture2DComputer` |
| 16 | `xg_xs.dll!XGSuggestSwizzleMode` | Unknown | (d) | Samples use `XGComputeOptimalSwizzleMode`; sources disagree |
| 17 | `d3d12_xs.dll!D3D12XboxCreateDevice` | Unknown | (d) | Signature only in restricted `d3d12_xs.h` |
| 18 | `d3d12_xs.dll!D3D12ConfigureVirtualMemoryX` | Unknown | (d) | No public documentation |
| 19 | `d3d12_xs.dll!D3D12SerializeVersionedRootSignature` | Unknown | (a)/(c) | PC API public; Xbox export equivalence unverified |
| 20 | `d3d12_xs.dll!D3D12CreateDevice` | Unsupported | (a)/(d) | PC name public; no guest device shim |
| 21 | `gameinput.dll!GameInputCreate` | Unsupported | (a)/(d) | Ordinal #1 known; no signature/vtable/struct layout in repo |

Tally: **0 Implemented, 2 PartiallyImplemented, 2 Unsupported, 17 Unknown**.
Corroborated by `tests/hle_expansion_tests.rs::registry_status_is_honest`
and `progress.json` (`hle-vm-services` notes).

## 4. Per-API implementation assessment

### 4.1 Memory (M37.2)

`XMemAlloc` and `XMemFree` are the only Xbox functions with a real host
implementation. The **two arguments are interpreted strictly from the
evidenced call shape**: `(size, attributes)` and `(pointer, attributes)`.
The attribute word is recorded verbatim in `GuestHeap::Allocation` and is
**never decoded** — no guessed flag bits. Alignment is xHoly's documented
fallback (`DEFAULT_ALIGNMENT = 16`), explicitly not an Xbox guarantee.
Guest addresses are returned as `HleValue::Unsigned`; no host pointer is
ever exposed. Invalid sizes, alignment, addresses and double frees produce
typed `HleFault`s.

Honest improvements in M37: stronger integration coverage for valid and
invalid parameters, allocation/free lifecycle and registry consistency.
**Status remains `PartiallyImplemented`** until attribute semantics are
publicly documented.

All other 10 `xmem.dll` entries have role-only or no evidence. Implementing
them would require inventing parameter lists, flag values or structure
layouts. They stay `Unknown`.

### 4.2 KernelX (M37.3)

No verified Series X|S kernel API signature exists in this repository. The
Xbox 360 Xenia ordinals in `xam_abi.rs` and the "plausible" values in
`xam.rs` are explicitly excluded by ADR 0001. The M36 `xh_vm.krn.dll` /
`xh_vm.sync.dll` services are xHoly VM scaffolding, **not** kernelX APIs;
they are not renamed or reused as Xbox implementations. Both `xam.xex`
entries stay `Unknown`.

### 4.3 Game Runtime (M37.4)

No `XGameRuntime`, `XTaskQueue`, `XAsync` or `XSystem` signature is present
in the repository. `xgameruntime.dll` therefore remains `Unsupported`.
The `xg_xs.dll` exports stay `Unknown`: exact names differ from the MIT
samples and third-party sources disagree.

### 4.4 GameInput (M37.5)

`GameInputCreate` has a verified ordinal (#1) from the ScarlettX README but
**no signature, vtable or structure layout** in the repository. An
internal host input abstraction (`src/input/`) exists, but connecting it to
a guessed Xbox vtable would fabricate an ABI. The service stays
`Unsupported` and dispatch returns a structured, value-less result.
A `P2` research gate is recorded: add a citable public signature/vtable
source before any implementation.

### 4.5 Storage / Platform (M37.6)

No `XStore`/`XPackage`/virtual-filesystem signature or package-structure
evidence exists in the repository. `msvcp140.dll` is a host CRT, not an
Xbox API. No package data, retail files or access-control bypasses were
added. No status changes.

### 4.6 Graphics (M37.7)

`src/core/hle/graphics.rs` is a clean architectural seam: vendor-neutral
`HostBackendKind`, `HostRenderBackend`, `GraphicsRuntime` device bookkeeping.
`submit_draw` and `create_resource` deliberately return
`GraphicsError::Unsupported`. Creating a host wgpu/Vulkan context would
**not** implement `D3D12CreateDevice`: guest command translation, resource
semantics and the Xbox export are unverified. All four `d3d12_xs.dll`
entries keep their current states.

## 5. Boundary: xHoly VM services are not Xbox HLE

`HleRegistry::with_xh_vm_entries()` defines 4 modules / 17 functions under
`xh_vm.*`. Every evidence string states "NOT an Xbox API". They are fully
implemented and tested, but they are excluded from the Xbox compatibility
inventory and from the HLE compatibility percentage. The registry helper
`with_all_entries()` combines both namespaces only for synthetic guests;
the read-only analyzer and CLI use `with_known_entries()`.

## 6. Implemented work in M37

No Xbox API status was changed. The milestone delivers:

1. this complete inventory with per-API evidence classification;
2. integration tests that dispatch **all 21 Xbox entries** through the
   single resolver/dispatcher and assert honest statuses and no fabricated
   return values;
3. focused lifecycle/bounds tests for the two partial xmem shims;
4. a documented research gate for each API that cannot be implemented
   without inventing semantics.

## 7. Priority list

| Priority | Item | Honest action |
|---|---|---|
| P0 | `XMemAlloc` / `XMemFree` | Keep partial; harden diagnostics and tests |
| P1 | Documentation | This inventory; record role-only evidence explicitly |
| P2 | `GameInputCreate` | Research gate: citable public signature/vtable required |
| P3 | `d3d12_xs.dll` exports | Export verification + real guest device shim required |
| P4 | All 17 Unknown entries | Unchanged until verifiable public documentation exists |

## 8. Known limitations

No retail boot, no Xbox kernel ABI, no MMU, no GPU command translation, no
GDK dependency, no copied ScarlettX code. M37 adds **no** new Xbox API
implementation because none can be implemented without violating the
evidence policy. Future milestones must add citable public sources first.
