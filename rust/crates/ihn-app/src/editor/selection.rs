//! Editor: Lasso, selection box, transforms, cross-page move.
//! Stub created by W1.1; implemented by the unit that owns this file in BUILD-PLAN.md. The four items below are the module contract
//! (`app.rs`): keep their signatures.

use crate::{App, Event, Host};

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State;

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(_app: &mut App, _host: &mut dyn Host, _event: &Event) -> u32 { 0 }

/// `__IHN_TEST_API__.cmd('<module>.<name>', args)`: JSON text in, JSON text out; `None` = unknown command.
pub fn test_cmd(_app: &mut App, _host: &mut dyn Host, _name: &str, _args: &str) -> Option<String> { None }

/// v5 `clearSelection()` — interface placeholder for W4.3 (`editor::images`); W4.2 replaces this file.
pub fn clear(_app: &mut App, _host: &mut dyn Host) {}

/// Selects `items` of `page` and shows the box — interface placeholder for W4.3; W4.2 replaces this file.
pub fn select(_app: &mut App, _host: &mut dyn Host, _page: ihn_core::id::Id, _items: &[ihn_core::id::Id]) {}
