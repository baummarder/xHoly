//! Gen9 HLE dispatch runtime (Phase 28).
//!
//! Connects implementations to the **single** metadata registry in
//! [`super::HleRegistry`]: the registry says what xHoly claims, this
//! module holds what xHoly can actually call, and [`HleRuntime::dispatch`]
//! reconciles the two. There is deliberately no second registry.
//!
//! Dispatch rules:
//!
//! * an registered implementation runs and either completes or reports a
//!   **runtime failure** ([`DispatchStatus::Failed`]);
//! * a registry entry marked `Unsupported` returns a structured
//!   [`DispatchStatus::Unsupported`] result — never a fake success value;
//! * a registry entry marked `Unknown` (no reliable public documentation)
//!   returns [`DispatchStatus::Unknown`];
//! * symbols with no registry entry are preserved as `Unknown` unless the
//!   owning module is explicitly `Unsupported`.
//!
//! Unsupported and unknown results never carry a return value: nothing is
//! silently substituted for a missing implementation.

use super::xmem::{GuestHeap, HeapError, DEFAULT_ALIGNMENT};
use super::{HleRegistry, ImplementationState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// A value passed to or returned from an HLE function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HleValue {
    /// Void return (also the zero/NULL guest value).
    Null,
    /// Unsigned guest word (guest addresses are returned as this).
    Unsigned(u64),
    /// Signed guest word.
    Signed(i64),
    /// Boolean guest value.
    Boolean(bool),
}

/// A runtime failure raised *by* an implementation.
///
/// This is distinct from "unsupported": the function exists and was
/// called, but the call itself failed (bad arguments, exhausted guest
/// memory, internal invariant broken).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HleFault {
    /// Wrong number or shape of arguments.
    InvalidArgument(&'static str),
    /// Guest address problem reported by the heap.
    InvalidAddress(u64),
    /// Guest heap exhausted.
    OutOfMemory,
    /// Other guest-memory failure.
    Memory(HeapError),
    /// Implementation-internal failure.
    Internal(String),
}

impl fmt::Display for HleFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(what) => write!(f, "invalid arguments: {what}"),
            Self::InvalidAddress(a) => write!(f, "invalid guest address {a:#x}"),
            Self::OutOfMemory => f.write_str("guest heap exhausted"),
            Self::Memory(e) => write!(f, "guest memory: {e}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

/// Outcome of one dispatch attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchStatus {
    /// The implementation ran and succeeded; `value` is set.
    Completed,
    /// The registry recognises the symbol but xHoly provides no
    /// implementation; `value` is always `None`.
    Unsupported,
    /// No reliable public documentation / no registry entry; `value` is
    /// always `None`.
    Unknown,
    /// The implementation ran and failed (or the registry claims an
    /// implementation that is not registered); `value` is `None`.
    Failed,
}

/// Structured result of one dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchResult {
    /// Requested module (as passed by the caller).
    pub module: String,
    /// Requested symbol (as passed by the caller).
    pub symbol: String,
    /// Outcome class.
    pub status: DispatchStatus,
    /// Return value — only `Some` for [`DispatchStatus::Completed`].
    pub value: Option<HleValue>,
    /// Human-readable reason / evidence (kept for existing reports).
    pub detail: String,
    /// Typed failure, when the dispatch did not complete (M36.1).
    pub failure: Option<DispatchFailure>,
}

/// Signature of an HLE implementation.
pub type HleFunction = fn(&mut HleContext, &[HleValue]) -> Result<HleValue, HleFault>;

/// State handed to implementations.
#[derive(Debug)]
pub struct HleContext {
    /// Guest heap backing the `xmem` shims.
    pub xmem: GuestHeap,
    /// Deterministic guest thread/event/mutex state (xHoly VM service).
    pub kernel: super::kernel::KernelContext,
    /// Deterministic VM tick/info state (xHoly VM service).
    pub system: super::system::SystemContext,
}

impl Default for HleContext {
    fn default() -> Self {
        Self {
            xmem: GuestHeap::with_defaults(),
            kernel: super::kernel::KernelContext::new(),
            system: super::system::SystemContext::new(),
        }
    }
}

/// Maximum number of integer arguments an HLE call may read from the
/// guest: the first four come from `RCX, RDX, R8, R9` and arguments
/// `4..` are read from the Microsoft x64 stack frame (after the 32-byte
/// shadow space). Bounded to 16 to prevent unbounded stack reads.
pub const MAX_HLE_ARGS: usize = 16;

/// Reason a registration was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterError {
    /// The registry has no such module+symbol.
    NotInRegistry,
    /// The registry does not claim this symbol is implemented.
    WrongState(ImplementationState),
    /// The declared arity exceeds [`MAX_HLE_ARGS`].
    ArityOutOfRange(u8),
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInRegistry => f.write_str("symbol is not in the HLE registry"),
            Self::WrongState(s) => write!(
                f,
                "registry state {} does not allow an implementation",
                s.label()
            ),
            Self::ArityOutOfRange(arity) => write!(
                f,
                "arity {arity} exceeds the {MAX_HLE_ARGS} integer argument registers"
            ),
        }
    }
}

/// Dispatch counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DispatchStats {
    /// Successful calls.
    pub completed: usize,
    /// Structured unsupported results.
    pub unsupported: usize,
    /// Preserved unknown results.
    pub unknown: usize,
    /// Runtime failures.
    pub failed: usize,
}

/// Maximum number of failure messages retained in the runtime log.
///
/// M36.1: guest input must never grow host memory without bound.
pub const MAX_HLE_FAILURES: usize = 64;
/// Maximum number of retained dispatch trace records.
pub const MAX_HLE_DISPATCH_TRACE: usize = 128;

/// Why a dispatch did not complete (typed, M36.1).
///
/// The prose `DispatchResult::detail` is kept for existing reports; this
/// enum preserves the typed distinction between a real implementation
/// fault and a registry claim without an implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchFailure {
    /// A registered implementation raised a fault.
    Fault(HleFault),
    /// The registry claims support but no implementation is registered.
    UnbackedClaim {
        /// Claimed state from the registry.
        state: ImplementationState,
    },
}

impl fmt::Display for DispatchFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fault(fault) => write!(f, "runtime failure: {fault}"),
            Self::UnbackedClaim { state } => write!(
                f,
                "registry marks {} but no implementation is registered",
                state.label()
            ),
        }
    }
}

impl std::error::Error for HleFault {}
impl std::error::Error for RegisterError {}
impl std::error::Error for DispatchFailure {}

/// One bounded dispatch record for diagnostics (M36.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRecord {
    /// Requested module.
    pub module: String,
    /// Requested symbol.
    pub symbol: String,
    /// Outcome class.
    pub status: DispatchStatus,
    /// Number of arguments passed to the dispatcher.
    pub args_count: usize,
    /// Whether a value was returned.
    pub has_value: bool,
    /// Short detail text (never guest memory contents).
    pub detail: String,
}

#[derive(Debug)]
struct Registered {
    func: HleFunction,
    evidence: &'static str,
    /// Number of integer arguments the implementation reads.
    arity: u8,
}

/// The Gen9 HLE dispatch runtime.
#[derive(Debug)]
pub struct HleRuntime {
    registry: HleRegistry,
    context: HleContext,
    impls: HashMap<(String, String), Registered>,
    stats: DispatchStats,
    failures: Vec<String>,
    /// Number of failures dropped by the bounded log.
    dropped_failures: usize,
    /// Bounded record of recent dispatches (M36.1).
    dispatch_trace: Vec<DispatchRecord>,
}

impl HleRuntime {
    /// Build a runtime around an existing registry (the single metadata
    /// source) with no implementations registered.
    pub fn new(registry: HleRegistry) -> Self {
        Self {
            registry,
            context: HleContext::default(),
            impls: HashMap::new(),
            stats: DispatchStats::default(),
            failures: Vec::new(),
            dropped_failures: 0,
            dispatch_trace: Vec::new(),
        }
    }

    /// Runtime with the current foundation implementations registered.
    pub fn with_foundation() -> Self {
        let mut runtime = Self::new(HleRegistry::with_known_entries());
        runtime
            .register("xmem.dll", "XMemAlloc", xmem_alloc, SHIM_EVIDENCE, 2)
            .expect("XMemAlloc must be a partially implemented registry entry");
        runtime
            .register("xmem.dll", "XMemFree", xmem_free, SHIM_EVIDENCE, 2)
            .expect("XMemFree must be a partially implemented registry entry");
        runtime
    }

    /// Runtime with the separate xHoly VM scaffold services registered.
    ///
    /// This includes the Xbox foundation shims plus `xh_vm.*` services.
    /// The names are deliberately not Xbox exports; see
    /// [`HleRegistry::with_all_entries`].
    pub fn with_xh_vm() -> Self {
        const VM_EVIDENCE: &str = "src/core/hle/ (xHoly VM scaffold service; NOT an Xbox API)";
        let mut runtime = Self::new(HleRegistry::with_all_entries());
        runtime
            .register("xmem.dll", "XMemAlloc", xmem_alloc, SHIM_EVIDENCE, 2)
            .expect("XMemAlloc registry entry");
        runtime
            .register("xmem.dll", "XMemFree", xmem_free, SHIM_EVIDENCE, 2)
            .expect("XMemFree registry entry");
        let services: [(&str, &str, HleFunction, u8); 17] = [
            ("xh_vm.mem.dll", "VmQuery", vm_query, 1),
            ("xh_vm.mem.dll", "VmProtect", vm_protect, 2),
            ("xh_vm.mem.dll", "VmStats", vm_stats, 0),
            ("xh_vm.sys.dll", "VmTicks", vm_ticks, 0),
            ("xh_vm.sys.dll", "VmInfo", vm_info, 0),
            ("xh_vm.krn.dll", "ThreadCreate", vm_thread_create, 0),
            ("xh_vm.krn.dll", "ThreadYield", vm_thread_yield, 1),
            ("xh_vm.krn.dll", "ThreadSleep", vm_thread_sleep, 2),
            ("xh_vm.krn.dll", "ThreadSuspend", vm_thread_suspend, 1),
            ("xh_vm.krn.dll", "ThreadResume", vm_thread_resume, 1),
            ("xh_vm.krn.dll", "ThreadExit", vm_thread_exit, 1),
            ("xh_vm.sync.dll", "EventCreate", vm_event_create, 0),
            ("xh_vm.sync.dll", "EventSet", vm_event_set, 1),
            ("xh_vm.sync.dll", "EventWait", vm_event_wait, 2),
            ("xh_vm.sync.dll", "MutexCreate", vm_mutex_create, 0),
            ("xh_vm.sync.dll", "MutexLock", vm_mutex_lock, 3),
            ("xh_vm.sync.dll", "MutexUnlock", vm_mutex_unlock, 2),
        ];
        for (module, symbol, func, arity) in services {
            runtime
                .register(module, symbol, func, VM_EVIDENCE, arity)
                .expect("xHoly VM service registry entry");
        }
        runtime
    }

    /// The single metadata registry.
    pub fn registry(&self) -> &HleRegistry {
        &self.registry
    }

    /// Mutable access to the guest context (tests and future shims).
    pub fn context(&mut self) -> &mut HleContext {
        &mut self.context
    }

    /// Read-only access to the guest context (reports and diagnostics).
    pub fn context_ref(&self) -> &HleContext {
        &self.context
    }

    /// Register an implementation with its integer argument count
    /// (`0..=`[`MAX_HLE_ARGS`]). Refused unless the registry already
    /// claims the symbol is implemented or partially implemented, so a
    /// placeholder can never be advertised as working.
    ///
    /// The arity is how many of `RCX, RDX, R8, R9` the guest execution
    /// pipeline reads before dispatching; it is metadata about the
    /// implementation, not a claim about any undocumented calling
    /// convention (see `crate::core::cpu::x64`).
    pub fn register(
        &mut self,
        module: &str,
        symbol: &str,
        func: HleFunction,
        evidence: &'static str,
        arity: u8,
    ) -> Result<(), RegisterError> {
        let meta = self
            .registry
            .function_meta(module, symbol)
            .ok_or(RegisterError::NotInRegistry)?;
        match meta.state {
            ImplementationState::Implemented | ImplementationState::PartiallyImplemented => {}
            other => return Err(RegisterError::WrongState(other)),
        }
        if usize::from(arity) > MAX_HLE_ARGS {
            return Err(RegisterError::ArityOutOfRange(arity));
        }
        self.impls.insert(
            (module.to_lowercase(), symbol.to_lowercase()),
            Registered {
                func,
                evidence,
                arity,
            },
        );
        Ok(())
    }

    /// Symbols that currently have a registered implementation.
    pub fn implemented_symbols(&self) -> Vec<(String, String)> {
        let mut keys: Vec<(String, String)> = self.impls.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Declared argument count of a registered implementation.
    pub fn implementation_arity(&self, module: &str, symbol: &str) -> Option<u8> {
        self.impls
            .get(&(module.to_lowercase(), symbol.to_lowercase()))
            .map(|r| r.arity)
    }

    /// Look up an implementation's evidence without calling it.
    pub fn implementation_evidence(&self, module: &str, symbol: &str) -> Option<&'static str> {
        self.impls
            .get(&(module.to_lowercase(), symbol.to_lowercase()))
            .map(|r| r.evidence)
    }

    /// Dispatch one call through the central runtime.
    ///
    /// M36.1: the verdict for unregistered symbols comes from the single
    /// resolver; every outcome is recorded in a bounded diagnostic trace.
    pub fn dispatch(&mut self, module: &str, symbol: &str, args: &[HleValue]) -> DispatchResult {
        let key = (module.to_lowercase(), symbol.to_lowercase());
        let result = if let Some(registered) = self.impls.get(&key) {
            let func = registered.func;
            let evidence = registered.evidence;
            match func(&mut self.context, args) {
                Ok(value) => {
                    self.stats.completed += 1;
                    // M35: add the guest-visible result/arguments to the
                    // diagnostic detail. No host pointer is ever exposed.
                    let detail = match (&value, args.first()) {
                        (HleValue::Unsigned(addr), Some(HleValue::Unsigned(attr)))
                            if symbol.eq_ignore_ascii_case("XMemAlloc") =>
                        {
                            format!("implemented: {evidence} guest={addr:#x} attrs={attr:#x}")
                        }
                        (HleValue::Null, Some(HleValue::Unsigned(addr)))
                            if symbol.eq_ignore_ascii_case("XMemFree") =>
                        {
                            format!("implemented: {evidence} freed={addr:#x}")
                        }
                        _ => format!("implemented: {evidence}"),
                    };
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Completed,
                        value: Some(value),
                        detail,
                        failure: None,
                    }
                }
                Err(fault) => {
                    self.stats.failed += 1;
                    let failure = DispatchFailure::Fault(fault);
                    let detail = failure.to_string();
                    self.push_failure(module, symbol, &detail);
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Failed,
                        value: None,
                        detail,
                        failure: Some(failure),
                    }
                }
            }
        } else {
            // The registry's own state decides whether an unbacked claim
            // is a failure, unsupported or unknown. Verdict precedence
            // still comes from the single resolver below.
            let verdict =
                crate::core::hle::resolve::ApiVerdict::resolve(&self.registry, module, symbol);
            match verdict.state {
                ImplementationState::Implemented | ImplementationState::PartiallyImplemented => {
                    self.stats.failed += 1;
                    let failure = DispatchFailure::UnbackedClaim {
                        state: verdict.state,
                    };
                    let detail = failure.to_string();
                    self.push_failure(module, symbol, &detail);
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Failed,
                        value: None,
                        detail,
                        failure: Some(failure),
                    }
                }
                ImplementationState::Unsupported => {
                    self.stats.unsupported += 1;
                    let detail = if verdict.evidence.is_empty() {
                        "unsupported".to_string()
                    } else {
                        format!("unsupported: {}", verdict.evidence)
                    };
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Unsupported,
                        value: None,
                        detail,
                        failure: None,
                    }
                }
                ImplementationState::Unknown => {
                    self.stats.unknown += 1;
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Unknown,
                        value: None,
                        detail: if verdict.evidence.is_empty() {
                            "no registry entry; preserved for diagnosis".into()
                        } else {
                            format!("unknown: {}", verdict.evidence)
                        },
                        failure: None,
                    }
                }
            }
        };
        self.push_dispatch_record(&result, args.len());
        result
    }

    /// Append a failure message without allowing unbounded growth.
    fn push_failure(&mut self, module: &str, symbol: &str, detail: &str) {
        if self.failures.len() == MAX_HLE_FAILURES {
            self.failures.remove(0);
            self.dropped_failures += 1;
        }
        self.failures.push(format!("{module}!{symbol}: {detail}"));
    }

    /// Append a bounded dispatch record for diagnostics.
    fn push_dispatch_record(&mut self, result: &DispatchResult, args_count: usize) {
        let record = DispatchRecord {
            module: result.module.clone(),
            symbol: result.symbol.clone(),
            status: result.status,
            args_count,
            has_value: result.value.is_some(),
            detail: result.detail.clone(),
        };
        if self.dispatch_trace.len() == MAX_HLE_DISPATCH_TRACE {
            self.dispatch_trace.remove(0);
        }
        self.dispatch_trace.push(record);
    }

    /// Current dispatch counters.
    pub fn stats(&self) -> DispatchStats {
        self.stats
    }

    /// Recorded runtime failures.
    pub fn failures(&self) -> &[String] {
        &self.failures
    }

    /// Number of failure messages dropped by the bounded log.
    pub fn dropped_failures(&self) -> usize {
        self.dropped_failures
    }

    /// Bounded record of recent dispatches (newest last).
    pub fn dispatch_trace(&self) -> &[DispatchRecord] {
        &self.dispatch_trace
    }

    /// How often a module!symbol was dispatched.
    pub fn call_count(&self, module: &str, symbol: &str) -> usize {
        self.dispatch_trace
            .iter()
            .filter(|r| {
                r.module.eq_ignore_ascii_case(module) && r.symbol.eq_ignore_ascii_case(symbol)
            })
            .count()
    }

    /// Clear dispatch diagnostics between guest sessions.
    pub fn reset_diagnostics(&mut self) {
        self.stats = DispatchStats::default();
        self.failures.clear();
        self.dropped_failures = 0;
        self.dispatch_trace.clear();
    }

    /// Human-readable diagnostics summary.
    pub fn render_diagnostics(&self) -> String {
        let s = self.stats;
        let mut out = format!(
            "HLE dispatch: {} completed, {} unsupported, {} unknown, {} failed\n",
            s.completed, s.unsupported, s.unknown, s.failed
        );
        if self.failures.is_empty() {
            out.push_str("runtime failures: none\n");
        } else {
            out.push_str("runtime failures:\n");
            for failure in &self.failures {
                out.push_str(&format!("  - {failure}\n"));
            }
        }
        out
    }
}

/// Evidence string for the foundation shims.
const SHIM_EVIDENCE: &str = "src/core/hle/xmem.rs (guest heap; XMem attributes not decoded)";

/// Map a heap error onto a guest-facing fault.
///
/// M35 keeps the real guest address for invalid-pointer failures instead
/// of the former hard-coded `0`, so a bad `XMemFree` is diagnosable.
fn fault_from_heap(error: HeapError, guest_addr: u64) -> HleFault {
    match error {
        HeapError::OutOfMemory => HleFault::OutOfMemory,
        HeapError::InvalidAddress | HeapError::NotAllocated | HeapError::DoubleFree => {
            HleFault::InvalidAddress(guest_addr)
        }
        other => HleFault::Memory(other),
    }
}

/// `XMemAlloc(size, attributes)` — allocates from the guest heap and
/// returns a **guest address**. Attributes are recorded, not decoded.
fn xmem_alloc(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let (size, attributes) = match args {
        [HleValue::Unsigned(size), HleValue::Unsigned(attributes)] => (*size, *attributes),
        _ => return Err(HleFault::InvalidArgument("XMemAlloc(size, attributes)")),
    };
    let addr = ctx
        .xmem
        .alloc(size, DEFAULT_ALIGNMENT, attributes)
        .map_err(|e| fault_from_heap(e, 0))?;
    Ok(HleValue::Unsigned(addr))
}

/// `XMemFree(pointer, attributes)` — frees a live allocation. Attributes
/// are recorded by the allocation and not decoded.
fn xmem_free(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let addr = match args {
        [HleValue::Unsigned(addr), HleValue::Unsigned(_attributes)] => *addr,
        _ => return Err(HleFault::InvalidArgument("XMemFree(pointer, attributes)")),
    };
    ctx.xmem.free(addr).map_err(|e| fault_from_heap(e, addr))?;
    Ok(HleValue::Null)
}

// --- xHoly VM services (M36) -------------------------------------------
//
// These functions implement xHoly-owned scaffold semantics only. They are
// registered under `xh_vm.*` modules with evidence that explicitly says
// "NOT an Xbox API". No Xbox signature, ordinal or return convention is
// claimed.

fn vm_query(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let addr = match args {
        [HleValue::Unsigned(addr)] => *addr,
        _ => return Err(HleFault::InvalidArgument("VmQuery(pointer)")),
    };
    let info = ctx.xmem.query(addr).map_err(|e| fault_from_heap(e, addr))?;
    // Guest-visible fields, never a host pointer.
    Ok(HleValue::Unsigned(info.size))
}

fn vm_protect(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let (addr, mode) = match args {
        [HleValue::Unsigned(addr), HleValue::Unsigned(mode)] => (*addr, *mode),
        _ => return Err(HleFault::InvalidArgument("VmProtect(pointer, mode)")),
    };
    let protection = match mode {
        0 => super::xmem::Protection::ReadWrite,
        1 => super::xmem::Protection::ReadOnly,
        2 => super::xmem::Protection::NoAccess,
        _ => return Err(HleFault::InvalidArgument("VmProtect mode 0..2")),
    };
    ctx.xmem
        .set_protection(addr, protection)
        .map_err(|e| fault_from_heap(e, addr))?;
    Ok(HleValue::Null)
}

fn vm_stats(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    if !args.is_empty() {
        return Err(HleFault::InvalidArgument("VmStats()"));
    }
    Ok(HleValue::Unsigned(ctx.xmem.stats().active as u64))
}

fn vm_ticks(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    if !args.is_empty() {
        return Err(HleFault::InvalidArgument("VmTicks()"));
    }
    ctx.system.advance();
    Ok(HleValue::Unsigned(ctx.system.ticks()))
}

fn vm_info(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    if !args.is_empty() {
        return Err(HleFault::InvalidArgument("VmInfo()"));
    }
    let info = ctx.system.info();
    // A deterministic, documented packing: major in the high word.
    Ok(HleValue::Unsigned((info.major << 32) | info.minor))
}

fn vm_thread_create(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    if !args.is_empty() {
        return Err(HleFault::InvalidArgument("ThreadCreate()"));
    }
    let id = ctx.kernel.create_thread().ok_or(HleFault::OutOfMemory)?;
    Ok(HleValue::Unsigned(id))
}

fn vm_thread_simple(
    ctx: &mut HleContext,
    args: &[HleValue],
    name: &'static str,
) -> Result<HleValue, HleFault> {
    let id = match args {
        [HleValue::Unsigned(id)] => *id,
        _ => return Err(HleFault::InvalidArgument(name)),
    };
    let ok = match name {
        "ThreadYield()" => ctx.kernel.yield_thread(id),
        "ThreadSuspend()" => ctx.kernel.suspend(id),
        "ThreadResume()" => ctx.kernel.resume(id),
        "ThreadExit()" => ctx.kernel.exit_thread(id),
        _ => return Err(HleFault::Internal("unknown VM thread service".into())),
    };
    if !ok {
        return Err(HleFault::InvalidAddress(id));
    }
    Ok(HleValue::Null)
}

fn vm_thread_yield(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    vm_thread_simple(ctx, args, "ThreadYield()")
}
fn vm_thread_suspend(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    vm_thread_simple(ctx, args, "ThreadSuspend()")
}
fn vm_thread_resume(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    vm_thread_simple(ctx, args, "ThreadResume()")
}
fn vm_thread_exit(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    vm_thread_simple(ctx, args, "ThreadExit()")
}

fn vm_thread_sleep(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let (id, ticks) = match args {
        [HleValue::Unsigned(id), HleValue::Unsigned(ticks)] => (*id, *ticks),
        _ => return Err(HleFault::InvalidArgument("ThreadSleep(thread, ticks)")),
    };
    if !ctx.kernel.sleep(id, ticks) {
        return Err(HleFault::InvalidAddress(id));
    }
    Ok(HleValue::Null)
}

fn vm_event_create(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    if !args.is_empty() {
        return Err(HleFault::InvalidArgument("EventCreate()"));
    }
    let id = ctx.kernel.create_event().ok_or(HleFault::OutOfMemory)?;
    Ok(HleValue::Unsigned(id))
}

fn vm_event_set(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let id = match args {
        [HleValue::Unsigned(id)] => *id,
        _ => return Err(HleFault::InvalidArgument("EventSet(event)")),
    };
    if !ctx.kernel.set_event(id) {
        return Err(HleFault::InvalidAddress(id));
    }
    Ok(HleValue::Null)
}

fn vm_event_wait(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let (id, timeout) = match args {
        [HleValue::Unsigned(id), HleValue::Unsigned(timeout)] => (*id, *timeout),
        _ => return Err(HleFault::InvalidArgument("EventWait(event, ticks)")),
    };
    let result = ctx.kernel.wait_event(id, timeout);
    Ok(HleValue::Unsigned(match result {
        super::kernel::WaitResult::Ready => 0,
        super::kernel::WaitResult::TimedOut => 1,
        super::kernel::WaitResult::InvalidHandle => 2,
        super::kernel::WaitResult::Deadlock => 3,
    }))
}

fn vm_mutex_create(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    if !args.is_empty() {
        return Err(HleFault::InvalidArgument("MutexCreate()"));
    }
    let id = ctx.kernel.create_mutex().ok_or(HleFault::OutOfMemory)?;
    Ok(HleValue::Unsigned(id))
}

fn vm_mutex_lock(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let (id, thread, timeout) = match args {
        [HleValue::Unsigned(id), HleValue::Unsigned(thread), HleValue::Unsigned(timeout)] => {
            (*id, *thread, *timeout)
        }
        _ => return Err(HleFault::InvalidArgument("MutexLock(mutex, thread, ticks)")),
    };
    let result = ctx.kernel.lock_mutex(id, thread, timeout);
    Ok(HleValue::Unsigned(match result {
        super::kernel::WaitResult::Ready => 0,
        super::kernel::WaitResult::TimedOut => 1,
        super::kernel::WaitResult::InvalidHandle => 2,
        super::kernel::WaitResult::Deadlock => 3,
    }))
}

fn vm_mutex_unlock(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let (id, thread) = match args {
        [HleValue::Unsigned(id), HleValue::Unsigned(thread)] => (*id, *thread),
        _ => return Err(HleFault::InvalidArgument("MutexUnlock(mutex, thread)")),
    };
    let result = ctx.kernel.unlock_mutex(id, thread);
    Ok(HleValue::Unsigned(match result {
        super::kernel::WaitResult::Ready => 0,
        _ => 2,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foundation_registers_only_registry_backed_symbols() {
        let rt = HleRuntime::with_foundation();
        let implemented = rt.implemented_symbols();
        assert_eq!(
            implemented,
            vec![
                ("xmem.dll".to_string(), "xmemalloc".to_string()),
                ("xmem.dll".to_string(), "xmemfree".to_string())
            ]
        );
        for (module, symbol) in &implemented {
            assert!(rt.registry().function_meta(module, symbol).is_some());
        }
    }

    #[test]
    fn registration_requires_registry_backing() {
        let mut rt = HleRuntime::new(HleRegistry::with_known_entries());
        // Unknown symbol: refused.
        assert_eq!(
            rt.register("xmem.dll", "XMemNotAFunction", xmem_alloc, "test", 2),
            Err(RegisterError::NotInRegistry)
        );
        // Registry says Unknown: refused, no placeholder may be installed.
        assert_eq!(
            rt.register("xmem.dll", "XMemVirtualAlloc", xmem_alloc, "test", 2),
            Err(RegisterError::WrongState(ImplementationState::Unknown))
        );
        // Unsupported module entry: refused.
        assert_eq!(
            rt.register(
                "d3d12_xs.dll",
                "D3D12XboxCreateDevice",
                xmem_alloc,
                "test",
                2
            ),
            Err(RegisterError::WrongState(ImplementationState::Unknown))
        );
        assert!(rt.implemented_symbols().is_empty());
    }

    #[test]
    fn arity_beyond_the_argument_registers_is_refused() {
        let mut rt = HleRuntime::new(HleRegistry::with_known_entries());
        let too_many = (MAX_HLE_ARGS + 1) as u8;
        assert_eq!(
            rt.register("xmem.dll", "XMemAlloc", xmem_alloc, "test", too_many),
            Err(RegisterError::ArityOutOfRange(too_many))
        );
        assert!(rt.implemented_symbols().is_empty());
        // Registering with stack-passed arguments (arity > 4) is accepted up to MAX_HLE_ARGS.
        rt.register("xmem.dll", "XMemAlloc", xmem_alloc, "test", 6)
            .expect("arity 6 fits within MAX_HLE_ARGS");
        assert_eq!(rt.implementation_arity("xmem.dll", "XMemAlloc"), Some(6));
        assert_eq!(rt.implementation_arity("xmem.dll", "XMemFree"), None);
        assert!(rt.implementation_arity("nope.dll", "nope").is_none());
    }

    #[test]
    fn foundation_declares_the_two_argument_xmem_signatures() {
        let rt = HleRuntime::with_foundation();
        assert_eq!(rt.implementation_arity("xmem.dll", "XMemAlloc"), Some(2));
        assert_eq!(rt.implementation_arity("xmem.dll", "XMemFree"), Some(2));
    }

    #[test]
    fn dispatch_completes_registered_functions() {
        let mut rt = HleRuntime::with_foundation();
        let result = rt.dispatch(
            "XMEM.DLL",
            "xmemalloc",
            &[HleValue::Unsigned(64), HleValue::Unsigned(0)],
        );
        assert_eq!(result.status, DispatchStatus::Completed);
        let addr = match result.value {
            Some(HleValue::Unsigned(a)) => a,
            other => panic!("expected guest address, got {other:?}"),
        };
        assert_eq!(addr % DEFAULT_ALIGNMENT, 0);

        let freed = rt.dispatch(
            "xmem.dll",
            "XMemFree",
            &[HleValue::Unsigned(addr), HleValue::Unsigned(0)],
        );
        assert_eq!(freed.status, DispatchStatus::Completed);
        assert_eq!(freed.value, Some(HleValue::Null));
        assert_eq!(rt.stats().completed, 2);
    }

    #[test]
    fn unsupported_symbols_return_a_structured_result_without_a_value() {
        let mut rt = HleRuntime::with_foundation();
        let result = rt.dispatch("gameinput.dll", "GameInputCreate", &[]);
        assert_eq!(result.status, DispatchStatus::Unsupported);
        assert_eq!(result.value, None, "unsupported must not fake a return");
        assert!(
            result.detail.starts_with("unsupported:"),
            "{}",
            result.detail
        );
        assert_eq!(rt.stats().unsupported, 1);
    }

    #[test]
    fn unknown_symbols_are_preserved_not_rejected() {
        let mut rt = HleRuntime::with_foundation();
        let result = rt.dispatch("mystery.dll", "DoSomething", &[HleValue::Unsigned(1)]);
        assert_eq!(result.status, DispatchStatus::Unknown);
        assert_eq!(result.value, None);
        assert!(result.detail.contains("preserved"), "{}", result.detail);

        // Symbols without their own entry inside a partially implemented
        // module stay unknown — module credit is not inherited.
        let partial = rt.dispatch("xmem.dll", "XMemTransferMemory", &[]);
        assert_eq!(partial.status, DispatchStatus::Unknown);
        assert_eq!(rt.stats().unknown, 2);
    }

    #[test]
    fn runtime_failures_are_distinct_from_unsupported() {
        let mut rt = HleRuntime::with_foundation();
        // Wrong argument shape → implementation runs and fails.
        let bad_args = rt.dispatch("xmem.dll", "XMemAlloc", &[HleValue::Unsigned(1)]);
        assert_eq!(bad_args.status, DispatchStatus::Failed);
        assert_eq!(bad_args.value, None);
        // Freeing an address that was never allocated → runtime failure.
        let not_alloc = rt.dispatch(
            "xmem.dll",
            "XMemFree",
            &[HleValue::Unsigned(0x1234), HleValue::Unsigned(0)],
        );
        assert_eq!(not_alloc.status, DispatchStatus::Failed);
        assert!(
            not_alloc.detail.contains("runtime failure"),
            "{}",
            not_alloc.detail
        );
        assert_eq!(rt.stats().failed, 2);
        assert_eq!(rt.failures().len(), 2);
        assert!(rt.render_diagnostics().contains("2 failed"));
    }

    #[test]
    fn registry_implementation_claim_without_registration_is_a_failure() {
        let mut registry = HleRegistry::with_known_entries();
        registry.functions.push(super::super::FunctionMeta {
            module: "xmem.dll".into(),
            name: "XMemGhost".into(),
            state: ImplementationState::Implemented,
            evidence: "test-only entry".into(),
        });
        let mut rt = HleRuntime::new(registry);
        let result = rt.dispatch("xmem.dll", "XMemGhost", &[]);
        assert_eq!(result.status, DispatchStatus::Failed);
        assert!(result.detail.contains("no implementation is registered"));
    }

    #[test]
    fn metadata_and_implementation_consistency() {
        let rt = HleRuntime::with_foundation();
        for (module, symbol) in rt.implemented_symbols() {
            let meta = rt
                .registry()
                .function_meta(&module, &symbol)
                .expect("registered symbol must have registry metadata");
            assert!(
                matches!(
                    meta.state,
                    ImplementationState::Implemented | ImplementationState::PartiallyImplemented
                ),
                "{module}!{symbol} registered with state {:?}",
                meta.state
            );
            assert!(!meta.evidence.is_empty());
            assert!(rt.implementation_evidence(&module, &symbol).is_some());
        }
        // Conversely, no registry "Implemented" entry may be missing an
        // implementation (that would be an unbacked claim).
        for function in &rt.registry().functions {
            if function.state == ImplementationState::Implemented {
                assert!(
                    rt.implemented_symbols().iter().any(|(m, s)| {
                        m.eq_ignore_ascii_case(&function.module)
                            && s.eq_ignore_ascii_case(&function.name)
                    }),
                    "{}!{} claims Implemented without an implementation",
                    function.module,
                    function.name
                );
            }
        }
    }

    #[test]
    fn case_insensitive_module_and_symbol_lookup() {
        let mut rt = HleRuntime::with_foundation();
        assert_eq!(
            rt.registry().module_state("XMEM.DLL"),
            ImplementationState::PartiallyImplemented
        );
        assert_eq!(
            rt.registry()
                .function_meta("Xmem.dll", "xmemalloc")
                .map(|m| m.state),
            Some(ImplementationState::PartiallyImplemented)
        );
        let result = rt.dispatch(
            "XmEm.DlL",
            "XmEmAlLoC",
            &[HleValue::Unsigned(8), HleValue::Unsigned(0)],
        );
        assert_eq!(result.status, DispatchStatus::Completed);
    }
}
