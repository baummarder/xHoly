//! Guest execution pipeline — Phase 29.
//!
//! ```text
//! PE loader → import resolution → X64Cpu → HLE dispatch → results
//! ```
//!
//! [`GuestSession::load`] parses and structurally validates a PE32+
//! image, maps it into guest memory at a fixed guest VA, applies base
//! relocations, resolves every IAT slot against the single HLE registry
//! and prepares the CPU (entry point, stack, HLE transition slots).
//! [`GuestSession::run`] then steps the x86-64 subset interpreter and
//! dispatches registered imports through the HLE runtime, recording a
//! structured [`ExecutionReport`].
//!
//! # Boundaries
//!
//! * **Guest addresses are never host pointers.** The IAT is
//!   intercepted by guest VA ([`GuestSession::space`]); a registered
//!   slot is called through without ever reading its contents, and
//!   nothing host-side is ever written into guest memory.
//! * **No Xbox calling convention is claimed.** Arguments travel in
//!   the documented Microsoft x64 registers (`RCX, RDX, R8, R9`; return
//!   in `RAX`) — see [`crate::core::cpu::x64`] for the evidence and
//!   [`HleRuntime::implementation_arity`] for how many are read.
//!   Unsupported/unknown imports receive **no** arguments and return
//!   **no** value.
//! * **Unsupported, unknown and failed dispatches stop execution.**
//!   xHoly never fabricates a return value for a missing
//!   implementation; `RAX` is left untouched.
//! * **One routed address space.** [`GuestSpace`] sends heap-range
//!   addresses to the xmem guest heap (allocation bounds + protection
//!   metadata) and everything else to the flat resident title window;
//!   both reject out-of-range access with structured failures.
//!
//! Loading, execution and diagnostics stay separate on purpose: the
//! loader never executes, the CPU never resolves imports, and the
//! diagnostics only observe.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use crate::core::cpu::x64::{reg, CpuFault, StackBounds, StepOutcome, X64Cpu};
use crate::core::hle::runtime::{DispatchStatus, HleRuntime, HleValue};
use crate::core::hle::xmem::{GuestHeap, XMEM_HEAP_BASE};
use crate::core::hle::ImplementationState;
use crate::core::loader::pe64::{self, Pe64Error};
use crate::core::memory::{regions, GuestMemory, Memory};
use crate::core::pipeline::abi::{read_hle_arguments, AbiFault, CallingConvention};
use crate::core::pipeline::diagnostics::{describe_fault, DiagnosticEvent, Diagnostics};
use crate::core::pipeline::imports::{resolve_imports, ResolvedImport};

pub mod abi;
pub mod diagnostics;
pub mod imports;

/// Size of the guest stack reserved below the xmem heap region.
///
/// xHoly scaffold policy, not an Xbox limit.
pub const STACK_SIZE: u64 = 64 * 1024;

/// Return address the entry point's `ret` pops.
///
/// Pushed by [`GuestSession::load`] before the entry point runs; when
/// the guest returns here, execution stops with
/// [`StopReason::Returned`]. The value lies outside every mapped guest
/// region, so it can never be executed by accident.
pub const ENTRY_RETURN_SENTINEL: u64 = 0xFFFF_FFFF_FFFF_FF00;

/// Why a guest session could not be created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// `pe64::parse` rejected the bytes.
    Parse(Pe64Error),
    /// Structural validation failed (`pe64::validate`).
    InvalidImage(Pe64Error),
    /// Headers/sections could not be mapped.
    Mapping(Pe64Error),
    /// A base relocation could not be applied.
    Relocate(Pe64Error),
    /// The image declares no entry point.
    NoEntryPoint,
    /// Image plus stack do not fit the resident title window.
    ImageTooLarge {
        /// `SizeOfImage` in bytes.
        size: u64,
        /// Bytes available to the image below the stack.
        available: u64,
    },
    /// The initial stack frame could not be written.
    StackSetup(CpuFault),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "PE32+ parse failed: {e}"),
            Self::InvalidImage(e) => write!(f, "image failed structural validation: {e}"),
            Self::Mapping(e) => write!(f, "image mapping failed: {e}"),
            Self::Relocate(e) => write!(f, "base relocation failed: {e}"),
            Self::NoEntryPoint => f.write_str("image declares no entry point"),
            Self::ImageTooLarge { size, available } => write!(
                f,
                "image of {size} bytes does not fit the {available}-byte budget \
                 below the {STACK_SIZE}-byte stack"
            ),
            Self::StackSetup(fault) => write!(f, "initial stack frame failed: {fault}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Why execution stopped — exactly one cause is reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum StopReason {
    /// The entry point executed `ret` onto the pushed sentinel.
    Returned,
    /// The guest executed `HLT`.
    Halted,
    /// `rip` left the loaded image; no code is mapped there.
    ExitedImage,
    /// The step budget passed to [`GuestSession::run`] was exhausted.
    StepsExhausted,
    /// The import is recognised as unsupported: no value was returned.
    UnsupportedImport {
        /// Importing module.
        module: String,
        /// Symbol that could not be dispatched.
        symbol: String,
    },
    /// The import has no verified verdict: no value was returned.
    UnknownImport {
        /// Importing module.
        module: String,
        /// Symbol that could not be dispatched.
        symbol: String,
    },
    /// The implementation ran and failed.
    HleFailure {
        /// Importing module.
        module: String,
        /// Symbol whose implementation failed.
        symbol: String,
        /// Runtime failure reason.
        detail: String,
    },
    /// The CPU raised a structured fault.
    Fault(CpuFault),
    /// An ABI rule (stack alignment, shadow space, stack args, convention) failed.
    AbiFault(AbiFault),
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Returned => f.write_str("returned from entry point"),
            Self::Halted => f.write_str("guest executed HLT"),
            Self::ExitedImage => f.write_str("instruction pointer left the loaded image"),
            Self::StepsExhausted => f.write_str("step budget exhausted"),
            Self::UnsupportedImport { module, symbol } => {
                write!(f, "unsupported import {module}!{symbol}")
            }
            Self::UnknownImport { module, symbol } => write!(f, "unknown import {module}!{symbol}"),
            Self::HleFailure {
                module,
                symbol,
                detail,
            } => write!(f, "HLE dispatch failed for {module}!{symbol}: {detail}"),
            Self::Fault(fault) => write!(f, "CPU fault: {fault}"),
            Self::AbiFault(fault) => write!(f, "ABI fault: {fault}"),
        }
    }
}

/// Metadata of the loaded executable (diagnostics/report).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageMeta {
    /// Label only (file or fixture name) — never a host path.
    pub name: String,
    /// Guest VA the image is mapped at.
    pub base: u64,
    /// `SizeOfImage` in bytes.
    pub size: u64,
    /// Absolute guest VA of the entry point.
    pub entry_va: u64,
    /// COFF machine field (`0x8664` for x86-64).
    pub machine: u16,
    /// Evidence-based Xbox GDK hint.
    pub xbox_gdk: bool,
}

/// Structured result of one guest run (report + trace viewer feed).
///
/// `Serialize` only: reports are written for external consumers; xHoly
/// never parses them back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionReport {
    /// Executable metadata.
    pub image: ImageMeta,
    /// Every import with its verdict and IAT slot.
    pub imports: Vec<ResolvedImport>,
    /// Why execution stopped.
    pub stop: StopReason,
    /// Instructions retired in this run.
    pub steps: u64,
    /// CPU state when execution stopped (IP, flags, registers).
    pub cpu: crate::core::cpu::x64::CpuSnapshot,
    /// Ordered diagnostics: load, imports, dispatches, faults, stop.
    pub events: Vec<DiagnosticEvent>,
}

impl ExecutionReport {
    /// Human-readable report. Contains no compatibility percentage:
    /// import coverage is not game compatibility.
    pub fn render_text(&self) -> String {
        use std::fmt::Write as _;
        let mut o = String::new();
        let _ = writeln!(o, "Guest execution: {}", self.image.name);
        let _ = writeln!(
            o,
            "  image: base={:#x} size={:#x} entry={:#x} machine={:#06x} xbox_gdk={}",
            self.image.base,
            self.image.size,
            self.image.entry_va,
            self.image.machine,
            self.image.xbox_gdk
        );
        let (mut implemented, mut partial, mut unsupported, mut unknown) =
            (0usize, 0usize, 0usize, 0usize);
        for import in &self.imports {
            match import.state {
                ImplementationState::Implemented => implemented += 1,
                ImplementationState::PartiallyImplemented => partial += 1,
                ImplementationState::Unsupported => unsupported += 1,
                ImplementationState::Unknown => unknown += 1,
            }
        }
        let _ = writeln!(
            o,
            "  imports: {} resolved (implemented {implemented}, partially {partial}, \
             unsupported {unsupported}, unknown {unknown})",
            self.imports.len()
        );
        for import in &self.imports {
            let slot = match import.slot_va {
                Some(va) => format!("{va:#x}"),
                None => "no IAT slot".into(),
            };
            let _ = writeln!(
                o,
                "    - {}!{} slot={slot} [{}]",
                import.module,
                import.symbol,
                import.state.label()
            );
            if !import.evidence.is_empty() {
                let _ = writeln!(o, "      evidence: {}", import.evidence);
            }
        }
        let _ = writeln!(o, "  stop: {} after {} step(s)", self.stop, self.steps);
        let _ = writeln!(o, "  cpu:");
        for line in self.cpu.render().lines() {
            let _ = writeln!(o, "    {line}");
        }
        let _ = writeln!(o, "  trace:");
        if self.events.is_empty() {
            let _ = writeln!(o, "    no diagnostic events");
        }
        for event in &self.events {
            let _ = writeln!(o, "    {}", event.render_line());
        }
        o
    }

    /// Every recorded HLE dispatch, in order.
    pub fn dispatches(&self) -> impl Iterator<Item = &DiagnosticEvent> {
        self.events
            .iter()
            .filter(|e| matches!(e, DiagnosticEvent::HleDispatch { .. }))
    }
}

/// Routed guest address space: the flat resident title window plus the
/// xmem guest heap.
///
/// Addresses inside `[heap.base(), heap.base() + heap.size())` route to
/// the heap (allocation bounds and protection metadata are enforced
/// there); every other address routes to the flat window, which rejects
/// anything outside `[regions::TITLE, regions::TITLE + 1 GiB)` with a
/// structured failure instead of panicking. Guest and host pointers
/// stay strictly apart — this type only ever sees guest addresses.
pub struct GuestSpace<'a> {
    /// Flat resident window (image + stack).
    pub flat: &'a mut Memory,
    /// xmem guest heap.
    pub heap: &'a mut GuestHeap,
}

impl GuestSpace<'_> {
    /// Address range owned by the heap.
    fn heap_range(&self) -> std::ops::Range<u64> {
        self.heap.base()..self.heap.base().saturating_add(self.heap.size())
    }
}

impl GuestMemory for GuestSpace<'_> {
    fn read_bytes(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
        if self.heap_range().contains(&addr) {
            self.heap.read_bytes(addr, len)
        } else {
            self.flat.read_bytes(addr, len)
        }
    }

    fn write_bytes(&mut self, addr: u64, data: &[u8]) -> bool {
        if self.heap_range().contains(&addr) {
            self.heap.write_bytes(addr, data)
        } else {
            self.flat.write_bytes(addr, data)
        }
    }
}

/// One loaded guest ready to execute.
pub struct GuestSession {
    image: ImageMeta,
    memory: Memory,
    cpu: X64Cpu,
    runtime: HleRuntime,
    /// IAT slot guest VA → index into `imports` (the CPU's HLE
    /// transition points; the CPU never sees module or symbol).
    slots: BTreeMap<u64, usize>,
    imports: Vec<ResolvedImport>,
    calling_convention: CallingConvention,
    diagnostics: Diagnostics,
}

impl fmt::Debug for GuestSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Guest memory is not dumped: the report carries metadata only.
        f.debug_struct("GuestSession")
            .field("image", &self.image)
            .field("rip", &self.cpu.rip)
            .field("imports", &self.imports.len())
            .field("hle_slots", &self.slots.len())
            .finish_non_exhaustive()
    }
}

impl GuestSession {
    /// Load a PE32+ image into guest memory and resolve its imports.
    ///
    /// Steps: parse → structural validation → map sections → fit check
    /// (image + stack inside the resident window) → base relocations →
    /// write into guest memory → resolve IAT slots against `runtime`'s
    /// registry → CPU setup (entry, stack, transition slots).
    ///
    /// `runtime` is taken by value: the session owns it while it runs.
    /// Nothing is executed here.
    pub fn load(name: &str, bytes: &[u8], runtime: HleRuntime) -> Result<Self, LoadError> {
        let image = pe64::parse(bytes).map_err(LoadError::Parse)?;
        pe64::validate(&image).map_err(LoadError::InvalidImage)?;
        if image.entry_rva == 0 {
            return Err(LoadError::NoEntryPoint);
        }

        // Layout: image at the bottom of the resident window, stack
        // directly below the xmem heap region. Checked before anything
        // is allocated.
        let base = regions::TITLE;
        let window = XMEM_HEAP_BASE - base;
        let available = window - STACK_SIZE;
        let size = u64::from(image.size_of_image);
        if size > available {
            return Err(LoadError::ImageTooLarge { size, available });
        }

        let mut mapped = pe64::map_image(&image).map_err(LoadError::Mapping)?;
        pe64::apply_relocations(&mut mapped, &image, base).map_err(LoadError::Relocate)?;

        let mut memory = Memory::new();
        memory.map_image(&mapped);
        if memory.image_size() != mapped.len() {
            return Err(LoadError::ImageTooLarge {
                size: mapped.len() as u64,
                available,
            });
        }

        // Imports: one verdict per symbol, bound to its IAT slot.
        let imports = resolve_imports(&image, runtime.registry(), base);
        let mut slots = BTreeMap::new();
        for (index, import) in imports.iter().enumerate() {
            if let Some(slot) = import.slot_va {
                slots.insert(slot, index);
            }
        }

        // CPU: entry point, stack with the return sentinel, and the
        // registered IAT slots as HLE transition points.
        let entry_va = base + u64::from(image.entry_rva);
        let stack_bottom = XMEM_HEAP_BASE - STACK_SIZE;
        let stack_top = XMEM_HEAP_BASE;
        let mut cpu = X64Cpu::new();
        cpu.rip = entry_va;
        cpu.set_stack_bounds(StackBounds::new(stack_bottom, stack_top));
        // The stack grows downwards from the top of the resident
        // window (one byte past its last addressable byte is fine: the
        // first push writes below it).
        cpu.regs[reg::RSP] = stack_top;
        cpu.push(&mut memory, ENTRY_RETURN_SENTINEL, entry_va)
            .map_err(LoadError::StackSetup)?;
        cpu.set_hle_slots(slots.keys().copied());

        let mut diagnostics = Diagnostics::new();
        diagnostics.push(DiagnosticEvent::Loaded {
            base,
            size,
            entry_va,
            imports: imports.len(),
            xbox_gdk: image.looks_like_xbox_gdk(),
        });
        diagnostics.push(DiagnosticEvent::StackInitialized {
            bottom: stack_bottom,
            top: stack_top,
            initial_rsp: cpu.regs[reg::RSP],
            sentinel: ENTRY_RETURN_SENTINEL,
        });
        for import in &imports {
            diagnostics.push(DiagnosticEvent::ImportResolved {
                module: import.module.clone(),
                symbol: import.symbol.clone(),
                slot_va: import.slot_va,
                state: import.state,
            });
        }

        Ok(Self {
            image: ImageMeta {
                name: name.into(),
                base,
                size,
                entry_va,
                machine: image.machine,
                xbox_gdk: image.looks_like_xbox_gdk(),
            },
            memory,
            cpu,
            runtime,
            slots,
            imports,
            calling_convention: CallingConvention::default(),
            diagnostics,
        })
    }

    /// Loaded image metadata.
    pub fn image(&self) -> &ImageMeta {
        &self.image
    }

    /// CPU state (IP, flags, registers).
    pub fn cpu(&self) -> &X64Cpu {
        &self.cpu
    }

    /// The HLE runtime driving dispatches.
    pub fn runtime(&self) -> &HleRuntime {
        &self.runtime
    }

    /// Mutable HLE runtime access (tests and future shims).
    pub fn runtime_mut(&mut self) -> &mut HleRuntime {
        &mut self.runtime
    }

    /// Resolved imports in IAT order.
    pub fn imports(&self) -> &[ResolvedImport] {
        &self.imports
    }

    /// Ordered diagnostics (load, imports, dispatches, faults, stop).
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Configured calling convention for HLE transitions.
    pub fn calling_convention(&self) -> CallingConvention {
        self.calling_convention
    }

    /// Configure the calling convention used for HLE transitions.
    pub fn set_calling_convention(&mut self, convention: CallingConvention) {
        self.calling_convention = convention;
    }

    /// The routed guest address space (flat window + xmem heap).
    pub fn space(&mut self) -> GuestSpace<'_> {
        GuestSpace {
            flat: &mut self.memory,
            heap: &mut self.runtime.context().xmem,
        }
    }

    /// Execute from the entry point until a stop condition.
    ///
    /// `max_steps` bounds the number of retired instructions; reaching
    /// it stops with [`StopReason::StepsExhausted`] instead of looping
    /// forever. Every stop reason comes from an actual result — the
    /// report never claims more than what ran.
    pub fn run(&mut self, max_steps: u64) -> ExecutionReport {
        let mut steps = 0u64;
        let reason = loop {
            if self.cpu.rip == ENTRY_RETURN_SENTINEL {
                break StopReason::Returned;
            }
            let image_end = self.image.base + self.image.size;
            if !(self.image.base..image_end).contains(&self.cpu.rip) {
                break StopReason::ExitedImage;
            }
            if steps >= max_steps {
                break StopReason::StepsExhausted;
            }
            // The façade borrows memory and heap for one step; both
            // borrows end before dispatch borrows the runtime again.
            let outcome = {
                let mut space = GuestSpace {
                    flat: &mut self.memory,
                    heap: &mut self.runtime.context().xmem,
                };
                self.cpu.step(&mut space)
            };
            match outcome {
                // Only retired instructions count towards the budget;
                // a faulting instruction never retires.
                StepOutcome::Executed => steps += 1,
                StepOutcome::Halted => {
                    steps += 1;
                    break StopReason::Halted;
                }
                StepOutcome::Fault(fault) => {
                    self.diagnostics.push(DiagnosticEvent::CpuFault {
                        rip: fault_rip(&fault),
                        description: describe_fault(&fault),
                    });
                    break StopReason::Fault(fault);
                }
                StepOutcome::HleCall { slot_addr } => {
                    steps += 1;
                    if let Some(stop) = self.dispatch(slot_addr) {
                        break stop;
                    }
                }
            }
        };
        self.diagnostics.push(DiagnosticEvent::Stopped {
            reason: reason.to_string(),
            steps,
        });
        ExecutionReport {
            image: self.image.clone(),
            imports: self.imports.clone(),
            stop: reason,
            steps,
            cpu: self.cpu.snapshot(),
            events: self.diagnostics.events().to_vec(),
        }
    }

    /// Handle one HLE transition. Returns `Some(stop)` when the call
    /// ends execution, `None` when the guest may continue.
    fn dispatch(&mut self, slot_addr: u64) -> Option<StopReason> {
        let import = match self.slots.get(&slot_addr).copied() {
            Some(index) => self.imports[index].clone(),
            // The CPU only reports registered slots; this guard fires
            // if the two tables ever drift apart.
            None => {
                return Some(StopReason::Fault(CpuFault::IllegalInstruction {
                    rip: self.cpu.rip,
                    opcode: 0xFF,
                    reason: "call through an unregistered HLE slot",
                }));
            }
        };

        // Arguments come from the configured calling convention
        // (`CallingConvention::MicrosoftX64` reads RCX, RDX, R8, R9 and
        // stack arguments 4.. after the 32-byte shadow space). Symbols
        // with no registered implementation receive no arguments.
        let arg_values = match self
            .runtime
            .implementation_arity(&import.module, &import.symbol)
        {
            Some(arity) => {
                let arity = usize::from(arity);
                let extracted = {
                    let space = GuestSpace {
                        flat: &mut self.memory,
                        heap: &mut self.runtime.context().xmem,
                    };
                    read_hle_arguments(&self.cpu, &space, self.calling_convention, arity)
                };
                match extracted {
                    Ok(values) => {
                        if values.len() > crate::core::cpu::x64::ARG_REGISTERS.len() {
                            let stack_vals =
                                values[crate::core::cpu::x64::ARG_REGISTERS.len()..].to_vec();
                            self.diagnostics.push(DiagnosticEvent::StackArgumentsRead {
                                module: import.module.clone(),
                                symbol: import.symbol.clone(),
                                rsp: self.cpu.regs[reg::RSP],
                                count: stack_vals.len(),
                                values: stack_vals,
                            });
                        }
                        values
                    }
                    Err(abi_fault) => {
                        self.diagnostics.push(DiagnosticEvent::AbiFault {
                            rip: self.cpu.rip,
                            description: abi_fault.to_string(),
                        });
                        return Some(StopReason::AbiFault(abi_fault));
                    }
                }
            }
            None => Vec::new(),
        };
        let hle_args: Vec<HleValue> = arg_values.iter().copied().map(HleValue::Unsigned).collect();
        let result = self
            .runtime
            .dispatch(&import.module, &import.symbol, &hle_args);
        let value = result.value.map(hle_value_as_u64);
        self.diagnostics.push(DiagnosticEvent::HleDispatch {
            module: import.module.clone(),
            symbol: import.symbol.clone(),
            args: arg_values,
            status: result.status,
            value,
            detail: result.detail.clone(),
        });

        match result.status {
            DispatchStatus::Completed => {
                // Play the callee's half of the call: pop the return
                // address the transition pushed, then write the result
                // through the x64 ABI.
                let popped = {
                    let space = GuestSpace {
                        flat: &mut self.memory,
                        heap: &mut self.runtime.context().xmem,
                    };
                    self.cpu.finish_hle_call(&space)
                };
                match popped {
                    // The transition pushed exactly the address `rip`
                    // now holds; the stack is balanced again.
                    Ok(_) => {}
                    Err(fault) => {
                        self.diagnostics.push(DiagnosticEvent::CpuFault {
                            rip: fault_rip(&fault),
                            description: describe_fault(&fault),
                        });
                        return Some(StopReason::Fault(fault));
                    }
                }
                if let Some(value) = value {
                    self.cpu.write_return(value);
                }
                None
            }
            DispatchStatus::Unsupported => Some(StopReason::UnsupportedImport {
                module: import.module,
                symbol: import.symbol,
            }),
            DispatchStatus::Unknown => Some(StopReason::UnknownImport {
                module: import.module,
                symbol: import.symbol,
            }),
            DispatchStatus::Failed => Some(StopReason::HleFailure {
                module: import.module,
                symbol: import.symbol,
                detail: result.detail,
            }),
        }
    }
}

/// Map an HLE return value onto the x64 return register.
fn hle_value_as_u64(value: HleValue) -> u64 {
    match value {
        HleValue::Null => 0,
        HleValue::Unsigned(v) => v,
        HleValue::Signed(v) => v as u64,
        HleValue::Boolean(b) => u64::from(b),
    }
}

/// Instruction pointer recorded for a fault.
fn fault_rip(fault: &CpuFault) -> u64 {
    match fault {
        CpuFault::FetchOutOfBounds { rip } => *rip,
        CpuFault::IllegalInstruction { rip, .. } => *rip,
        CpuFault::Memory { rip, .. } => *rip,
        CpuFault::StackOverflow { rip, .. } => *rip,
        CpuFault::StackUnderflow { rip, .. } => *rip,
        CpuFault::MisalignedStack { rip, .. } => *rip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cpu::x64::CpuFault;
    use crate::core::hle::runtime::DispatchStatus;
    use crate::core::loader::pe64;
    use crate::core::memory::GuestMemory;

    const WINDOW: u64 = XMEM_HEAP_BASE - regions::TITLE;

    /// Entry code lives at file offset 0x400 == RVA 0x1000.
    fn with_entry_code(mut bytes: Vec<u8>, code: &[u8]) -> Vec<u8> {
        bytes[0x400..0x400 + code.len()].copy_from_slice(code);
        bytes
    }

    fn load(bytes: &[u8]) -> GuestSession {
        GuestSession::load("fixture.exe", bytes, HleRuntime::with_foundation())
            .expect("fixture must load")
    }

    #[test]
    fn rejects_non_pe_bytes_without_panicking() {
        let err = GuestSession::load("bad.exe", b"NOPE", HleRuntime::with_foundation())
            .expect_err("not a PE");
        assert!(
            matches!(err, LoadError::Parse(pe64::Pe64Error::NotPe)),
            "{err}"
        );
        assert!(err.to_string().contains("parse failed"), "{err}");
    }

    #[test]
    fn rejects_structurally_broken_images() {
        // Entry point outside SizeOfImage.
        let mut bytes = pe64::synthetic_pe();
        bytes[0xA8..0xAC].copy_from_slice(&0x3000u32.to_le_bytes()); // opt+16
        let err = GuestSession::load("bad.exe", &bytes, HleRuntime::with_foundation())
            .expect_err("entry outside image");
        assert!(
            matches!(
                err,
                LoadError::InvalidImage(pe64::Pe64Error::Malformed(
                    "entry point outside the image"
                ))
            ),
            "{err}"
        );

        // Section spilling past SizeOfImage (0x1800 keeps the entry
        // point inside so the section check is the one that fires).
        let mut bytes = pe64::synthetic_pe();
        bytes[0xD0..0xD4].copy_from_slice(&0x1800u32.to_le_bytes()); // opt+56
        let err = GuestSession::load("bad.exe", &bytes, HleRuntime::with_foundation())
            .expect_err("section exceeds image");
        assert!(
            matches!(
                err,
                LoadError::InvalidImage(pe64::Pe64Error::Malformed("section exceeds SizeOfImage"))
            ),
            "{err}"
        );
    }

    #[test]
    fn rejects_images_that_do_not_fit_the_resident_window() {
        // SizeOfImage just above the budget below the stack.
        let mut bytes = pe64::synthetic_pe();
        let available = WINDOW - STACK_SIZE;
        bytes[0xD0..0xD4].copy_from_slice(&((available + 1) as u32).to_le_bytes());
        let err = GuestSession::load("huge.exe", &bytes, HleRuntime::with_foundation())
            .expect_err("does not fit");
        match err {
            LoadError::ImageTooLarge {
                size,
                available: budget,
            } => {
                assert_eq!(size, available + 1);
                assert_eq!(budget, available);
            }
            other => panic!("expected ImageTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn synthetic_guest_returns_to_the_sentinel() {
        let bytes = pe64::synthetic_pe();
        let mut session = load(&bytes);
        assert_eq!(session.image().base, regions::TITLE);
        assert_eq!(session.image().entry_va, regions::TITLE + 0x1000);
        assert!(session.image().xbox_gdk, "fixture imports d3d12_xs.dll");

        let report = session.run(10_000);
        assert_eq!(report.stop, StopReason::Returned);
        assert_eq!(report.steps, 2, "mov eax,42 ; ret");
        assert_eq!(report.cpu.register(reg::RAX), 42);
        assert_eq!(
            session.cpu().regs[reg::RSP],
            XMEM_HEAP_BASE,
            "the return sentinel was popped: stack balanced"
        );
        // Trace: load, one import, stop.
        assert!(matches!(
            report.events.first(),
            Some(DiagnosticEvent::Loaded { .. })
        ));
        assert!(
            report
                .events
                .iter()
                .any(|e| matches!(e, DiagnosticEvent::ImportResolved { .. })),
            "import verdicts must be recorded"
        );
        assert!(
            matches!(report.events.last(), Some(DiagnosticEvent::Stopped { .. })),
            "the trace ends with the stop event"
        );
        assert_eq!(report.dispatches().count(), 0, "nothing was called");
    }

    #[test]
    fn hle_call_runs_end_to_end_and_preserves_cpu_state() {
        let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
        let mut session = load(&bytes);
        let report = session.run(10_000);

        // The guest called XMemAlloc(64, 0) and wrote 0x11 through the
        // returned pointer, then returned from the entry point.
        assert_eq!(report.stop, StopReason::Returned);
        assert_eq!(report.steps, 5);
        let heap_addr = report.cpu.register(reg::RAX);
        assert_eq!(heap_addr % 16, 0, "XMemAlloc honours 16-byte alignment");
        assert!(
            heap_addr >= XMEM_HEAP_BASE,
            "result is a guest heap address"
        );
        // The store went through the routed address space into the heap.
        let stored = session
            .space()
            .read_u64(heap_addr)
            .expect("heap allocation must be readable");
        assert_eq!(stored, 0x11, "guest wrote 0x11 through the pointer");

        // CPU state survived the HLE transition: arguments were passed
        // in RCX/RDX and are untouched afterwards.
        assert_eq!(report.cpu.register(reg::RCX), 64);
        assert_eq!(report.cpu.register(reg::RDX), 0);
        assert_eq!(
            session.cpu().regs[reg::RSP],
            XMEM_HEAP_BASE,
            "the HLE frame was popped: stack balanced across the call"
        );

        // Dispatch recorded with arguments, value and evidence.
        let dispatch = report
            .dispatches()
            .find_map(|e| match e {
                DiagnosticEvent::HleDispatch {
                    module,
                    symbol,
                    args,
                    status,
                    value,
                    detail,
                } => Some((
                    module.clone(),
                    symbol.clone(),
                    args.clone(),
                    *status,
                    *value,
                    detail.clone(),
                )),
                _ => None,
            })
            .expect("one dispatch must be recorded");
        assert_eq!(dispatch.0, "xmem.dll");
        assert_eq!(dispatch.1, "XMemAlloc");
        assert_eq!(dispatch.2, vec![64, 0], "arguments from RCX/RDX");
        assert_eq!(dispatch.3, DispatchStatus::Completed);
        assert_eq!(dispatch.4, Some(heap_addr));
        assert!(dispatch.5.contains("implemented"), "{}", dispatch.5);

        // Verdicts also appear in the import list.
        assert_eq!(report.imports.len(), 1);
        assert_eq!(
            report.imports[0].state,
            ImplementationState::PartiallyImplemented
        );
    }

    #[test]
    fn unsupported_import_stops_without_a_fake_return() {
        let bytes = pe64::synthetic_pe_calling("d3d12_xs.dll", "D3D12CreateDevice");
        let mut session = load(&bytes);
        let report = session.run(10_000);

        assert_eq!(
            report.stop,
            StopReason::UnsupportedImport {
                module: "d3d12_xs.dll".into(),
                symbol: "D3D12CreateDevice".into(),
            }
        );
        // No fabricated return value: RAX still holds what the guest
        // left there (zero — nothing wrote to it).
        assert_eq!(report.cpu.register(reg::RAX), 0);
        let status = report.dispatches().find_map(|e| match e {
            DiagnosticEvent::HleDispatch {
                status,
                args,
                value,
                ..
            } => Some((*status, args.clone(), *value)),
            _ => None,
        });
        assert_eq!(
            status,
            Some((DispatchStatus::Unsupported, Vec::new(), None)),
            "unsupported calls receive no arguments and return no value"
        );
    }

    #[test]
    fn unknown_import_stops_execution_and_is_preserved() {
        let bytes = pe64::synthetic_pe_calling("mystery.dll", "DoSomething");
        let mut session = load(&bytes);
        let report = session.run(10_000);
        assert_eq!(
            report.stop,
            StopReason::UnknownImport {
                module: "mystery.dll".into(),
                symbol: "DoSomething".into(),
            }
        );
        assert_eq!(report.imports[0].state, ImplementationState::Unknown);
        assert_eq!(report.cpu.register(reg::RAX), 0, "no fake return value");
        assert!(
            report
                .render_text()
                .contains("unknown import mystery.dll!DoSomething"),
            "the report names the unresolved import"
        );
    }

    #[test]
    fn explicit_unknown_verdict_wins_over_the_unsupported_module() {
        let bytes = pe64::synthetic_pe_calling("d3d12_xs.dll", "D3D12XboxCreateDevice");
        let mut session = load(&bytes);
        let report = session.run(10_000);
        assert_eq!(
            report.stop,
            StopReason::UnknownImport {
                module: "d3d12_xs.dll".into(),
                symbol: "D3D12XboxCreateDevice".into(),
            },
            "the explicit research verdict decides, not the module state"
        );
        assert_eq!(report.imports[0].state, ImplementationState::Unknown);
    }

    #[test]
    fn runtime_failure_stops_execution_with_the_failure_detail() {
        // The fixture calls XMemFree(64, 0): address 64 was never
        // allocated, so the implementation runs and fails.
        let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemFree");
        let mut session = load(&bytes);
        let report = session.run(10_000);
        match &report.stop {
            StopReason::HleFailure {
                module,
                symbol,
                detail,
            } => {
                assert_eq!(module, "xmem.dll");
                assert_eq!(symbol, "XMemFree");
                assert!(detail.contains("runtime failure"), "{detail}");
            }
            other => panic!("expected HleFailure, got {other:?}"),
        }
        assert_eq!(report.cpu.register(reg::RAX), 0, "no value on failure");
    }

    #[test]
    fn step_budget_stops_a_looping_guest() {
        // jmp $ — an endless loop inside the image.
        let bytes = with_entry_code(pe64::synthetic_pe(), &[0xEB, 0xFE]);
        let mut session = load(&bytes);
        let report = session.run(10);
        assert_eq!(report.stop, StopReason::StepsExhausted);
        assert_eq!(report.steps, 10);
        assert_eq!(
            session.cpu().rip,
            session.image().entry_va,
            "the loop never left the entry instruction"
        );
    }

    #[test]
    fn unsupported_instruction_is_a_structured_fault() {
        // lea rax, [rax] — outside the supported subset.
        let bytes = with_entry_code(pe64::synthetic_pe(), &[0x48, 0x8D, 0x00]);
        let mut session = load(&bytes);
        let report = session.run(100);
        match &report.stop {
            StopReason::Fault(CpuFault::IllegalInstruction { opcode, rip, .. }) => {
                assert_eq!(*opcode, 0x8D);
                assert_eq!(*rip, session.image().entry_va);
            }
            other => panic!("expected illegal instruction, got {other:?}"),
        }
        assert_eq!(report.steps, 0, "a faulting instruction does not retire");
        assert!(
            report
                .events
                .iter()
                .any(|e| matches!(e, DiagnosticEvent::CpuFault { .. })),
            "the fault must appear in the trace"
        );
        assert!(
            report.render_text().contains("CPU fault:"),
            "{}",
            report.render_text()
        );
    }

    #[test]
    fn unmapped_write_is_a_memory_fault() {
        // mov qword ptr [rax], 1 with rax = 0: address 0 is in neither
        // the resident window nor the heap.
        let bytes = with_entry_code(
            pe64::synthetic_pe(),
            &[0x48, 0xC7, 0x00, 0x01, 0x00, 0x00, 0x00],
        );
        let mut session = load(&bytes);
        let report = session.run(100);
        assert_eq!(
            report.stop,
            StopReason::Fault(CpuFault::Memory {
                addr: 0,
                write: true,
                rip: session.image().entry_va,
            })
        );
        assert_eq!(report.steps, 0);
    }

    #[test]
    fn execution_leaving_the_image_stops_explicitly() {
        // jmp to an address far outside the image.
        let bytes = with_entry_code(
            pe64::synthetic_pe(),
            &[0xE9, 0x00, 0x00, 0x10, 0x00], // jmp +0x100000
        );
        let mut session = load(&bytes);
        let report = session.run(100);
        assert_eq!(report.stop, StopReason::ExitedImage);
        assert_eq!(report.steps, 1, "the jump itself retired");
    }

    #[test]
    fn hlt_stops_with_the_halted_state() {
        let bytes = with_entry_code(pe64::synthetic_pe(), &[0xF4]);
        let mut session = load(&bytes);
        let report = session.run(100);
        assert_eq!(report.stop, StopReason::Halted);
        assert!(report.cpu.halted);
        assert_eq!(report.steps, 1);
    }

    #[test]
    fn report_renders_metadata_imports_cpu_state_and_trace() {
        let bytes = pe64::synthetic_pe_calling("xmem.dll", "XMemAlloc");
        let mut session = load(&bytes);
        let report = session.run(10_000);
        let text = report.render_text();

        assert!(text.contains("Guest execution: fixture.exe"), "{text}");
        assert!(
            text.contains(&format!("base={:#x}", regions::TITLE)),
            "{text}"
        );
        assert!(text.contains("machine=0x8664"), "{text}");
        assert!(text.contains("xbox_gdk=true"), "{text}");
        assert!(text.contains("xmem.dll!XMemAlloc"), "{text}");
        assert!(text.contains("[Partially Implemented]"), "{text}");
        assert!(text.contains("evidence:"), "{text}");
        assert!(text.contains("stop: returned from entry point"), "{text}");
        assert!(text.contains("rip=0x"), "{text}");
        assert!(text.contains("[hle] xmem.dll!XMemAlloc"), "{text}");
        assert!(text.contains("[stop]"), "{text}");
        // Honest reporting: no compatibility percentage anywhere.
        assert!(!text.contains('%'), "{text}");
    }

    #[test]
    fn guest_space_routes_by_address_range() {
        let mut session = load(&pe64::synthetic_pe());
        let image_addr = regions::TITLE + 0x1000;
        // Flat window: the mapped image is readable.
        assert_eq!(
            session.space().read_bytes(image_addr, 4).as_deref(),
            Some(&[0xB8, 42, 0, 0][..])
        );
        // Unmapped guest address: structured failure, no panic.
        assert_eq!(session.space().read_bytes(regions::HYPERVISOR, 8), None);
        assert!(!session.space().write_bytes(regions::IO_APERTURE, b"x"));

        // Heap range: only live allocations resolve.
        let heap_addr = {
            let mut space = session.space();
            assert!(!space.write_bytes(XMEM_HEAP_BASE, b"nope"), "unallocated");
            space
                .heap
                .alloc(64, 16, 0)
                .expect("heap must accept an allocation")
        };
        let mut space = session.space();
        assert!(space.write_bytes(heap_addr, b"heap-data"));
        assert_eq!(
            space.read_bytes(heap_addr, 9).as_deref(),
            Some(b"heap-data".as_slice())
        );
        // A range straddling the window/heap boundary is rejected as a
        // whole instead of leaking into the other region.
        assert!(!space.write_bytes(XMEM_HEAP_BASE - 4, b"12345678"));
        assert_eq!(space.read_bytes(XMEM_HEAP_BASE - 4, 8), None);
    }
}
