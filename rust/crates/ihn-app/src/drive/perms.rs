//! Drive permissions calls of the share modal (spec/home.md §9.1, 5.18.0 §4.21a). Each call returns the id of the
//! `Event::Io` that completes it at the caller's module (`drive::client` conventions: `status >= 0` with the
//! response, else the 5.18.0 error message). [`set_link`] is the only multi-step call: it lists the permissions
//! and then deletes, patches or creates the `anyone` permission; its caller gets one completion at the end.
// v5: app-v5.js:fetchDrivePermissions / addDrivePermission / setDriveLinkPermission / updateDrivePermissionRole /
// revokeDrivePermission / fetchDriveOwnerInfo

use crate::drive::client::{self, Body, Req};
use crate::shell::fx::{self, Response};
use crate::shell::json::Json;
use crate::testapi::json_str;
use crate::{shell, App, Event, Host, Mod};

const FILES: &str = "https://www.googleapis.com/drive/v3/files/";
const LIST_FIELDS: &str = "permissions(id,type,role,emailAddress,displayName,photoLink)";
const OWNER_FIELDS: &str = "ownedByMe,capabilities(canEdit),owners(displayName,emailAddress,photoLink),resourceKey";

/// One Drive permission as the share modal reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Perm {
    pub id: String,
    /// `user`, `group`, `domain` or `anyone`.
    pub kind: String,
    pub role: String,
    pub email: String,
    pub name: String,
    pub photo: String,
}

/// The `permissions` of a `permissions.list` answer (`[]` when the member is missing, `None` when not JSON).
pub fn parse_list(body: &str) -> Option<Vec<Perm>> {
    let j = Json::parse(body)?;
    let list = j.get("permissions").map_or(&[][..], Json::items);
    Some(list.iter().map(|p| Perm { id: p.text("id").into(), kind: p.text("type").into(), role: p.text("role").into(), email: p.text("emailAddress").into(), name: p.text("displayName").into(), photo: p.text("photoLink").into() }).collect())
}

/// What `fetchDriveOwnerInfo` reads: `ownedByMe`, `capabilities.canEdit` (only when booleans), the resource key
/// and the first owner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OwnerInfo {
    pub owned_by_me: Option<bool>,
    pub can_edit: Option<bool>,
    pub resource_key: String,
    pub owner: Option<Perm>,
}

pub fn parse_owner(body: &str) -> Option<OwnerInfo> {
    let j = Json::parse(body)?;
    let flag = |v: Option<&Json>| match v {
        Some(Json::Bool(b)) => Some(*b),
        _ => None,
    };
    Some(OwnerInfo {
        owned_by_me: flag(j.get("ownedByMe")),
        can_edit: flag(j.get("capabilities").and_then(|c| c.get("canEdit"))),
        resource_key: j.text("resourceKey").into(),
        owner: j.get("owners").and_then(|o| o.items().first()).map(|o| Perm { name: o.text("displayName").into(), email: o.text("emailAddress").into(), photo: o.text("photoLink").into(), ..Perm::default() }),
    })
}

fn json_req(method: &'static str, url: String, body: String) -> Req {
    Req { method, url, headers: "Content-Type: application/json\n".into(), body: Body::Bytes(body.into_bytes()), ..Req::default() }
}

/// `GET files/{id}/permissions?fields=permissions(id,type,role,emailAddress,displayName,photoLink)`.
pub fn list(app: &mut App, host: &mut dyn Host, owner: Mod, file: &str) -> u32 {
    client::request(app, host, owner, Req::get(&format!("{FILES}{file}/permissions?fields={}", fx::url_encode(LIST_FIELDS))))
}

/// `POST files/{id}/permissions?sendNotificationEmail=true` `{type:'user', role, emailAddress}`.
pub fn add(app: &mut App, host: &mut dyn Host, owner: Mod, file: &str, email: &str, role: &str) -> u32 {
    let body = format!("{{\"type\":\"user\",\"role\":{},\"emailAddress\":{}}}", json_str(role), json_str(email));
    client::request(app, host, owner, json_req("POST", format!("{FILES}{file}/permissions?sendNotificationEmail=true"), body))
}

/// `PATCH files/{id}/permissions/{pid}` `{role}`.
pub fn update_role(app: &mut App, host: &mut dyn Host, owner: Mod, file: &str, perm: &str, role: &str) -> u32 {
    client::request(app, host, owner, json_req("PATCH", format!("{FILES}{file}/permissions/{perm}"), format!("{{\"role\":{}}}", json_str(role))))
}

/// `DELETE files/{id}/permissions/{pid}`.
pub fn revoke(app: &mut App, host: &mut dyn Host, owner: Mod, file: &str, perm: &str) -> u32 {
    client::request(app, host, owner, Req { method: "DELETE", ..Req::get(&format!("{FILES}{file}/permissions/{perm}")) })
}

/// `GET files/{id}?fields=ownedByMe,capabilities(canEdit),owners(…),resourceKey[&resourceKey]` (read with
/// [`parse_owner`]).
pub fn owner_info(app: &mut App, host: &mut dyn Host, owner: Mod, file: &str, resource_key: &str) -> u32 {
    let mut url = format!("{FILES}{file}?fields={}", fx::url_encode(OWNER_FIELDS));
    if !resource_key.is_empty() {
        url.push_str(&format!("&resourceKey={}", fx::url_encode(resource_key)));
    }
    client::request(app, host, owner, Req::get(&url))
}

/// `GET files/{id}?fields=id,name,resourceKey` (the copied link's name and key).
pub fn file_info(app: &mut App, host: &mut dyn Host, owner: Mod, file: &str) -> u32 {
    client::request(app, host, owner, Req::get(&format!("{FILES}{file}?fields={}", fx::url_encode("id,name,resourceKey"))))
}

/// A running [`set_link`].
struct Link {
    /// The caller's completion id.
    id: u32,
    /// This module's id of the step in flight.
    step: u32,
    file: String,
    role: String,
    listed: bool,
}

#[derive(Default)]
pub struct State {
    links: Vec<Link>,
}

/// `setDriveLinkPermission(file, role)`: `role` is `reader`, `writer` or `restricted`. Completes with status 0 and
/// no data on success.
pub fn set_link(app: &mut App, host: &mut dyn Host, owner: Mod, file: &str, role: &str) -> u32 {
    let id = app.req(owner);
    let step = list(app, host, Mod::DrivePerms, file);
    app.st.drive_perms.links.push(Link { id, step, file: file.into(), role: role.into(), listed: false });
    id
}

// v5: app-v5.js:setDriveLinkPermission
fn link_step(app: &mut App, host: &mut dyn Host, io: &crate::Io) {
    let links = &mut app.st.drive_perms.links;
    let Some(at) = links.iter().position(|l| l.step == io.req) else { return };
    if io.status < 0 || links[at].listed {
        let l = links.swap_remove(at);
        let status = io.status.min(0);
        return shell::deliver(app, host, l.id, status, 0, if status < 0 { io.data } else { &[] });
    }
    let Some(perms) = parse_list(Response::parse(io.data).text()) else {
        let l = links.swap_remove(at);
        return shell::deliver(app, host, l.id, client::err::NETWORK, 0, b"Drive returned an invalid permission list");
    };
    links[at].listed = true;
    let (file, role) = (links[at].file.clone(), links[at].role.clone());
    let anyone = perms.into_iter().find(|p| p.kind == "anyone");
    let step = match (role.as_str(), anyone) {
        ("restricted", Some(p)) => revoke(app, host, Mod::DrivePerms, &file, &p.id),
        ("restricted", None) => 0,
        (_, Some(p)) if p.role != role => update_role(app, host, Mod::DrivePerms, &file, &p.id, &role),
        (_, Some(_)) => 0,
        _ => {
            let body = format!("{{\"type\":\"anyone\",\"role\":{},\"allowFileDiscovery\":false}}", json_str(&role));
            client::request(app, host, Mod::DrivePerms, json_req("POST", format!("{FILES}{file}/permissions?fields=id,type,role"), body))
        }
    };
    let links = &mut app.st.drive_perms.links;
    let Some(at) = links.iter().position(|l| l.step == io.req) else { return };
    if step == 0 {
        let l = links.swap_remove(at);
        shell::deliver(app, host, l.id, 0, 0, &[]);
    } else {
        links[at].step = step;
    }
}

pub fn init(_app: &mut App, _host: &mut dyn Host) {}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    if let Event::Io(io) = event {
        link_step(app, host, io);
    }
    0
}

/// `drive.perms.parse` `{list}` → the permissions as `[[id, type, role, email]…]` (unit-level check of the reader).
pub fn test_cmd(_app: &mut App, _host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    (name == "parse").then(|| {
        let rows: Vec<String> = parse_list(args).unwrap_or_default().iter().map(|p| format!("[{},{},{},{}]", json_str(&p.id), json_str(&p.kind), json_str(&p.role), json_str(&p.email))).collect();
        format!("[{}]", rows.join(","))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_and_owner_info_are_read_like_5_18_0() {
        let list = parse_list(r#"{"permissions":[{"id":"1","type":"user","role":"owner","emailAddress":"a@x","displayName":"A"},{"id":"anyoneWithLink","type":"anyone","role":"reader"}]}"#).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].role.as_str(), list[0].name.as_str(), list[1].kind.as_str()), ("owner", "A", "anyone"));
        assert_eq!(parse_list("{}"), Some(vec![]));
        assert_eq!(parse_list("<html>"), None);
        let o = parse_owner(r#"{"ownedByMe":false,"capabilities":{"canEdit":true},"owners":[{"displayName":"Ana","emailAddress":"ana@x"}],"resourceKey":"rk"}"#).unwrap();
        assert_eq!((o.owned_by_me, o.can_edit, o.resource_key.as_str(), o.owner.unwrap().name.as_str()), (Some(false), Some(true), "rk", "Ana"));
        let o = parse_owner(r#"{"ownedByMe":"yes"}"#).unwrap();
        assert_eq!((o.owned_by_me, o.can_edit, o.owner), (None, None, None));
    }
}
