//! Deterministic simulator for the sync tests (SYNC §19); compiled for tests only.
//! Stub created by W1.1; implemented by W2.6 (BUILD-PLAN.md).

/// Test command hook: `__IHN_TEST_API__.cmd('sync.<module>.<name>', args)` (JSON text in, JSON text out).
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }
