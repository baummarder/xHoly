# Milestone 35 — Guest Execution Expansion

**Status:** implemented and verified (synthetic fixtures only)
**Verification date:** 2026-11-29
**Evidence:** `tests/guest_expansion_tests.rs` (7 tests), CPU/HLE/loader unit
tests, `cargo test --workspace`

## Goal

Execute a more complex self-authored guest through the existing pipeline
(PE loader → GuestMemory → GuestSpace → X64Cpu → HLE → trace) by adding only
the instructions and diagnostics that the audit showed are genuinely useful
— never by replacing M33/M34 architecture or implementing the full x86-64 ISA.

## 1. CPU coverage audit (before M35)

Source: `src/core/cpu/x64.rs`. Widths: 32/64-bit only, no 0x66/0x67/LOCK/segment
prefixes. Flags modelled: **CF, ZF, SF, OF**; PF/AF/DF/IF are not modelled
(which is why JP/JNP fault explicitly).

| Class | Encodings | Behaviour | Flags |
|---|---|---|---|
| Data movement | `B8+r` (`imm32`/REX.W `imm64`), `89`, `8B`, `C7 /0` imm32 sign-extended | register/memory MOV | none |
| ALU | `83 /n ib`, `81 /n id`, `01/09/21/29/31/39`, `03/0B/23/2B/33/3B` | ADD/OR/AND/SUB/XOR/CMP; ADC/SBB fault | CF/ZF/SF/OF per `alu()` |
| Test | `85 /r` | AND without write-back | CF=OF=0, ZF/SF |
| Control | `EB`, `E9`, `70..7F`, `0F 80..8F`, `E8`, `FF /2`, `C3` | JMP/Jcc/CALL/RET | reads CF/ZF/SF/OF |
| Stack | `50+r`, `58+r` | PUSH/POP r64 with bounds faults | none |
| System | `90` NOP, `F4` HLT | retire / halt | none |

Addressing: register, `[reg]`, `[reg+disp8/32]`, SIB, RIP-relative.
Explicitly unsupported: ADC/SBB, byte/word operands, LEA, INT/INT3, far
jumps, JP/JNP, all other `0F` opcodes.

**Gap that blocked complex guests:** no address calculation without a dummy
memory access (LEA), no idiomatic loop counter (INC/DEC), no zero/sign
extension (MOVZX/MOVSX) and no flag-driven byte result (SETcc). A 6-step
guest could call HLE but could not compute, branch and derive a value.

## 2. New CPU instructions

| Mnemonic | Encodings | Semantics / limitations |
|---|---|---|
| `LEA` | `8D /r` | Effective address of any supported addressing mode (incl. RIP-relative and SIB) written to the destination register. **No memory access, no flags.** REX.W selects 64-bit, otherwise truncated to 32-bit. |
| `INC` / `DEC` | `FF /0`, `FF /1` | ADD/SUB 1 with **CF preserved** (x86 rule); other flags updated. 32/64-bit register or memory. Byte forms `FE` remain unsupported. |
| `MOVZX` | `0F B6 /r`, `0F B7 /r` | Byte/word → register, zero-extended. Narrow register source reads the low bytes; memory reads validated. No flags. |
| `MOVSX` | `0F BE /r`, `0F BF /r` | Byte/word sign-extended to width; 32-bit destinations truncate to 32 bits. No flags. |
| `SETcc` | `0F 90..9F /0` | Reuses the single `condition()` source; writes **only one byte** (upper register bits preserved). `9A/9B` (parity) fault. No flags. |

All new encodings fault explicitly when malformed; no new decoder layer
was introduced. Opcodes `ADC/SBB`, `INT`, `JP/JNP`, shifts, IMUL, CMOV and
full byte/word ALU remain unsupported.

## 3. HLE coverage audit (before → after)

Registry (unchanged): 7 modules, 21 function entries — **0 Implemented**,
2 `PartiallyImplemented` (`xmem.dll!XMemAlloc`, `xmem.dll!XMemFree`), 2
`Unsupported`, 17 `Unknown`. Runtime registers exactly the two xmem shims
(arity 2). All other symbols are preserved with evidence; no fabricated
return value is ever produced.

**No registry state was changed in M35.** Adding a callable query export
would require a publicly documented Xbox signature, which does not exist
for the remaining xmem symbols. Instead:

- `GuestHeap::query(addr) -> AllocationInfo` (new, Rust-side diagnostics):
  resolves any interior address to the guest base and reports size,
  alignment, the **raw, undecoded** attribute word and protection. Unknown
  or freed addresses return `InvalidAddress`.
- `XMemFree` faults now carry the **actual guest address** instead of `0`.
- `Completed` dispatch details now include `guest=0x… attrs=0x…` (alloc)
  or `freed=0x…` (free). Existing assertions on `"implemented"` remain valid.

All HLE implementations still validate arguments, respect guest bounds and
return guest VAs — never host pointers.

## 4. Guest memory and loader

- GuestSpace routing and `validate_range` are unchanged: flat title window
  and guest heap remain strictly separated, cross-boundary ranges are
  rejected as a whole.
- `pe64::validate` now requires the entry RVA to lie inside a section with
  `IMAGE_SCN_MEM_EXECUTE (0x2000_0000)` and validates that every relocation
  site lies inside a section.
- `apply_relocations` rejects unsupported relocation kinds even when the
  base delta is zero.
- `Pe64Image::string_at` is now panic-free on unvalidated images.

No MMU, no delay-load imports, no TLS callbacks, no retail package support.

## 5. Synthetic guest program

`pe64::synthetic_compute_guest()` (source-generated, 43 entry bytes):

```text
mov ecx, 3          ; loop counter
xor eax, eax        ; accumulator
loop: add eax,5
      dec ecx
      jnz loop      ; 3 iterations → EAX = 15
lea rcx,[rax+1]     ; size = 16 (arg0)
xor edx,edx         ; attributes = 0 (arg1)
mov r8d,eax         ; preserve the sum across the HLE call
call [IAT 0x1600]   ; XMemAlloc(16, 0)
test eax,eax
setnz cl            ; success bit
movzx ecx,cl
add ecx,r8d         ; 1 + 15
mov eax,ecx
ret                 ; entry sentinel
```

**Actual result:** `StopReason::Returned`, 21 retired instructions,
`RAX = 16` (`SYNTHETIC_COMPUTE_RESULT`), one `XMemAlloc` dispatch with
`args = [16, 0]` and a guest heap pointer. Trace step records carry the
mnemonics `dec`, `jcc`, `lea`, `movzx`, `setcc`, `call`.

## 6. Trace format

M35 adds an **optional** `mnemonic: Option<String>` to `StepRecord` and
`BootEvent::InstructionExecuted`. The versioned trace file moves to
`schema_version: 2`; files written with M34 schema 1 still import
unchanged (`TRACE_MIN_SCHEMA_VERSION = 1`). No other field shape changed;
M33/M34 CLI wire formats remain identical.

## 7. Tests

- `tests/guest_expansion_tests.rs` (7): valid fixture, full compute guest
  with mnemonics and HLE args, schema-2 round trip, invalid-pointer
  diagnostics with real address, heap query metadata, non-executable entry
  rejection, M33/M34 regressions.
- CPU unit tests for LEA/INC/DEC/MOVZX/MOVSX/SETcc semantics.
- Loader/HLE unit tests for the new validation and diagnostics.
- Existing suites (`guest_pipeline_tests`, `guest_boot_tests`,
  `gui_trace_tests`, `cli_progress_tests`) remain green.

## 8. Limitations

No full x86-64 ISA, no byte/word ALU, no shifts/IMUL/CMOV, no MMU, no
retail boot, no GDK dependency. `XMem` remains **Partially Implemented**.
Trace mnemonics are best-effort decoder labels, not a disassembler.

## 9. M36 suggestion

Add a GUI step inspector that renders each imported `StepRecord` with its
mnemonic and guest RIP (M34's suggestion), then extend the subset with
shifts (`C1 /4,/5`) for bit-mask driven synthetic guests.
