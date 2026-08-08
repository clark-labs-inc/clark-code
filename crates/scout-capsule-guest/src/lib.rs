//! No-WASI guest adapter for `scout-capsule-core`.
//!
//! The two raw-pointer operations are the complete capsule ABI boundary. The
//! host writes only into memory returned by `scout_alloc`, caps all lengths,
//! invokes the module once, and then destroys the instance. Guest allocations
//! are intentionally leaked for that one short-lived invocation.

#![cfg(target_arch = "wasm32")]

use std::mem;
use std::slice;

use scout_capsule_core::{normalize_json, CapsuleLimits};

#[no_mangle]
pub extern "C" fn scout_alloc(length: i32) -> i32 {
    let Ok(length) = usize::try_from(length) else {
        return 0;
    };
    let mut allocation = vec![0u8; length];
    let pointer = allocation.as_mut_ptr();
    mem::forget(allocation);
    pointer as usize as u32 as i32
}

/// Runs the pure normalization kernel.
///
/// # Safety
///
/// `pointer..pointer + length` must name the allocation returned by the
/// immediately preceding `scout_alloc` call in this fresh module instance.
#[no_mangle]
pub unsafe extern "C" fn scout_run(pointer: i32, length: i32) -> i64 {
    let Ok(length) = usize::try_from(length) else {
        return 0;
    };
    let input = unsafe { slice::from_raw_parts(pointer as u32 as usize as *const u8, length) };
    let Ok(mut output) = normalize_json(input, CapsuleLimits::default()) else {
        return 0;
    };
    let output_pointer = output.as_mut_ptr() as usize as u32;
    let Ok(output_length) = u32::try_from(output.len()) else {
        return 0;
    };
    mem::forget(output);
    ((u64::from(output_length) << 32) | u64::from(output_pointer)) as i64
}
