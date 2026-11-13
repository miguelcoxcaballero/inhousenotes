//! `ihn-core` — the pure library of Inhouse Notes 6: document model, codecs, render geometry and 5.x readers
//! (ARCHITECTURE §2.2). No host access, no I/O, no external crates.
//!
//! Module declarations are owned by W1.1; each module file is owned by the unit named in its header.

pub mod catalog;
pub mod codec;
pub mod doc;
pub mod geom;
pub mod history;
pub mod id;
pub mod jsfmt;
pub mod jsmath;
pub mod motion;
pub mod op;
pub mod palette;
pub mod pos;
pub mod render;
pub mod rows;
pub mod snapshot;
pub mod stamp;
#[doc(hidden)]
pub mod testutil;
pub mod v5;

/// Routes `core.<module>.<name>` test commands to `<module>::test_cmd` (called by `ihn-app::testapi`).
pub fn test_cmd(module: &str, name: &str, args: &str) -> Option<String> {
    let f: fn(&str, &str) -> Option<String> = match module {
        "catalog" => catalog::test_cmd,
        "doc" => doc::test_cmd,
        "geom" => geom::test_cmd,
        "history" => history::test_cmd,
        "id" => id::test_cmd,
        "jsfmt" => jsfmt::test_cmd,
        "jsmath" => jsmath::test_cmd,
        "motion" => motion::test_cmd,
        "op" => op::test_cmd,
        "palette" => palette::test_cmd,
        "pos" => pos::test_cmd,
        "render" => render::test_cmd,
        "rows" => rows::test_cmd,
        "snapshot" => snapshot::test_cmd,
        "stamp" => stamp::test_cmd,
        "v5" => v5::test_cmd,
        _ => return None,
    };
    f(name, args)
}
