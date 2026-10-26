//! Ported logic for the offline area lives here. Pure functions + a `call` dispatcher entry.
mod worker_kernels;

pub fn call(method: &str, input: &[u8]) -> Option<Result<Vec<u8>, String>> {
    worker_kernels::call(method, input)
}
