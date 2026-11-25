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
    /// Human-readable reason / evidence.
    pub detail: String,
}

/// Signature of an HLE implementation.
pub type HleFunction = fn(&mut HleContext, &[HleValue]) -> Result<HleValue, HleFault>;

/// State handed to implementations.
#[derive(Debug)]
pub struct HleContext {
    /// Guest heap backing the `xmem` shims.
    pub xmem: GuestHeap,
}

impl Default for HleContext {
    fn default() -> Self {
        Self {
            xmem: GuestHeap::with_defaults(),
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

    /// Dispatch one call.
    pub fn dispatch(&mut self, module: &str, symbol: &str, args: &[HleValue]) -> DispatchResult {
        let key = (module.to_lowercase(), symbol.to_lowercase());
        if let Some(registered) = self.impls.get(&key) {
            let func = registered.func;
            let evidence = registered.evidence;
            return match func(&mut self.context, args) {
                Ok(value) => {
                    self.stats.completed += 1;
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Completed,
                        value: Some(value),
                        detail: format!("implemented: {evidence}"),
                    }
                }
                Err(fault) => {
                    self.stats.failed += 1;
                    let detail = format!("runtime failure: {fault}");
                    self.failures.push(format!("{module}!{symbol}: {detail}"));
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Failed,
                        value: None,
                        detail,
                    }
                }
            };
        }

        if let Some(meta) = self.registry.function_meta(module, symbol) {
            return match meta.state {
                ImplementationState::Implemented | ImplementationState::PartiallyImplemented => {
                    self.stats.failed += 1;
                    let detail = format!(
                        "registry marks {} but no implementation is registered",
                        meta.state.label()
                    );
                    self.failures.push(format!("{module}!{symbol}: {detail}"));
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Failed,
                        value: None,
                        detail,
                    }
                }
                ImplementationState::Unsupported => {
                    self.stats.unsupported += 1;
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Unsupported,
                        value: None,
                        detail: format!("unsupported: {}", meta.evidence),
                    }
                }
                ImplementationState::Unknown => {
                    self.stats.unknown += 1;
                    DispatchResult {
                        module: module.into(),
                        symbol: symbol.into(),
                        status: DispatchStatus::Unknown,
                        value: None,
                        detail: format!("unknown: {}", meta.evidence),
                    }
                }
            };
        }

        // No function entry: fall back to module metadata. Only an
        // explicitly unsupported module may classify its functions as
        // unsupported — a partially implemented module must not inherit
        // credit for functions it does not provide.
        if self.registry.module_state(module) == ImplementationState::Unsupported {
            let detail = self
                .registry
                .module_meta(module)
                .map(|m| format!("unsupported module: {}", m.evidence))
                .unwrap_or_else(|| "unsupported module".into());
            self.stats.unsupported += 1;
            DispatchResult {
                module: module.into(),
                symbol: symbol.into(),
                status: DispatchStatus::Unsupported,
                value: None,
                detail,
            }
        } else {
            self.stats.unknown += 1;
            DispatchResult {
                module: module.into(),
                symbol: symbol.into(),
                status: DispatchStatus::Unknown,
                value: None,
                detail: "no registry entry; preserved for diagnosis".into(),
            }
        }
    }

    /// Current dispatch counters.
    pub fn stats(&self) -> DispatchStats {
        self.stats
    }

    /// Recorded runtime failures.
    pub fn failures(&self) -> &[String] {
        &self.failures
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

fn fault_from_heap(error: HeapError) -> HleFault {
    match error {
        HeapError::OutOfMemory => HleFault::OutOfMemory,
        HeapError::InvalidAddress | HeapError::NotAllocated | HeapError::DoubleFree => {
            HleFault::InvalidAddress(0)
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
        .map_err(fault_from_heap)?;
    Ok(HleValue::Unsigned(addr))
}

/// `XMemFree(pointer, attributes)` — frees a live allocation. Attributes
/// are recorded by the allocation and not decoded.
fn xmem_free(ctx: &mut HleContext, args: &[HleValue]) -> Result<HleValue, HleFault> {
    let addr = match args {
        [HleValue::Unsigned(addr), HleValue::Unsigned(_attributes)] => *addr,
        _ => return Err(HleFault::InvalidArgument("XMemFree(pointer, attributes)")),
    };
    ctx.xmem.free(addr).map_err(fault_from_heap)?;
    Ok(HleValue::Null)
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
