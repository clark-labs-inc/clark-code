use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;

use sha2::{Digest, Sha256};
use wasmi::core::TrapCode;
use wasmi::{
    Config, Engine, Error as WasmiError, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
};

use crate::{
    CapsuleHostError, CapsuleHostLimits, CapsuleHostResult, CapsuleInvocation,
    CapsuleIsolationReceipt, CAPSULE_HOST_RUNTIME,
};

#[derive(Clone)]
pub struct CapsuleHost {
    inner: Arc<HostInner>,
}

struct HostInner {
    engine: Engine,
    approved_module_digests: BTreeSet<String>,
    limits: CapsuleHostLimits,
    active: Arc<AtomicUsize>,
}

struct HostState {
    limits: StoreLimits,
}

struct InvocationSlot {
    active: Arc<AtomicUsize>,
}

impl Drop for InvocationSlot {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl CapsuleHost {
    pub fn new(
        approved_module_digests: impl IntoIterator<Item = String>,
        limits: CapsuleHostLimits,
    ) -> CapsuleHostResult<Self> {
        let limits = limits.validate()?;
        let approved_module_digests = approved_module_digests.into_iter().collect::<BTreeSet<_>>();
        if approved_module_digests.is_empty() {
            return Err(CapsuleHostError::EmptyApprovalPolicy);
        }
        if approved_module_digests
            .iter()
            .any(|digest| !is_sha256(digest))
        {
            return Err(CapsuleHostError::InvalidApprovedDigest);
        }
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        Ok(Self {
            inner: Arc::new(HostInner {
                engine,
                approved_module_digests,
                limits,
                active: Arc::new(AtomicUsize::new(0)),
            }),
        })
    }

    pub fn limits(&self) -> CapsuleHostLimits {
        self.inner.limits
    }

    /// Runs one approved module invocation.
    ///
    /// A deadline returns [`CapsuleHostError::DeadlineExceeded`] immediately.
    /// The timed-out worker remains counted against the concurrency limit until
    /// its finite fuel budget ends, because wasmi 0.40 has deterministic fuel
    /// interruption but no hard thread preemption.
    pub fn invoke(
        &self,
        module_bytes: &[u8],
        input: &[u8],
    ) -> CapsuleHostResult<CapsuleInvocation> {
        if module_bytes.len() > self.inner.limits.max_module_bytes {
            return Err(CapsuleHostError::ModuleTooLarge);
        }
        if input.len() > self.inner.limits.max_input_bytes {
            return Err(CapsuleHostError::InputTooLarge);
        }
        let module_digest = module_sha256(module_bytes);
        if !self.inner.approved_module_digests.contains(&module_digest) {
            return Err(CapsuleHostError::ModuleNotApproved);
        }
        let slot = self.acquire_slot()?;
        let engine = self.inner.engine.clone();
        let limits = self.inner.limits;
        let module_bytes = module_bytes.to_vec();
        let input = input.to_vec();
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("scout-wasm-capsule".into())
            .spawn(move || {
                let _slot = slot;
                let result = invoke_inner(&engine, &module_bytes, &input, module_digest, limits);
                let _ = sender.send(result);
            })
            .map_err(|_| CapsuleHostError::WorkerFailed)?;

        match receiver.recv_timeout(limits.deadline()) {
            Ok(result) => {
                worker.join().map_err(|_| CapsuleHostError::WorkerFailed)?;
                result
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(CapsuleHostError::DeadlineExceeded),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                Err(CapsuleHostError::WorkerFailed)
            }
        }
    }

    fn acquire_slot(&self) -> CapsuleHostResult<InvocationSlot> {
        self.inner
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < self.inner.limits.max_concurrent_instances).then_some(active + 1)
            })
            .map_err(|_| CapsuleHostError::ConcurrencyLimit)?;
        Ok(InvocationSlot {
            active: Arc::clone(&self.inner.active),
        })
    }
}

pub fn module_sha256(module_bytes: &[u8]) -> String {
    hex_sha256(module_bytes)
}

fn invoke_inner(
    engine: &Engine,
    module_bytes: &[u8],
    input: &[u8],
    module_digest: String,
    limits: CapsuleHostLimits,
) -> CapsuleHostResult<CapsuleInvocation> {
    let started = Instant::now();
    let module = Module::new(engine, module_bytes).map_err(|_| CapsuleHostError::InvalidModule)?;
    if module.imports().next().is_some() {
        return Err(CapsuleHostError::ImportedCapability);
    }
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.max_linear_memory_bytes)
        .table_elements(limits.max_table_elements)
        .instances(1)
        .memories(1)
        .tables(1)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(
        engine,
        HostState {
            limits: store_limits,
        },
    );
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(limits.max_fuel)
        .map_err(|_| CapsuleHostError::InvalidLimit("max_fuel"))?;
    let linker = Linker::<HostState>::new(engine);
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(map_wasmi_error)?
        .start(&mut store)
        .map_err(map_wasmi_error)?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or(CapsuleHostError::InvalidAbi)?;
    let allocate = instance
        .get_typed_func::<i32, i32>(&store, "scout_alloc")
        .map_err(|_| CapsuleHostError::InvalidAbi)?;
    let run = instance
        .get_typed_func::<(i32, i32), i64>(&store, "scout_run")
        .map_err(|_| CapsuleHostError::InvalidAbi)?;

    let input_len = i32::try_from(input.len()).map_err(|_| CapsuleHostError::InputTooLarge)?;
    let input_pointer = allocate
        .call(&mut store, input_len)
        .map_err(map_wasmi_error)? as u32 as usize;
    memory
        .write(&mut store, input_pointer, input)
        .map_err(|_| CapsuleHostError::MemoryBounds)?;
    let packed = run
        .call(&mut store, (input_pointer as u32 as i32, input_len))
        .map_err(map_wasmi_error)? as u64;
    let output_pointer = (packed & u64::from(u32::MAX)) as usize;
    let output_len = (packed >> 32) as usize;
    if output_len > limits.max_output_bytes {
        return Err(CapsuleHostError::OutputTooLarge);
    }
    let output_end = output_pointer
        .checked_add(output_len)
        .ok_or(CapsuleHostError::MemoryBounds)?;
    if output_end > memory.data_size(&store) {
        return Err(CapsuleHostError::MemoryBounds);
    }
    let mut output = vec![0u8; output_len];
    memory
        .read(&store, output_pointer, &mut output)
        .map_err(|_| CapsuleHostError::MemoryBounds)?;
    let remaining_fuel = store
        .get_fuel()
        .map_err(|_| CapsuleHostError::InvalidLimit("max_fuel"))?;
    let elapsed_micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let receipt = CapsuleIsolationReceipt::new(
        CAPSULE_HOST_RUNTIME,
        module_digest,
        limits,
        hex_sha256(input),
        hex_sha256(&output),
        limits.max_fuel.saturating_sub(remaining_fuel),
        elapsed_micros,
    );
    Ok(CapsuleInvocation { output, receipt })
}

fn map_wasmi_error(error: WasmiError) -> CapsuleHostError {
    match error.as_trap_code() {
        Some(TrapCode::OutOfFuel) => CapsuleHostError::FuelExhausted,
        Some(_) => CapsuleHostError::GuestTrap,
        None => CapsuleHostError::InvalidAbi,
    }
}

fn is_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
