//! Updater (ARCHITECTURE §2.3, §3.7): the shared Inhouse services updater is JavaScript loaded unmodified; this
//! module only decides that it exists (app mode) and reveals its settings entry in both profile menus.

use crate::abi::{mode, tag, Event, Node};
use crate::dom::Prop;
use crate::shell::fx;
use crate::{App, Host, Mod, Ui};

#[derive(Default)]
pub struct State {
    create: u32,
    created: bool,
}

const L_SETTINGS: u32 = tag(Mod::Updater, 1); // and 2: the editor menu

// v5: app-v5.js § 4.1 "Inhouse services updater" — only in the Android app.
pub fn init(app: &mut App, host: &mut dyn Host) {
    if !app.cfg.has(mode::APP) || app.cfg.has(mode::HEADLESS) {
        return;
    }
    let req = app.req(Mod::Updater);
    app.st.updater.create = req;
    fx::updater_create(host.out(), req);
}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        // The updater exists: unhide `[data-inhouse-update-settings]` in both menus; a click opens its settings.
        Event::Io(io) if io.req == app.st.updater.create && io.status == 0 => {
            app.st.updater.created = true;
            let out = host.out();
            for (i, menu) in [Ui::DRIVE_PROFILE_MENU, Ui::DRIVE_PROFILE_MENU_EDITOR].into_iter().enumerate() {
                let button: Node = out.query(menu, "[data-inhouse-update-settings]");
                out.prop(button, "hidden", Prop::Bool(false));
                out.listen(button, "click", L_SETTINGS + i as u32, 0);
                out.release(button);
            }
        }
        Event::Dom(_) => fx::updater_settings(host.out()),
        _ => {}
    }
    0
}

pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    (name == "state").then(|| format!("{{\"created\":{}}}", app.st.updater.created))
}
