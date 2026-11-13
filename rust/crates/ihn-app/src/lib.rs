//! `ihn-app` — the Inhouse Notes 6 application: UI state machines, input, scheduler, storage/Drive/auth logic,
//! all driven through `trait Host` (ARCHITECTURE §2.3). Rust owns state and decisions; `host/*.js` only applies
//! the out-buffer and forwards events.
//!
//! Module declarations, `app.rs`, `abi.rs`, `dom.rs`, `host.rs`, `testapi.rs` and `ui_ids.rs` are owned by W1.1.
//! Every other file is owned by the unit named in BUILD-PLAN.md and follows the module contract of `app.rs`.

pub mod abi;
pub mod app;
pub mod auth;
pub mod collab;
pub mod doc;
pub mod dom;
pub mod drive;
pub mod editor;
pub mod export;
pub mod home;
pub mod host;
pub mod migrate;
pub mod panels;
pub mod shell;
pub mod store;
pub mod testapi;
pub mod ui_ids;
pub mod updater;

pub use abi::{tag, tag_local, tag_mod, Config, DomEvent, Event, Io, Life, Msg, Node, OutBuf, PointerBatch, Sample};
pub use app::{App, Mod, Sched};
pub use dom::{Dialog, Prop, Scroll};
pub use host::{Host, IntlOp, LsOp, Metric, Rect};
pub use ui_ids::Ui;
