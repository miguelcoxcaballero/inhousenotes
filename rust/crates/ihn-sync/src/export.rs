//! Export decisions, upload loop, known-export table, head classification (SYNC §14).
//! Stub created by W1.1; implemented by W2.7 (BUILD-PLAN.md).

/// Test command hook: `__IHN_TEST_API__.cmd('sync.<module>.<name>', args)` (JSON text in, JSON text out).
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }

/// This module's protocol part for a session (`crate::Part`); `None` until the unit builds it (see `lib.rs`).
pub fn part() -> Option<Box<dyn crate::Part>> { None }
