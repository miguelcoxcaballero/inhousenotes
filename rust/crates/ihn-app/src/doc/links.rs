//! Shared links `?drive=<id>[&resourcekey=…][&name=…][&role=writer|&mode=edit]` (spec/shell.md §4.4): parsing and
//! the signed-in open. The anonymous open of a public link is `doc::public`. Owned by W3.1.
// v5: app-v5.js:showFastInitialView / init ("Handle shared drive links on load") / updateDriveAuthUI

use super::open::{self, File};
use crate::app::Mod;
use crate::drive::client::{self, Req};
use crate::shell::fx::{self, Response};
use crate::shell::json::Json;
use crate::shell::{Status, StatusOpts};
use crate::testapi::json_str;
use crate::{auth, shell, App, Event, Host};

/// A parsed shared link.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    pub id: String,
    pub resource_key: String,
    /// `name`, trimmed, at most 240 characters.
    pub name: String,
    /// `role=writer` or `mode=edit`.
    pub writer: bool,
}

/// The shared link of a page URL, if it has one.
pub fn parse(url: &str) -> Option<Link> {
    let query = fx::url_query(url);
    let param = |name: &str| fx::query_param(query, name).filter(|v| !v.is_empty());
    let id = param("drive")?;
    Some(Link {
        id,
        resource_key: param("resourcekey").or_else(|| param("rk")).unwrap_or_default(),
        name: param("name").map(|n| n.trim().chars().take(240).collect()).unwrap_or_default(),
        writer: param("role").as_deref() == Some("writer") || param("mode").as_deref() == Some("edit"),
    })
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    /// The link of this page load until it opened (5.18.0 `pendingSharedDriveId` …).
    pending: Option<Link>,
    info_req: u32,
    open_req: u32,
}

/// The pending shared link (the anonymous open and the sign-in gate read it).
pub fn pending(app: &App) -> Option<&Link> { app.st.doc_links.pending.as_ref() }

// v5: updateDriveAuthUI "Load pending shared file if present"
fn try_open(app: &mut App, host: &mut dyn Host) {
    let st = &app.st.doc_links;
    let Some(link) = st.pending.clone() else { return };
    if st.info_req != 0 || st.open_req != 0 || open::busy(app) || auth::token(app).is_none() {
        return;
    }
    let mut url = format!("https://www.googleapis.com/drive/v3/files/{}?fields={}", link.id, fx::url_encode("id,name,resourceKey"));
    if !link.resource_key.is_empty() {
        url.push_str(&format!("&resourceKey={}", fx::url_encode(&link.resource_key)));
    }
    shell::status(app, host, "Loading document info...", Status::Info);
    app.st.doc_links.info_req = client::request(app, host, Mod::DocLinks, Req::get(&url));
}

fn open_link(app: &mut App, host: &mut dyn Host, file: File) {
    let req = app.req(Mod::DocLinks);
    app.st.doc_links.open_req = req;
    if !open::open_file(app, host, file, req) {
        app.st.doc_links.open_req = 0;
    }
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(app: &mut App, _host: &mut dyn Host) { app.st.doc_links.pending = parse(&app.cfg.url); }

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Msg(m) if m.chan == shell::sig::AUTH => try_open(app, host),
        Event::Msg(m) if m.chan == shell::sig::SIGNED_OUT => app.st.doc_links = State::default(),
        Event::Io(io) if io.req == app.st.doc_links.info_req => {
            app.st.doc_links.info_req = 0;
            let Some(link) = app.st.doc_links.pending.clone() else { return 0 };
            let info = if io.status >= 0 { Json::parse(Response::parse(io.data).text()) } else { None };
            let file = match info.filter(|j| !j.text("id").is_empty()) {
                Some(j) => {
                    shell::status(app, host, "Opening document...", Status::Info);
                    let key = j.text("resourceKey");
                    File { id: j.text("id").into(), name: j.text("name").into(), resource_key: if key.is_empty() { link.resource_key } else { key.into() } }
                }
                // v5: the metadata failed — open with the name of the link.
                None => File { id: link.id, name: if link.name.is_empty() { "Shared Document".into() } else { link.name }, resource_key: link.resource_key },
            };
            open_link(app, host, file);
        }
        Event::Io(io) if io.req == app.st.doc_links.open_req => {
            app.st.doc_links.open_req = 0;
            if io.status >= 0 {
                app.st.doc_links.pending = None;
                shell::set_pending_share(app, None);
            } else {
                shell::status(app, host, "Could not open shared document", StatusOpts::from(Status::Error));
            }
        }
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('doc.links.state')` → `{pending, writer}`.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    (name == "state").then(|| {
        let link = app.st.doc_links.pending.clone().unwrap_or_default();
        format!("{{\"pending\":{},\"writer\":{}}}", json_str(&link.id), link.writer)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_parsed_like_5_18_0() {
        assert_eq!(parse("https://inhousenotes.com/?e2e=1"), None);
        assert_eq!(parse("https://inhousenotes.com/?drive="), None);
        let long = "n".repeat(300);
        let link = parse(&format!("https://inhousenotes.com/?drive=abc&public=1&role=writer&name=%20{long}%20&mode=edit&resourcekey=0-key")).unwrap();
        assert_eq!((link.id.as_str(), link.resource_key.as_str(), link.name.len(), link.writer), ("abc", "0-key", 240, true));
        let link = parse("https://inhousenotes.com/?drive=abc&rk=k2&role=reader").unwrap();
        assert_eq!((link.resource_key.as_str(), link.writer, link.name.as_str()), ("k2", false, ""));
        assert!(parse("https://inhousenotes.com/?drive=abc&mode=edit").unwrap().writer);
    }
}
