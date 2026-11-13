//! `ihn-sync` — pure sync protocol state machines (SYNC.md §18.1). No I/O; depends on `ihn-core` only.
//! Stub created by W1.1; `lib.rs`, `mode.rs`, `wire.rs` and `sim.rs` are implemented by W2.6, which keeps the module
//! declarations and `test_cmd` routing below.

pub mod entropy;
pub mod export;
pub mod live;
pub mod mode;
pub mod peer;
pub mod relay;
#[cfg(test)]
pub mod sim;
pub mod wire;

/// Routes `sync.<module>.<name>` test commands to `<module>::test_cmd` (called by `ihn-app::testapi`).
pub fn test_cmd(module: &str, name: &str, args: &str) -> Option<String> {
    let f: fn(&str, &str) -> Option<String> = match module {
        "entropy" => entropy::test_cmd,
        "export" => export::test_cmd,
        "live" => live::test_cmd,
        "mode" => mode::test_cmd,
        "peer" => peer::test_cmd,
        "relay" => relay::test_cmd,
        "wire" => wire::test_cmd,
        _ => return None,
    };
    f(name, args)
}
