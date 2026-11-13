//! Timeline versions: capture, coalesce, prune, restore planning, diff (ARCHITECTURE §4.6).
//! Stub created by W1.1; implemented by W2.9 (BUILD-PLAN.md).

/// Test command hook: `__IHN_TEST_API__.cmd('core.<module>.<name>', args)` (JSON text in, JSON text out).
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }
