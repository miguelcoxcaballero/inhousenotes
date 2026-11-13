//! The document statistics line of the editor profile menu (`#doc-stats`). Owned by W3.1.
// v5: app-v5.js:updateDocStats / formatFileSize

use super::session;
use crate::testapi::json_str;
use crate::ui_ids::Ui;
use crate::{shell, App, Event, Host};
use ihn_core::doc::Doc;
use ihn_core::jsfmt;
use ihn_core::op::Body;

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State;

// v5: app-v5.js:formatFileSize
pub fn file_size(bytes: f64) -> String {
    if bytes < 1024.0 {
        format!("{} B", jsfmt::num(bytes))
    } else if bytes < 1024.0 * 1024.0 {
        format!("{} KB", jsfmt::fixed(bytes / 1024.0, 1))
    } else {
        format!("{} MB", jsfmt::fixed(bytes / (1024.0 * 1024.0), 1))
    }
}

/// `N page(s) · M stroke(s)[ · size]`: pages in the document's order, live strokes on them, and the size of the
/// Drive file last downloaded or uploaded.
pub fn line(doc: &Doc, export_size: f64) -> String {
    let pages = doc.order().len();
    let strokes: usize = doc.order().iter().map(|&p| doc.draw_list(p).iter().filter(|r| matches!(doc.items.get(&r.id).map(|i| &i.body.v), Some(Body::Stroke { .. }))).count()).sum();
    let plural = |n: usize| if n != 1 { "s" } else { "" };
    let mut out = format!("{pages} page{} · {strokes} stroke{}", plural(pages), plural(strokes));
    if export_size > 0.0 {
        out.push_str(" · ");
        out.push_str(&file_size(export_size));
    }
    out
}

fn current(app: &App) -> String { session::get(app).map_or_else(|| line(&Doc::default(), 0.0), |s| line(&s.doc, s.export_size)) }

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    // v5: toggleDriveProfileMenu — the line is computed when the editor menu opens.
    if let Event::Msg(m) = event {
        if m.chan == shell::sig::MENU && m.data == b"editor" {
            let text = current(app);
            host.out().text(Ui::DOC_STATS, &text);
        }
    }
    0
}

/// `__IHN_TEST_API__.cmd('doc.stats.line')` → the line as it would be written now.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> { (name == "line").then(|| json_str(&current(app))) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_counts_pages_and_live_strokes_and_formats_the_size_like_5_18_0() {
        assert_eq!(line(&Doc::default(), 0.0), "0 pages · 0 strokes");
        let doc = session::fixture();
        assert_eq!(line(&doc, 0.0), "3 pages · 2 strokes");
        assert_eq!(line(&doc, 1023.0), "3 pages · 2 strokes · 1023 B");
        assert_eq!(line(&doc, 1536.0), "3 pages · 2 strokes · 1.5 KB");
        assert_eq!(line(&doc, 5.0 * 1024.0 * 1024.0 + 1.0), "3 pages · 2 strokes · 5.0 MB");
        assert_eq!(file_size(1024.0 * 1024.0 - 1.0), "1024.0 KB");
    }
}
