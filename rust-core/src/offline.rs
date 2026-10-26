//! Ported logic for the offline area lives here. Pure functions + a `call` dispatcher entry.
mod library;
mod worker_kernels;

pub fn call(method: &str, input: &[u8]) -> Option<Result<Vec<u8>, String>> {
    library::call(method, input).or_else(|| worker_kernels::call(method, input))
}
