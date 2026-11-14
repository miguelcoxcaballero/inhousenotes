//! Drive requests of the export path (ARCHITECTURE §8.3, SYNC §14–§15): the first create with the reserved id, the
//! recovered copy of a foreign revision, the head download and the name push. Owned by W5.4. (The conditional
//! export PUT and `files.get` of the upload loop are executed by `drive::relay` for the sync engine.)

use crate::drive::client::{self, Body, Req};
use crate::shell::fx;
use crate::testapi::json_str;
use crate::{App, Event, Host, Mod};

const API: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const BOUNDARY: &str = "ihn6-export-boundary";
/// The create answers these itself: 409 = the reserved id exists (a lost response, ARCHITECTURE §8.3).
const ACCEPT: &[i32] = &[400, 403, 404, 409];
pub const CREATE_FIELDS: &str = "id,headRevisionId,modifiedTime,name";

fn parents_json(parents: &[String]) -> String { parents.iter().map(|p| json_str(p)).collect::<Vec<_>>().join(",") }

fn meta_json(id: Option<&str>, name: &str, parents: &[String], app: &[(&str, String)]) -> String {
    let id = id.map_or(String::new(), |id| format!("\"id\":{},", json_str(id)));
    let props: Vec<String> = app.iter().map(|(k, v)| format!("{}:{}", json_str(k), json_str(v))).collect();
    let app = if props.is_empty() { String::new() } else { format!(",\"appProperties\":{{{}}}", props.join(",")) };
    format!("{{{id}\"name\":{},\"mimeType\":\"application/pdf\",\"parents\":[{}]{app}}}", json_str(name), parents_json(parents))
}

fn part_head(meta: &str) -> String { format!("--{BOUNDARY}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{BOUNDARY}\r\nContent-Type: application/pdf\r\n\r\n") }
fn part_tail() -> String { format!("\r\n--{BOUNDARY}--") }
fn multipart_header() -> String { format!("Content-Type: multipart/related; boundary={BOUNDARY}\n") }

/// `multipart/related` body: the JSON metadata then the PDF.
pub fn multipart(meta: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = part_head(meta).into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(part_tail().as_bytes());
    body
}

/// First upload = create with the document's reserved id (`flags.created = false` until it succeeds). The answer is
/// the file's [`CREATE_FIELDS`]; 409 means an earlier create whose answer was lost succeeded.
pub fn create(app: &mut App, host: &mut dyn Host, owner: Mod, id: &str, name: &str, parents: &[String], vv_tag: &str, bytes: &[u8]) -> u32 {
    let meta = meta_json(Some(id), name, parents, &[("ihn6", "1".into()), ("ihn6_vv", vv_tag.into())]);
    let req = Req {
        method: "POST",
        url: format!("{UPLOAD}?uploadType=multipart&fields={}", fx::url_encode(CREATE_FIELDS)),
        headers: multipart_header(),
        body: Body::Bytes(multipart(&meta, bytes)),
        accept: ACCEPT,
        timeout_ms: 120_000.0,
        ..Req::default()
    };
    client::request(app, host, owner, req)
}

/// The sibling copy that preserves a foreign revision before the next export overwrites it (SYNC §15.1, 5.18.0
/// recovery rule): `name` next to the document, the bytes of `blob` unchanged.
pub fn recovered_copy(app: &mut App, host: &mut dyn Host, owner: Mod, name: &str, parents: &[String], blob: u32) -> u32 {
    let meta = meta_json(None, name, parents, &[]);
    let (head, tail, body) = (fx::new_obj(app), fx::new_obj(app), fx::new_obj(app));
    let out = host.out();
    fx::blob_new(out, head, "", part_head(&meta).as_bytes());
    fx::blob_new(out, tail, "", part_tail().as_bytes());
    fx::blob_concat(out, body, "", &[head, blob, tail]);
    out.obj_release(head);
    out.obj_release(tail);
    let req = Req {
        method: "POST",
        url: format!("{UPLOAD}?uploadType=multipart&fields=id"),
        headers: multipart_header(),
        body: Body::Blob { obj: body, start: 0.0, end: -1.0 },
        accept: ACCEPT,
        timeout_ms: 120_000.0,
        ..Req::default()
    };
    let id = client::request(app, host, owner, req);
    app.st.drive_revisions.bodies.push((id, body));
    id
}

/// The head revision's bytes as a Blob (`Io::obj`).
pub fn download(app: &mut App, host: &mut dyn Host, owner: Mod, file: &str, resource_key: &str) -> u32 {
    let mut url = format!("{API}/{}?alt=media", fx::url_encode(file));
    if !resource_key.is_empty() {
        url.push_str(&format!("&resourceKey={}", fx::url_encode(resource_key)));
    }
    client::request(app, host, owner, Req { blob: true, timeout_ms: 120_000.0, ..Req::get(&url) })
}

/// `PATCH name` (SYNC §14.7: an export pushes the name when the `DocName` register is newer than the Drive name).
pub fn rename(app: &mut App, host: &mut dyn Host, owner: Mod, file: &str, name: &str) -> u32 {
    let req = Req {
        method: "PATCH",
        url: format!("{API}/{}?fields=name", fx::url_encode(file)),
        headers: "Content-Type: application/json\n".into(),
        body: Body::Bytes(format!("{{\"name\":{}}}", json_str(name)).into_bytes()),
        accept: ACCEPT,
        ..Req::default()
    };
    client::request(app, host, owner, req)
}

/// A request body blob made here is released when its request ends (called by the owner with the event).
pub fn release_body(app: &mut App, host: &mut dyn Host, req: u32) {
    let st = &mut app.st.drive_revisions;
    if let Some(at) = st.bodies.iter().position(|b| b.0 == req) {
        let (_, obj) = st.bodies.swap_remove(at);
        host.out().obj_release(obj);
    }
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    /// `(request, body blob)` of uploads built from blobs.
    bodies: Vec<(u32, u32)>,
}

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(_app: &mut App, _host: &mut dyn Host, _event: &Event) -> u32 { 0 }

/// `__IHN_TEST_API__.cmd('drive.revisions.<name>', args)`: none.
pub fn test_cmd(_app: &mut App, _host: &mut dyn Host, _name: &str, _args: &str) -> Option<String> { None }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_metadata_and_body() {
        let meta = meta_json(Some("ID1"), "Notes.pdf", &["F1".into()], &[("ihn6", "1".into())]);
        assert_eq!(meta, r#"{"id":"ID1","name":"Notes.pdf","mimeType":"application/pdf","parents":["F1"],"appProperties":{"ihn6":"1"}}"#);
        assert_eq!(meta_json(None, "a (recovered Drive version).pdf", &[], &[]), r#"{"name":"a (recovered Drive version).pdf","mimeType":"application/pdf","parents":[]}"#);
        let body = multipart("{}", b"%PDF");
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("\r\n\r\n{}\r\n") && text.contains("application/pdf\r\n\r\n%PDF\r\n--") && text.ends_with("--"));
    }
}
