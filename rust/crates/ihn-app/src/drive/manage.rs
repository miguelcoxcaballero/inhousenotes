//! Drive calls for management (ARCHITECTURE §2.3): create folder, device upload, copy, move, star, colour, bin,
//! restore, and the folder picker's listings. Each function builds 5.18.0's request exactly and starts it for the
//! caller's module (`owner`), which receives the `Event::Io` of the returned id (see `drive::client`). Owned by W5.5.

use super::client::{self, Body, Req};
use crate::shell::fx::url_encode;
use crate::shell::json::Json;
use crate::testapi::json_str;
use crate::{App, Event, Host, Mod};

pub const FILES: &str = "https://www.googleapis.com/drive/v3/files";
pub const FOLDER: &str = "application/vnd.google-apps.folder";

/// `new URLSearchParams(pairs).toString()` (application/x-www-form-urlencoded).
pub fn form(pairs: &[(&str, &str)]) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => (b as char).to_string(),
                b' ' => "+".into(),
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    pairs.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
}

fn json_req(method: &'static str, url: String, body: String) -> Req {
    Req { method, url, headers: "Content-Type: application/json\n".into(), body: Body::Bytes(body.into_bytes()), ..Default::default() }
}

/// `"parents":["<id>"]` unless the folder is My Drive (5.18.0 leaves `parents` undefined there).
fn parents_field(folder: &str) -> String {
    if folder.is_empty() || folder == "root" { String::new() } else { format!(",\"parents\":[{}]", json_str(folder)) }
}

// v5: app-v5.js:createDriveFolder
pub fn create_folder(app: &mut App, host: &mut dyn Host, owner: Mod, parent: &str, name: &str) -> u32 {
    let body = format!("{{\"name\":{},\"mimeType\":\"{FOLDER}\"{}}}", json_str(name), parents_field(parent));
    client::request(app, host, owner, json_req("POST", format!("{FILES}?fields=id,name,mimeType,starred,ownedByMe,folderColorRgb"), body))
}

// v5: app-v5.js:updateDriveStarStatus
pub fn star(app: &mut App, host: &mut dyn Host, owner: Mod, id: &str, on: bool) -> u32 {
    client::request(app, host, owner, json_req("PATCH", format!("{FILES}/{id}?fields=id,starred"), format!("{{\"starred\":{on}}}")))
}

// v5: app-v5.js:updateDriveFolderColor
pub fn color(app: &mut App, host: &mut dyn Host, owner: Mod, id: &str, color: &str) -> u32 {
    client::request(app, host, owner, json_req("PATCH", format!("{FILES}/{id}?fields=id,folderColorRgb"), format!("{{\"folderColorRgb\":{}}}", json_str(color))))
}

// v5: app-v5.js:confirmTrashFile
pub fn trash(app: &mut App, host: &mut dyn Host, owner: Mod, id: &str) -> u32 {
    client::request(app, host, owner, json_req("PATCH", format!("{FILES}/{id}"), "{\"trashed\":true}".into()))
}

// v5: app-v5.js:restoreDriveFile
pub fn restore(app: &mut App, host: &mut dyn Host, owner: Mod, id: &str, parents: &[String]) -> u32 { client::request(app, host, owner, restore_req(id, parents)) }
fn restore_req(id: &str, parents: &[String]) -> Req {
    let joined = parents.join(",");
    let query = if parents.is_empty() { String::new() } else { form(&[("addParents", &joined)]) };
    json_req("PATCH", format!("{FILES}/{id}?{query}"), "{\"trashed\":false}".into())
}

/// `GET files/{id}?fields=<fields>` (`parents` for a move or a path, `id,name,parents` for a folder's metadata,
/// `properties` before a copy).
pub fn get(app: &mut App, host: &mut dyn Host, owner: Mod, id: &str, fields: &str) -> u32 {
    client::request(app, host, owner, Req::get(&format!("{FILES}/{id}?fields={fields}")))
}

// v5: app-v5.js:moveDriveFileToFolder (the PATCH, after `get(…, "parents")`)
pub fn move_to(app: &mut App, host: &mut dyn Host, owner: Mod, id: &str, folder: &str, parents: &[String]) -> u32 { client::request(app, host, owner, move_req(id, folder, parents)) }
fn move_req(id: &str, folder: &str, parents: &[String]) -> Req {
    let (add, remove) = (if folder.is_empty() { "root" } else { folder }, parents.join(","));
    let mut pairs = vec![("addParents", add)];
    if !remove.is_empty() {
        pairs.push(("removeParents", &remove));
    }
    Req { method: "PATCH", url: format!("{FILES}/{id}?{}", form(&pairs)), ..Default::default() }
}

/// `files.copy` with the source's sync properties cleared: every `ihn6_*` key of `properties` (the answer of
/// `get(…, "properties")`) is sent as `null`, so a copy never points at the original's relay (ARCHITECTURE §13).
// v5: app-v5.js:executeDriveCopy
pub fn copy(app: &mut App, host: &mut dyn Host, owner: Mod, id: &str, name: &str, folder: &str, properties: &Json) -> u32 {
    client::request(app, host, owner, copy_req(id, name, folder, properties))
}
fn copy_req(id: &str, name: &str, folder: &str, properties: &Json) -> Req {
    let cleared: Vec<String> = match properties {
        Json::Obj(fields) => fields.iter().filter(|(k, _)| k.starts_with("ihn6_")).map(|(k, _)| format!("{}:null", json_str(k))).collect(),
        _ => Vec::new(),
    };
    let props = if cleared.is_empty() { String::new() } else { format!(",\"properties\":{{{}}}", cleared.join(",")) };
    let body = format!("{{\"name\":{}{}{props}}}", json_str(name), parents_field(folder));
    json_req("POST", format!("{FILES}/{id}/copy"), body)
}

/// 5.18.0's `fields` plus `appProperties`, so sync files can be left out (ARCHITECTURE §13).
// v5: app-v5.js:refreshDriveBin
pub fn bin_list(app: &mut App, host: &mut dyn Host, owner: Mod) -> u32 {
    let q = url_encode(&format!("trashed=true and (mimeType='application/pdf' or mimeType='{FOLDER}')"));
    let fields = url_encode("files(id,name,modifiedTime,createdTime,mimeType,parents,appProperties)");
    client::request(app, host, owner, Req::get(&format!("{FILES}?q={q}&orderBy=modifiedTime desc&pageSize=100&fields={fields}")))
}

/// 5.18.0's `fields` plus `appProperties` (the sync folder is never offered).
// v5: app-v5.js:refreshFolderPickerFolders
pub fn subfolders(app: &mut App, host: &mut dyn Host, owner: Mod, folder: &str) -> u32 {
    let q = url_encode(&format!("'{folder}' in parents and mimeType='{FOLDER}' and trashed=false"));
    let fields = url_encode("files(id,name,folderColorRgb,appProperties)");
    client::request(app, host, owner, Req::get(&format!("{FILES}?q={q}&orderBy=name&pageSize=200&fields={fields}")))
}

/// The session request of a resumable device upload; the answer's `Location` is the session URL for [`upload_put`].
// v5: app-v5.js:uploadDeviceFile → uploadDriveBlobResumable (start)
pub fn upload_start(app: &mut App, host: &mut dyn Host, owner: Mod, name: &str, mime: &str, size: f64, folder: &str) -> u32 {
    let body = format!("{{\"name\":{},\"mimeType\":{}{}}}", json_str(name), json_str(mime), parents_field(folder));
    let headers = format!("X-Upload-Content-Type: {mime}\nX-Upload-Content-Length: {}\nContent-Type: application/json; charset=UTF-8\n", size as u64);
    let url = "https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&fields=id,name,mimeType,parents,modifiedTime,version".into();
    client::request(app, host, owner, Req { method: "POST", url, headers, body: Body::Bytes(body.into_bytes()), timeout_ms: 30000.0, ..Default::default() })
}

/// The bytes of the blob in one `PUT` to the session (`Content-Range: bytes 0-<n-1>/<n>`).
pub fn upload_put(app: &mut App, host: &mut dyn Host, owner: Mod, session: &str, blob: u32, size: f64) -> u32 {
    let n = size as u64;
    let headers = format!("Content-Range: bytes 0-{}/{n}\n", n.saturating_sub(1));
    client::request(app, host, owner, Req { method: "PUT", url: session.into(), headers, body: Body::Blob { obj: blob, start: 0.0, end: -1.0 }, timeout_ms: 300000.0, ..Default::default() })
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State;

/// Called once at boot, in `app::MODULES` order, after the `Ui` table is bound.
pub fn init(_app: &mut App, _host: &mut dyn Host) {}

/// Events addressed to this module (ids made with `abi::tag`) and broadcasts (`Life`, `Msg`). Returns `abi::ret` bits.
pub fn event(_app: &mut App, _host: &mut dyn Host, _event: &Event) -> u32 { 0 }

/// `__IHN_TEST_API__.cmd('<module>.<name>', args)`: JSON text in, JSON text out; `None` = unknown command.
pub fn test_cmd(_app: &mut App, _host: &mut dyn Host, _name: &str, _args: &str) -> Option<String> { None }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_strings_are_url_search_params() {
        assert_eq!(form(&[("addParents", "a"), ("removeParents", "b,c d")]), "addParents=a&removeParents=b%2Cc+d");
    }

    #[test]
    fn requests_are_5_18_0s() {
        let body = |r: &Req| match &r.body { Body::Bytes(b) => String::from_utf8(b.clone()).unwrap(), _ => String::new() };
        let props = Json::parse("{\"ihn6_a\":\"x\",\"keep\":\"1\",\"ihn6_vv\":\"y\"}").unwrap();
        let c = copy_req("f1", "Copy of A.pdf", "fold", &props);
        assert_eq!((c.method, c.url.as_str()), ("POST", "https://www.googleapis.com/drive/v3/files/f1/copy"));
        assert_eq!(body(&c), "{\"name\":\"Copy of A.pdf\",\"parents\":[\"fold\"],\"properties\":{\"ihn6_a\":null,\"ihn6_vv\":null}}");
        assert_eq!(body(&copy_req("f1", "B", "root", &Json::Null)), "{\"name\":\"B\"}");
        assert!(restore_req("f2", &["p1".into(), "p2".into()]).url.ends_with("files/f2?addParents=p1%2Cp2"));
        assert!(restore_req("f2", &[]).url.ends_with("files/f2?"));
        let m = move_req("f3", "", &["p1".into()]);
        assert!(m.url.ends_with("files/f3?addParents=root&removeParents=p1") && matches!(m.body, Body::None));
    }
}
