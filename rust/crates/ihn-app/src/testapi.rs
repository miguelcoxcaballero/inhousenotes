//! Test-command dispatcher behind `__IHN_TEST_API__.cmd(name, args)` (BUILD-PLAN "Rules for every unit") and the
//! `app` pseudo-module: `app.state` and the ABI self-test that e2e `00-boot` runs against the real host.
//!
//! `name` is `<module>.<command>`: `<module>` is a name of `app::MODULES` (`store`, `drive.client`, …), or
//! `core.<module>` / `sync.<module>` for the pure crates. `args` and the result are JSON text; the result reaches
//! the page through the `test_result` effect.

use crate::abi::{ety, ev, fam, fx, observe, ret, tag, Event, Node};
use crate::app::{Mod, MODULES};
use crate::dom::Prop;
use crate::host::{Host, IntlOp, LsOp, Metric};
use crate::App;

/// State of the `app` pseudo-module: what the self-test observed.
#[derive(Default)]
pub struct State {
    root: Node,
    button: Node,
    anim: Node,
    clicks: u32,
    path_ok: bool,
    timer: bool,
    interval: u32,
    frame: bool,
    idle: bool,
    finish: bool,
    work: bool,
    dragover: bool,
    dragleave: bool,
    resized: bool,
    mutated: bool,
    pen: u32,
    sync: String,
}

const CLICK: u32 = tag(Mod::App, 1);
const TIMER: u32 = tag(Mod::App, 2);
const FINISH: u32 = tag(Mod::App, 3);
const IDLE: u32 = tag(Mod::App, 4);
const WORK: u32 = tag(Mod::App, 5);
const INTERVAL: u32 = tag(Mod::App, 6);
const DRAGOVER: u32 = tag(Mod::App, 10);
const DRAGLEAVE: u32 = tag(Mod::App, 11);
const RESIZED: u32 = tag(Mod::App, 12);
const MUTATED: u32 = tag(Mod::App, 13);
const PEN_DOWN: u32 = tag(Mod::App, 15);
const PEN_MOVE: u32 = tag(Mod::App, 16);
const PEN_WORK: u32 = tag(Mod::App, 17);

pub fn init(_app: &mut App, _host: &mut dyn Host) {}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    let st = &mut app.st.app;
    match *event {
        Event::Dom(e) if e.listener == CLICK => {
            st.clicks += 1;
            st.path_ok = e.node == st.button && e.path.first() == st.button && e.has(ev::EXACT) && e.ty == ety::CLICK
                && e.path.contains(st.root) && e.path.contains(Node::BODY) && !e.path.contains(st.anim) && e.t > 0.0;
        }
        Event::Dom(e) if e.listener == FINISH => st.finish = e.ty == ety::FINISH,
        // v5: app-v5.js:handleImageDragOver — accept files with the copy cursor.
        Event::Dom(e) if e.listener == DRAGOVER => {
            st.dragover = e.ty == ety::DRAGOVER && e.has(ev::DT_FILES);
            return ret::PREVENT_DEFAULT | ret::DROP_COPY;
        }
        Event::Dom(e) if e.listener == DRAGLEAVE => {
            st.dragleave = e.ty == ety::DRAGLEAVE && e.related.first() == st.root && !e.related.contains(st.button);
        }
        Event::Dom(e) if e.listener == RESIZED => st.resized = e.ty == ety::RESIZE_OBSERVED && e.node == Node::BODY,
        Event::Dom(e) if e.listener == MUTATED => {
            st.mutated = e.ty == ety::MUTATION_OBSERVED && e.node == st.root && e.path.first() == st.root;
        }
        Event::Pointer(p) if p.listener == PEN_DOWN => {
            st.pen += 1;
            app.sched.set_busy(Mod::App, true);
        }
        Event::Pointer(p) if p.listener == PEN_MOVE => st.pen += p.samples().count() as u32,
        Event::Timer(TIMER) => st.timer = true,
        Event::Timer(INTERVAL) => {
            st.interval += 1;
            if st.interval == 2 {
                app.sched.clear(host.out(), INTERVAL);
            }
        }
        Event::Frame(_) => st.frame = true,
        Event::Idle { id: IDLE, .. } => st.idle = true,
        Event::Work { id: WORK, stale: false } => st.work = true,
        _ => {}
    }
    0
}

pub fn test_cmd(app: &mut App, host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    Some(match name {
        "state" => state(app),
        "selftest" => selftest(app, host),
        // W6.3p: allocator accounting (`--features memstat`; `{}` otherwise).
        "mem" => crate::memstat::report(_args.contains("reset")),
        "selftest_result" => selftest_result(app, host),
        // A pen stroke on the self-test button while background work is queued (e2e: no host timer per sample).
        "pen_probe" => {
            let (out, button) = (host.out(), app.st.app.button);
            out.listen(button, "pointerdown", PEN_DOWN, crate::abi::listen::PASSIVE);
            out.listen(button, "pointermove", PEN_MOVE, crate::abi::listen::PASSIVE);
            app.sched.defer(PEN_WORK, 60000.0, false).to_string()
        }
        "pen_probe_result" => {
            host.out().unlisten(PEN_DOWN);
            host.out().unlisten(PEN_MOVE);
            app.sched.cancel(PEN_WORK);
            app.sched.set_busy(Mod::App, false);
            app.st.app.pen.to_string()
        }
        _ => return None,
    })
}

/// Routes one test command and writes its `test_result`.
pub fn dispatch(app: &mut App, host: &mut dyn Host, name: &str, args: &str) {
    let (module, command) = name.rsplit_once('.').unwrap_or(("", name));
    let result = if let Some(m) = MODULES.iter().find(|m| m.name == module) {
        (m.test_cmd)(app, host, command, args)
    } else if let Some(core) = module.strip_prefix("core.") {
        ihn_core::test_cmd(core, command, args)
    } else if let Some(sync) = module.strip_prefix("sync.") {
        ihn_sync::test_cmd(sync, command, args)
    } else {
        None
    };
    let json = result.unwrap_or_else(|| format!("{{\"error\":{}}}", json_str(&format!("unknown test command {name}"))));
    let out = host.out();
    let m = out.w.rec(fam::FX, fx::TEST_RESULT);
    out.w.str(&json);
    out.w.end(m);
}

/// A JSON string literal (for modules building `test_cmd` results).
pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn state(app: &App) -> String {
    let names: Vec<String> = MODULES.iter().map(|m| json_str(m.name)).collect();
    format!(
        "{{\"version\":{},\"abi\":{},\"flags\":{},\"viewport\":[{},{}],\"dpr\":{},\"ready\":{},\"modules\":[{}]}}",
        json_str(&app.cfg.version),
        crate::abi::IHN_ABI,
        app.cfg.flags,
        app.cfg.viewport.0,
        app.cfg.viewport.1,
        app.cfg.dpr,
        app.is_ready(),
        names.join(",")
    )
}

/// Exercises every DOM op, the 3.1 effects and every synchronous import against the real host. The page checks
/// the resulting DOM under `#ihn-selftest`, then reads `app.selftest_result`.
fn selftest(app: &mut App, host: &mut dyn Host) -> String {
    let out = host.out();
    let root = out.insert_html(
        Node::BODY,
        Node::NONE,
        "<div id=\"ihn-selftest\" hidden><span class=\"a\">x</span><input><button type=\"button\">b</button><dialog></dialog></div>",
    );
    let span = out.child(root, &[0]);
    let input = out.child(root, &[1]);
    let button = out.query(root, "button");
    let dialog = out.query(root, "dialog");
    out.text(span, "héllo");
    out.class(span, "b", true);
    out.class(span, "a", false);
    out.attr(span, "data-k", Some("v"));
    out.attr(span, "data-gone", Some("1"));
    out.attr(span, "data-gone", None);
    out.style(span, "color", Some("red"));
    out.style(span, "backgroundColor", Some("blue"));
    out.style(span, "backgroundColor", None);
    out.style(span, "--ihn-x", Some("1"));
    out.prop(input, "value", Prop::Str("typed"));
    out.prop(input, "disabled", Prop::Bool(true));
    out.prop(root, "scrollTop", Prop::Num(0.0));
    let made = out.create("p");
    out.class_set(made, "c d");
    out.html(made, "<b>k</b>");
    out.insert(root, span, made);
    out.append_html(root, "<i>last</i>");
    let gone = out.create("u");
    out.insert(root, Node::NONE, gone);
    out.remove(gone);
    out.release(gone);
    // A second handle to a node that already has one; releasing it must not hide the first from event paths.
    let bound = out.bind("ihn-selftest");
    out.attr(bound, "data-bound", Some("1"));
    out.release(bound);
    out.dialog(dialog, crate::dom::Dialog::Show);
    out.dialog(dialog, crate::dom::Dialog::Close);
    out.focus(button, true);
    out.blur(button);
    out.select(input);
    out.scroll_to(root, 0.0, f64::NAN, crate::dom::Scroll::Instant);
    out.pointer_capture(button, 1, false);
    out.listen(button, "click", CLICK, 0);
    out.click(button);
    out.listen(button, "click", tag(Mod::App, 7), crate::abi::listen::ONCE);
    out.unlisten(tag(Mod::App, 7));
    out.listen(button, "dragover", DRAGOVER, 0);
    out.listen(button, "dragleave", DRAGLEAVE, 0);
    out.observe(Node::BODY, RESIZED, observe::RESIZE, "");
    out.observe(root, MUTATED, observe::ATTRIBUTES, "data-m");
    out.attr(root, "data-m", Some("1"));
    out.observe(root, tag(Mod::App, 14), observe::CHILD_LIST | observe::SUBTREE, "");
    out.unobserve(tag(Mod::App, 14));
    let anim = out.animate(span, "[{\"opacity\":0},{\"opacity\":1}]", "{\"duration\":50}");
    out.listen(anim, "finish", FINISH, 0);
    out.anim_finish(anim);
    let cancelled = out.animate(span, "[{\"opacity\":1},{\"opacity\":0}]", "{\"duration\":50}");
    out.anim_set(cancelled, crate::dom::AnimPlay::Pause, Some("[{\"opacity\":1}]"), anim);
    out.anim_cancel(cancelled);
    out.release(cancelled);
    for node in [input, dialog, made] {
        out.release(node);
    }
    app.sched.timer(out, TIMER, 1.0);
    app.sched.interval(out, INTERVAL, 1.0);
    app.sched.timer(out, tag(Mod::App, 8), 1.0);
    app.sched.clear(out, tag(Mod::App, 8));
    app.sched.frame(out, Mod::App);
    app.sched.idle(out, IDLE, 50.0);
    app.sched.idle(out, tag(Mod::App, 9), 50.0);
    app.sched.idle_cancel(out, tag(Mod::App, 9));
    app.sched.defer(WORK, 0.0, false);

    // Synchronous imports: each measurement implies a flush, so the nodes above exist by now.
    let connected = host.metric(root, Metric::Connected);
    let body = host.rect(Node::BODY);
    let inner = host.metric(Node::WINDOW, Metric::InnerWidth);
    host.flush();
    let set = host.ls(LsOp::Set, "ihn-selftest", Some("ß1")).is_some();
    let got = host.ls(LsOp::Get, "ihn-selftest", None);
    let long = "x".repeat(5000);
    host.ls(LsOp::SessionSet, "ihn-selftest", Some(&long));
    let session = host.ls(LsOp::SessionGet, "ihn-selftest", None).is_some_and(|v| v == long);
    host.ls(LsOp::Remove, "ihn-selftest", None);
    host.ls(LsOp::SessionRemove, "ihn-selftest", None);
    let missing = host.ls(LsOp::Get, "ihn-selftest", None).is_none();
    let mut random = [0u8; 32];
    host.random(&mut random);
    let lower = host.intl(IntlOp::Lower, "ÀB");
    let compare = host.intl(IntlOp::Compare, "a\0b\0\0");
    let number = host.intl(IntlOp::Number, "1234.5\0\"en-US\"\0");
    let date = host.intl(IntlOp::Date, "0\0date\0\"en-US\"\0{\"timeZone\":\"UTC\",\"month\":\"long\"}");
    host.log(0, "ihn selftest");
    let color = host.style(span, "color");
    let custom = host.style(span, "--ihn-x");
    let media = host.media("(min-width: 1px)") && !host.media("(max-width: 1px)");
    let hit = host.hit(1.0, 1.0).contains(&Node::BODY);
    let active = host.active().first() == Some(&Node::BODY);
    let capture = host.has_capture(button, 1);
    host.out().release(span);
    let st = &mut app.st.app;
    *st = State { root, button, anim, ..State::default() };
    st.sync = format!(
        "{{\"connected\":{connected},\"bodyWidth\":{},\"innerWidth\":{inner},\"lsSet\":{set},\"lsGet\":{},\"session\":{session},\
         \"lsMissing\":{missing},\"random\":{},\"lower\":{},\"compare\":{},\"number\":{},\"date\":{},\"now\":{},\"time\":{},\
         \"color\":{},\"custom\":{},\"media\":{media},\"hit\":{hit},\"active\":{active},\"capture\":{capture}}}",
        body.w,
        json_str(got.as_deref().unwrap_or("")),
        random.iter().any(|&b| b != 0),
        json_str(&lower),
        json_str(&compare),
        json_str(&number),
        json_str(&date),
        host.now() >= 0.0,
        host.time() > 1.6e12,
        json_str(&color),
        json_str(custom.trim()),
    );
    "true".into()
}

/// The observations of the self-test; also removes its nodes.
fn selftest_result(app: &mut App, host: &mut dyn Host) -> String {
    let st = std::mem::take(&mut app.st.app);
    let out = host.out();
    for listener in [CLICK, FINISH, DRAGOVER, DRAGLEAVE] {
        out.unlisten(listener);
    }
    out.unobserve(RESIZED);
    out.unobserve(MUTATED);
    out.remove(st.root);
    for node in [st.root, st.button, st.anim] {
        out.release(node);
    }
    format!(
        "{{\"clicks\":{},\"pathOk\":{},\"timer\":{},\"interval\":{},\"frame\":{},\"idle\":{},\"finish\":{},\"work\":{},\
         \"dragover\":{},\"dragleave\":{},\"resized\":{},\"mutated\":{},\"sync\":{}}}",
        st.clicks,
        st.path_ok,
        st.timer,
        st.interval,
        st.frame,
        st.idle,
        st.finish,
        st.work,
        st.dragover,
        st.dragleave,
        st.resized,
        st.mutated,
        if st.sync.is_empty() { "null" } else { &st.sync }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{kind, Config};
    use crate::host::FakeHost;
    use ihn_core::codec::Writer;

    fn cmd(app: &mut App, host: &mut FakeHost, name: &str) -> String {
        let mut w = Writer::default();
        w.str(name);
        w.str("null");
        app.event(host, kind::TEST, &w.buf);
        host.take()
    }

    fn booted() -> (App, FakeHost) {
        let (mut app, mut host) = (App::new(Config { version: "6.0.0".into(), dpr: 1.0, flags: crate::abi::mode::E2E, ..Config::default() }), FakeHost::default());
        app.init(&mut host);
        // The frame modules asked for while booting is painted before a test starts.
        app.frame(&mut host, 0.0);
        host.take();
        (app, host)
    }

    #[test]
    fn commands_route_by_module_prefix() {
        let (mut app, mut host) = booted();
        let state = cmd(&mut app, &mut host, "app.state");
        assert!(state.starts_with("test_result \"{\\\"version\\\":\\\"6.0.0\\\",\\\"abi\\\":6,"), "{state}");
        assert!(state.contains("\\\"drive.client\\\"") && state.contains("\\\"ready\\\":true"));
        for unknown in ["app.nope", "drive.client.nope", "core.codec.x", "core.jsfmt.x", "sync.wire.x", "nope", "zzz.y"] {
            assert_eq!(cmd(&mut app, &mut host, unknown), format!("test_result \"{{\\\"error\\\":\\\"unknown test command {unknown}\\\"}}\"\n"));
        }
        assert_eq!(json_str("a\"b\\c\n\u{1}é"), "\"a\\\"b\\\\c\\n\\u0001é\"");
        // Outside e2e mode test commands are ignored.
        app.cfg.flags = 0;
        assert_eq!(cmd(&mut app, &mut host, "app.state"), "");
    }

    /// DOM-op golden test: the exact records of the self-test, one line per op, covering every DOM op of the ABI.
    #[test]
    fn selftest_emits_every_dom_op() {
        let (mut app, mut host) = booted();
        let text = cmd(&mut app, &mut host, "app.selftest");
        let golden = "\
insert_html 1024 4 0 \"<div id=\\\"ihn-selftest\\\" hidden><span class=\\\"a\\\">x</span><input><button type=\\\"button\\\">b</button><dialog></dialog></div>\"
child 1025 1024 [0]
child 1026 1024 [1]
query 1027 1024 \"button\"
query 1028 1024 \"dialog\"
text 1025 \"héllo\"
class 1025 \"b\" 1
class 1025 \"a\" 0
attr 1025 \"data-k\" \"v\"
attr 1025 \"data-gone\" \"1\"
attr 1025 \"data-gone\" null
style 1025 \"color\" \"red\"
style 1025 \"backgroundColor\" \"blue\"
style 1025 \"backgroundColor\" null
style 1025 \"--ihn-x\" \"1\"
prop 1026 \"value\" \"typed\"
prop 1026 \"disabled\" true
prop 1024 \"scrollTop\" 0
create 1029 \"p\"
class_set 1029 \"c d\"
html 1029 \"<b>k</b>\"
insert 1024 1025 1029
insert_html 0 1024 0 \"<i>last</i>\"
create 1030 \"u\"
insert 1024 0 1030
remove 1030
release 1030
bind 1030 \"ihn-selftest\"
attr 1030 \"data-bound\" \"1\"
release 1030
dialog 1028 0
dialog 1028 2
focus 1027 1
blur 1027
select 1026
scroll_to 1024 0 NaN 2
pointer_capture 1027 1 0
listen 1027 \"click\" 1 0
click 1027
listen 1027 \"click\" 7 4
unlisten 7
listen 1027 \"dragover\" 10 0
listen 1027 \"dragleave\" 11 0
observe 4 12 0 \"\"
observe 1024 13 1 \"data-m\"
attr 1024 \"data-m\" \"1\"
observe 1024 14 6 \"\"
unobserve 14
animate 1030 1025 \"[{\\\"opacity\\\":0},{\\\"opacity\\\":1}]\" \"{\\\"duration\\\":50}\"
listen 1030 \"finish\" 3 0
anim_finish 1030
animate 1031 1025 \"[{\\\"opacity\\\":1},{\\\"opacity\\\":0}]\" \"{\\\"duration\\\":50}\"
anim_set 1031 2 \"[{\\\"opacity\\\":1}]\" 1030
anim_cancel 1031
release 1031
release 1026
release 1028
release 1029
timer_set 2 1 0
timer_set 6 1 1
timer_set 8 1 0
timer_clear 8
raf_request
idle_request 4 50
idle_request 9 50
idle_cancel 9
release 1025
test_result \"true\"
timer_set 4278190080 1 0
";
        // Modules hold dynamic handles of their own since boot: the golden text counts from the first free one.
        let base: u32 = text.split(' ').nth(1).and_then(|n| n.parse().ok()).unwrap_or(1024);
        let golden = (0..8u32).rev().fold(golden.to_string(), |g, i| g.replace(&(1024 + i).to_string(), &(base + i).to_string()));
        assert_eq!(text, golden);
        let ops: std::collections::BTreeSet<&str> = golden.lines().filter_map(|l| l.split(' ').next()).collect();
        for (_, name, _) in crate::dom::LAYOUT {
            assert!(ops.contains(name), "DOM op {name} is not covered by the self-test");
        }
        assert!(host.flushes >= 10 && host.logs == [(0, "ihn selftest".to_string())]);
        assert!(host.local.is_empty() && host.session.is_empty());

        // Feed the events the host would send back and read the observations.
        let mut click = Writer::default();
        click.u32(CLICK);
        click.u32(base + 3);
        click.u8(ety::CLICK);
        click.u32(ev::EXACT);
        click.f64(5.0);
        click.i32(0);
        click.u32(0);
        for _ in 0..4 {
            click.f64(0.0);
        }
        click.u32(0);
        click.i32(0);
        click.u8(0);
        click.u32(0);
        for _ in 0..3 {
            click.str("");
        }
        click.u32(3);
        click.u32(base + 3);
        click.u32(base);
        click.u32(4);
        click.u32(0);
        click.u32(0);
        click.u32(0);
        assert_eq!(app.event(&mut host, kind::DOM, &click.buf), 0);
        // The same payload sent to the dragover listener asks for the copy effect.
        click.buf[..4].copy_from_slice(&DRAGOVER.to_le_bytes());
        assert_eq!(app.event(&mut host, kind::DOM, &click.buf), ret::PREVENT_DEFAULT | ret::DROP_COPY);
        let mut w = Writer::default();
        w.u32(TIMER);
        app.event(&mut host, kind::TIMER, &w.buf);
        app.frame(&mut host, 16.0);
        host.take();
        let result = cmd(&mut app, &mut host, "app.selftest_result");
        assert!(result.starts_with(&format!("unlisten 1\nunlisten 3\nunlisten 10\nunlisten 11\nunobserve 12\nunobserve 13\nremove {base}\nrelease {base}\nrelease {}\nrelease {}\ntest_result ", base + 3, base + 6)), "{result}");
        assert!(result.contains("{\\\"clicks\\\":1,\\\"pathOk\\\":true,\\\"timer\\\":true,\\\"interval\\\":0,\\\"frame\\\":true,\\\"idle\\\":false,\\\"finish\\\":false,\\\"work\\\":false,\\\"dragover\\\":false,\\\"dragleave\\\":false,\\\"resized\\\":false,\\\"mutated\\\":false,\\\"sync\\\":{\\\"connected\\\":0,"), "{result}");
    }
}
