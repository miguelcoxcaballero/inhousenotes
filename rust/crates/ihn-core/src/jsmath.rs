//! V8-exact `Math.*` ports (ARCHITECTURE §6.5).
//! Stub created by W1.1; implemented by W1.4 (BUILD-PLAN.md).

/// Test command hook: `__IHN_TEST_API__.cmd('core.<module>.<name>', args)` (JSON text in, JSON text out).
pub fn test_cmd(_name: &str, _args: &str) -> Option<String> { None }
