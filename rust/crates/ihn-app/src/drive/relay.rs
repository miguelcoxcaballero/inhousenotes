//! Drive executor of `ihn-sync`'s requests (SYNC §8.2–§8.6, §14.2; NOTES-W2.7 "executor contract"): the poll,
//! relay/archive file reads and writes, the record `properties` PATCH, the sync folder, the id pool and the export
//! PUT. Owned by W4.5. Sync never sees JSON: a `Meta` answer is re-encoded as [`DriveMeta`], `GenerateIds` as one id
//! per line, `SyncFolder` as `<id>` or `<id>\n1`, `Put` as the new `headRevisionId`; every other answer is the raw
//! body. Status 0 = no answer (network, deadline, no token); errors keep Drive's JSON body (its `reason`).

use crate::drive::client::{self, Body, Req};
use crate::shell::{fx, json::Json};
use crate::testapi::json_str;
use crate::{collab, App, Event, Host, Mod};
use ihn_core::catalog::parse_time;
use ihn_sync::{DriveMeta, HttpReq, SyncIo};

const API: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const META_FIELDS: &str = "properties,appProperties,headRevisionId,version,modifiedTime,name,trashed,capabilities(canEdit),permissionIds";
/// Statuses sync decides on itself (with Drive's error body).
const ACCEPT: &[i32] = &[400, 403, 404, 409, 412, 429];
const BOUNDARY: &str = "ihn6-relay-boundary";
pub const SYNC_FOLDER_NAME: &str = "Inhouse Notes sync";

#[derive(Clone, PartialEq, Debug)]
enum Step {
    Plain,
    Meta,
    Ids,
    Head,
    FolderFind,
    FolderCreate,
    FolderShare(String),
}

struct Pending {
    req: u32,
    /// The sync session (`collab`) and its request id.
    gen: u32,
    id: u32,
    step: Step,
}

#[derive(Default)]
pub struct State {
    pending: Vec<Pending>,
    /// The account's sync folder, once a lookup answered.
    folder: String,
    /// A folder lookup is in flight: creates wait for it (a relay file must land in the link-shared folder).
    lookup: bool,
    waiting: Vec<(u32, u32, HttpReq)>,
}

/// Request id of a folder lookup the executor starts itself (no session waits for its answer).
const INTERNAL: u32 = u32::MAX;

/// Requests in flight for sync session `gen`.
pub fn busy(app: &App, gen: u32) -> usize {
    let st = &app.st.drive_relay;
    st.pending.iter().filter(|p| p.gen == gen).count() + st.waiting.iter().filter(|w| w.0 == gen).count()
}

/// Aborts every request of session `gen` (no answer follows).
pub fn abort(app: &mut App, host: &mut dyn Host, gen: u32) {
    let st = &mut app.st.drive_relay;
    let (gone, keep) = std::mem::take(&mut st.pending).into_iter().partition::<Vec<_>, _>(|p| p.gen == gen);
    st.pending = keep;
    st.waiting.retain(|w| w.0 != gen);
    if gone.iter().any(|p| matches!(p.step, Step::FolderFind | Step::FolderCreate | Step::FolderShare(_))) {
        st.lookup = false;
    }
    for p in gone {
        client::abort(app, host, p.req);
    }
}

fn json_map(pairs: &[(String, Option<String>)]) -> String {
    let body: Vec<String> = pairs.iter().map(|(k, v)| format!("{}:{}", json_str(k), v.as_deref().map_or("null".into(), json_str))).collect();
    format!("{{{}}}", body.join(","))
}
fn json_pairs(pairs: &[(String, String)]) -> Vec<(String, Option<String>)> { pairs.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect() }

fn multipart(meta: &str, mime: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let mut body = format!("--{BOUNDARY}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{BOUNDARY}\r\nContent-Type: {mime}\r\n\r\n").into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--").as_bytes());
    (format!("Content-Type: multipart/related; boundary={BOUNDARY}\n"), body)
}

/// Starts `req` for session `gen` (its id `id`); `folder` = the account's sync folder (parents of `Create`).
pub fn exec(app: &mut App, host: &mut dyn Host, gen: u32, id: u32, req: HttpReq, folder: &str) {
    let st = &mut app.st.drive_relay;
    let folder = if folder.is_empty() { st.folder.clone() } else { folder.to_string() };
    if let HttpReq::Create { .. } = req {
        if folder.is_empty() || st.lookup {
            st.waiting.push((gen, id, req));
            if !std::mem::replace(&mut st.lookup, true) {
                exec(app, host, 0, INTERNAL, HttpReq::SyncFolder, "");
            }
            return;
        }
    }
    if req == HttpReq::SyncFolder {
        st.lookup = true;
    }
    let folder = &folder[..];
    let mut step = Step::Plain;
    let r = match req {
        HttpReq::Meta { file } => {
            step = Step::Meta;
            Req { url: format!("{API}/{}?fields={}", fx::url_encode(&file), fx::url_encode(META_FIELDS)), headers: "Cache-Control: no-cache\n".into(), accept: ACCEPT, timeout_ms: 30000.0, ..Req::default() }
        }
        HttpReq::Media { file } => Req { url: format!("{API}/{}?alt=media", fx::url_encode(&file)), accept: ACCEPT, timeout_ms: 30000.0, ..Req::default() },
        HttpReq::Create { file, name, app: props, bytes, keepalive } => {
            let parents = if folder.is_empty() { String::new() } else { format!(",\"parents\":[{}]", json_str(folder)) };
            let meta = format!("{{\"id\":{},\"name\":{},\"mimeType\":\"application/octet-stream\"{parents},\"appProperties\":{}}}", json_str(&file), json_str(&name), json_map(&json_pairs(&props)));
            let (headers, body) = multipart(&meta, "application/octet-stream", &bytes);
            Req { method: "POST", url: format!("{UPLOAD}?uploadType=multipart&fields=id"), headers, body: Body::Bytes(body), accept: ACCEPT, keepalive, ..Req::default() }
        }
        HttpReq::Update { file, bytes, keepalive } => Req {
            method: "PATCH",
            url: format!("{UPLOAD}/{}?uploadType=media&fields=id", fx::url_encode(&file)),
            headers: "Content-Type: application/octet-stream\n".into(),
            body: Body::Bytes(bytes),
            accept: ACCEPT,
            keepalive,
            ..Req::default()
        },
        HttpReq::Props { file, set, app: props, keepalive } => {
            let mut parts = Vec::new();
            if !set.is_empty() {
                parts.push(format!("\"properties\":{}", json_map(&set)));
            }
            if !props.is_empty() {
                parts.push(format!("\"appProperties\":{}", json_map(&props)));
            }
            Req {
                method: "PATCH",
                url: format!("{API}/{}?fields=id", fx::url_encode(&file)),
                headers: "Content-Type: application/json\n".into(),
                body: Body::Bytes(format!("{{{}}}", parts.join(",")).into_bytes()),
                accept: ACCEPT,
                keepalive,
                ..Req::default()
            }
        }
        HttpReq::Put { file, if_match, bytes, app: props } => {
            step = Step::Head;
            let meta = format!("{{\"appProperties\":{}}}", json_map(&json_pairs(&props)));
            let (headers, body) = multipart(&meta, "application/pdf", &bytes);
            Req {
                method: "PATCH",
                url: format!("{UPLOAD}/{}?uploadType=multipart&fields=headRevisionId", fx::url_encode(&file)),
                headers: format!("{headers}If-Match: {if_match}\n"),
                body: Body::Bytes(body),
                accept: ACCEPT,
                timeout_ms: 120000.0,
                ..Req::default()
            }
        }
        HttpReq::Delete { file } => Req { method: "DELETE", url: format!("{API}/{}", fx::url_encode(&file)), accept: ACCEPT, ..Req::default() },
        HttpReq::GenerateIds { count } => {
            step = Step::Ids;
            Req::get(&format!("{API}/generateIds?count={count}&space=drive&type=files"))
        }
        HttpReq::SyncFolder => {
            step = Step::FolderFind;
            let q = "'me' in owners and mimeType='application/vnd.google-apps.folder' and appProperties has {key='ihn6Kind' and value='sync-folder'} and trashed=false";
            Req::get(&format!("{API}?q={}&fields=files(id)&pageSize=100&spaces=drive", fx::url_encode(q)))
        }
    };
    send(app, host, gen, id, step, r);
}

fn send(app: &mut App, host: &mut dyn Host, gen: u32, id: u32, step: Step, r: Req) {
    let req = client::request(app, host, Mod::DriveRelay, r);
    app.st.drive_relay.pending.push(Pending { req, gen, id, step });
}

/// Drive's JSON file resource (the `Meta` fields) → [`DriveMeta`].
pub fn meta_of(json: &Json) -> DriveMeta {
    let map = |key: &str| match json.get(key) {
        Some(Json::Obj(members)) => members.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect(),
        _ => Vec::new(),
    };
    DriveMeta {
        props: map("properties"),
        app: map("appProperties"),
        head: json.text("headRevisionId").into(),
        version: json.text("version").parse().unwrap_or(0),
        modified_ms: parse_time(json.text("modifiedTime")).unwrap_or(0.0).max(0.0) as u64,
        name: json.text("name").into(),
        trashed: json.get("trashed").is_some_and(Json::truthy),
        can_edit: json.get("capabilities").and_then(|c| c.get("canEdit")).is_some_and(Json::truthy),
        permission_ids: json.get("permissionIds").map_or(Vec::new(), |a| a.items().iter().filter_map(|p| p.as_str().map(str::to_string)).collect()),
    }
}

/// The folder lookup ended (`folder` empty: it failed): answer its session and start the creates that waited.
fn folder_known(app: &mut App, host: &mut dyn Host, gen: u32, id: u32, folder: String, ev: SyncIo) {
    let st = &mut app.st.drive_relay;
    st.lookup = false;
    if !folder.is_empty() {
        st.folder = folder.lines().next().unwrap_or("").to_string();
    }
    let waiting = std::mem::take(&mut st.waiting);
    if id != INTERNAL {
        collab::http_answer(app, host, gen, ev);
    }
    for (g, i, req) in waiting {
        if app.st.drive_relay.folder.is_empty() {
            collab::http_answer(app, host, g, SyncIo::Http { id: i, status: 0, etag: String::new(), body: Vec::new() });
        } else {
            exec(app, host, g, i, req, "");
        }
    }
}

fn done(app: &mut App, host: &mut dyn Host, io: &crate::Io) {
    let st = &mut app.st.drive_relay;
    let Some(at) = st.pending.iter().position(|p| p.req == io.req) else { return };
    let p = st.pending.swap_remove(at);
    if io.obj != 0 {
        host.out().obj_release(io.obj);
    }
    // `-(HTTP status)` keeps the status (after the 401 retry); other failures are "no answer".
    let (status, etag, body): (u16, String, Vec<u8>) = if io.status >= 0 {
        let r = fx::Response::parse(io.data);
        (io.status as u16, r.header("etag").unwrap_or("").into(), r.body.to_vec())
    } else if io.status <= -100 {
        ((-io.status) as u16, String::new(), io.data.to_vec())
    } else {
        (0, String::new(), Vec::new())
    };
    let ok = (200..300).contains(&status);
    let json = || Json::parse(std::str::from_utf8(&body).unwrap_or(""));
    let body = match (&p.step, ok) {
        (Step::Meta, true) => match json() {
            Some(j) => meta_of(&j).encode(),
            None => return collab::http_answer(app, host, p.gen, SyncIo::Http { id: p.id, status: 0, etag, body: Vec::new() }),
        },
        (Step::Ids, true) => json().map_or(String::new(), |j| j.get("ids").map_or(Vec::new(), |a| a.items().iter().filter_map(Json::as_str).collect::<Vec<_>>()).join("\n")).into_bytes(),
        (Step::Head, true) => json().map_or(String::new(), |j| j.text("headRevisionId").into()).into_bytes(),
        (Step::FolderFind, true) => {
            let mut ids: Vec<String> = json().and_then(|j| j.get("files").map(|f| f.items().iter().map(|e| e.text("id").to_string()).filter(|s| !s.is_empty()).collect())).unwrap_or_default();
            ids.sort();
            if let Some(id) = ids.into_iter().next() {
                return folder_known(app, host, p.gen, p.id, id.clone(), SyncIo::Http { id: p.id, status, etag, body: id.into_bytes() });
            }
            let body = format!("{{\"name\":{},\"mimeType\":\"application/vnd.google-apps.folder\",\"parents\":[\"root\"],\"appProperties\":{{\"ihn6Kind\":\"sync-folder\"}}}}", json_str(SYNC_FOLDER_NAME));
            let r = Req { method: "POST", url: format!("{API}?fields=id"), headers: "Content-Type: application/json\n".into(), body: Body::Bytes(body.into_bytes()), accept: ACCEPT, ..Req::default() };
            return send(app, host, p.gen, p.id, Step::FolderCreate, r);
        }
        (Step::FolderCreate, true) => {
            let folder = json().map(|j| j.text("id").to_string()).unwrap_or_default();
            let body = b"{\"type\":\"anyone\",\"role\":\"reader\",\"allowFileDiscovery\":false}".to_vec();
            let r = Req { method: "POST", url: format!("{API}/{}/permissions?fields=id", fx::url_encode(&folder)), headers: "Content-Type: application/json\n".into(), body: Body::Bytes(body), accept: ACCEPT, ..Req::default() };
            return send(app, host, p.gen, p.id, Step::FolderShare(folder), r);
        }
        // The folder exists; a refused link permission is flag `x` (SYNC §8.2).
        (Step::FolderShare(folder), _) if status != 0 => {
            let text = if ok { folder.clone() } else { format!("{folder}\n1") };
            return folder_known(app, host, p.gen, p.id, folder.clone(), SyncIo::Http { id: p.id, status: 200, etag, body: text.into_bytes() });
        }
        _ => body,
    };
    let ev = SyncIo::Http { id: p.id, status, etag, body };
    match p.step {
        // A lookup that failed: the creates waiting for it fail too (sync retries them).
        Step::FolderFind | Step::FolderCreate | Step::FolderShare(_) => folder_known(app, host, p.gen, p.id, String::new(), ev),
        _ => collab::http_answer(app, host, p.gen, ev),
    }
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match event {
        Event::Io(io) => done(app, host, io),
        // The folder belongs to the account.
        Event::Msg(m) if m.chan == crate::shell::sig::SIGNED_OUT => app.st.drive_relay.folder.clear(),
        _ => {}
    }
    0
}

/// `__IHN_TEST_API__.cmd('drive.relay.<name>', args)`: `busy` → requests in flight.
pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, _args: &str) -> Option<String> {
    match name {
        "busy" => Some(app.st.drive_relay.pending.len().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_json_to_drive_meta() {
        let j = Json::parse(r#"{"properties":{"ihn6_a":"1|x","n":"2"},"appProperties":{"k":"v"},"headRevisionId":"r9","version":"42","modifiedTime":"2026-10-20T08:00:00.000Z","name":"Doc.pdf","trashed":false,"capabilities":{"canEdit":true},"permissionIds":["p1","anyoneWithLink"]}"#).unwrap();
        let m = meta_of(&j);
        assert_eq!(m.props, vec![("ihn6_a".to_string(), "1|x".to_string()), ("n".into(), "2".into())]);
        assert_eq!((m.head.as_str(), m.version, m.name.as_str(), m.trashed, m.can_edit), ("r9", 42, "Doc.pdf", false, true));
        assert_eq!(m.modified_ms, 1_792_483_200_000);
        assert_eq!(m.permission_ids, vec!["p1".to_string(), "anyoneWithLink".into()]);
        assert_eq!(DriveMeta::decode(&m.encode()), Some(m));
    }

    #[test]
    fn json_maps_and_multipart() {
        assert_eq!(json_map(&[("a".into(), Some("x\"y".into())), ("b".into(), None)]), r#"{"a":"x\"y","b":null}"#);
        let (h, b) = multipart("{}", "application/octet-stream", &[1, 2]);
        assert!(h.starts_with("Content-Type: multipart/related; boundary="));
        let text = String::from_utf8_lossy(&b);
        assert!(text.contains("\r\n\r\n{}\r\n") && text.ends_with(&format!("--{BOUNDARY}--")));
    }
}
