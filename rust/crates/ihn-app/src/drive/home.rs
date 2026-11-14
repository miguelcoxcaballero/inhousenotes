//! Drive calls for Home: the list and search requests (5.18.0's `q`/`fields`, plus `appProperties` so sync files
//! can be dropped), the full enumeration that keeps the offline catalogue complete, and the thumbnail cache.
//! Implemented by W2.5 (BUILD-PLAN.md).

use crate::drive::client::{self, Req};
use crate::shell::fx::{self, fetch_flag, Fetch, Response};
use crate::shell::json::Json;
use crate::store::{self, Key, Put, Reply, Store, Tx, Value};
use crate::{shell, App, Event, Host, Mod};
use ihn_core::catalog::{Entry, FOLDER, PDF};

const FILES: &str = "https://www.googleapis.com/drive/v3/files";
// v5: app-v5.js:refreshDriveRecents / refreshDriveStarred / refreshDriveShared / refreshDriveFolderContents
const FILE_FIELDS: &str = "files(id,name,mimeType,modifiedTime,createdTime,thumbnailLink,iconLink,parents,starred,ownedByMe,resourceKey,appProperties)";
const MIXED_FIELDS: &str = "files(id,name,modifiedTime,createdTime,thumbnailLink,iconLink,mimeType,parents,starred,ownedByMe,resourceKey,folderColorRgb,appProperties)";
const SHARED_FIELDS: &str = "files(id,name,mimeType,modifiedTime,createdTime,thumbnailLink,iconLink,parents,starred,ownedByMe,resourceKey,folderColorRgb,appProperties)";
const FOLDER_FIELDS: &str = "files(id,name,mimeType,starred,ownedByMe,folderColorRgb,appProperties)";
// v5: offline-library-v5.js:catalogue
const CRAWL_FIELDS: &str = "nextPageToken,files(id,name,mimeType,parents,modifiedTime,createdTime,thumbnailLink,starred,ownedByMe,resourceKey,folderColorRgb,appProperties)";

/// The lists Home asks Drive for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Recents,
    Starred,
    Shared,
    /// Subfolders of the current folder.
    Folders,
    /// PDFs of the current folder.
    Files,
}

fn search_clause(search: &str) -> Option<String> {
    (!search.is_empty()).then(|| format!("name contains '{}'", search.replace('\'', "\\'")))
}

/// The request URL of a list, exactly as 5.18.0 writes it (`q` and `fields` percent-encoded, `orderBy` raw).
/// `folder` is the Drive id of the current folder (`root` for My Drive).
pub fn list_url(list: List, search: &str, folder: &str) -> String {
    let search = search_clause(search);
    let both = format!("(mimeType='{PDF}' or mimeType='{FOLDER}')");
    let mut parts: Vec<String> = match list {
        List::Recents => vec![format!("mimeType='{PDF}'"), "trashed=false".into()],
        List::Starred => vec![format!("starred=true and trashed=false and {both}")],
        List::Shared => vec![both, "sharedWithMe=true".into(), "trashed=false".into()],
        List::Folders | List::Files => vec![format!("'{folder}' in parents"), "trashed=false".into()],
    };
    if list != List::Starred {
        parts.extend(search);
    }
    let (order, size, fields) = match list {
        List::Recents => ("modifiedTime desc", 12, FILE_FIELDS),
        List::Starred => ("modifiedTime desc", 12, MIXED_FIELDS),
        List::Shared => ("modifiedTime desc", 24, SHARED_FIELDS),
        List::Folders => {
            parts.push(format!("mimeType='{FOLDER}'"));
            ("name", 100, FOLDER_FIELDS)
        }
        List::Files => {
            parts.push(format!("mimeType='{PDF}'"));
            ("modifiedTime desc", 100, FILE_FIELDS)
        }
    };
    format!("{FILES}?q={}&orderBy={order}&pageSize={size}&fields={}", fx::url_encode(&parts.join(" and ")), fx::url_encode(fields))
}

/// Starts a list request for `owner`; the completion is an `Event::Io` with the returned id.
pub fn request(app: &mut App, host: &mut dyn Host, owner: Mod, list: List, search: &str, folder: &str) -> u32 {
    client::request(app, host, owner, Req::get(&list_url(list, search, folder)))
}

/// One Drive file resource as a catalogue entry.
pub fn entry(file: &Json) -> Entry {
    let flag = |key: &str| file.get(key).and_then(|v| match v {
        Json::Bool(b) => Some(*b),
        _ => None,
    });
    Entry {
        id: file.text("id").into(),
        name: file.text("name").into(),
        mime: file.text("mimeType").into(),
        modified: file.text("modifiedTime").into(),
        created: file.text("createdTime").into(),
        thumbnail: file.text("thumbnailLink").into(),
        icon: file.text("iconLink").into(),
        parents: file.get("parents").map(|p| p.items().iter().filter_map(Json::as_str).map(String::from).collect()),
        starred: flag("starred").unwrap_or(false),
        owned_by_me: flag("ownedByMe"),
        resource_key: file.text("resourceKey").into(),
        folder_color: file.text("folderColorRgb").into(),
        kind: file.get("appProperties").map_or("", |p| p.text("ihn6Kind")).into(),
        ..Entry::default()
    }
}

/// `(await response.json()).files || []` and `nextPageToken`.
pub fn parse_files(body: &str) -> (Vec<Entry>, String) {
    match Json::parse(body) {
        Some(data) => (data.get("files").map_or(Vec::new(), |f| f.items().iter().map(entry).collect()), data.text("nextPageToken").into()),
        None => (Vec::new(), String::new()),
    }
}

// ---- catalogue entries ⇄ the `catalog` store --------------------------------------------------------------------

pub fn entry_value(e: &Entry) -> Value {
    let s = |t: &str| Value::Str(t.to_string());
    let mut fields = vec![("id", s(&e.id)), ("name", s(&e.name)), ("mimeType", s(&e.mime))];
    for (key, text) in [("modifiedTime", &e.modified), ("createdTime", &e.created), ("thumbnailLink", &e.thumbnail), ("iconLink", &e.icon), ("resourceKey", &e.resource_key), ("folderColorRgb", &e.folder_color), ("localPreview", &e.local_preview), ("ihn6Kind", &e.kind)] {
        if !text.is_empty() {
            fields.push((key, s(text)));
        }
    }
    if let Some(parents) = &e.parents {
        fields.push(("parents", Value::Arr(parents.iter().map(|p| s(p)).collect())));
    }
    fields.push(("starred", Value::Bool(e.starred)));
    if let Some(owned) = e.owned_by_me {
        fields.push(("ownedByMe", Value::Bool(owned)));
    }
    for (key, on) in [("trashed", e.trashed), ("pending", e.pending)] {
        if on {
            fields.push((key, Value::Bool(true)));
        }
    }
    Value::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn entry_from_value(v: &Value) -> Entry {
    let t = |key: &str| v.get(key).text();
    Entry {
        id: t("id"),
        name: t("name"),
        mime: t("mimeType"),
        modified: t("modifiedTime"),
        created: t("createdTime"),
        thumbnail: t("thumbnailLink"),
        icon: t("iconLink"),
        parents: match v.get("parents") {
            Value::Arr(items) => Some(items.iter().map(Value::text).collect()),
            _ => None,
        },
        starred: v.get("starred").flag(),
        owned_by_me: v.get("ownedByMe").as_bool(),
        resource_key: t("resourceKey"),
        folder_color: t("folderColorRgb"),
        trashed: v.get("trashed").flag(),
        pending: v.get("pending").flag(),
        local_preview: t("localPreview"),
        kind: t("ihn6Kind"),
    }
}

// ---- thumbnails -------------------------------------------------------------------------------------------------

/// The size 5.18.0 asks Drive for.
// v5: app-v5.js:normalizeDriveThumbnail
pub fn normalize_thumbnail(link: &str) -> String {
    if link.is_empty() || link.starts_with("data:") || link.starts_with("blob:") || link.starts_with('/') {
        return link.to_string();
    }
    // `link.replace(/=s\d+/, '=s440')`
    let mut resized = link.to_string();
    let mut from = 0;
    while let Some(at) = link[from..].find("=s").map(|i| i + from) {
        let digits = link[at + 2..].bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            resized = format!("{}=s440{}", &link[..at], &link[at + 2 + digits..]);
            break;
        }
        from = at + 2;
    }
    if resized.contains("=s") { resized } else { format!("{resized}&sz=w312-h440") }
}

/// The answer of [`media_url`].
pub enum Media {
    Ready(String),
    /// The waiter gets an `Event::Io`: `status >= 0` and `data` = the URL, or `status < 0` (nothing to show).
    Pending,
}

struct Remembered {
    id: String,
    revision: String,
    url: String,
}

enum Stage {
    Lookup,
    Fetch,
    Url { fresh: bool },
}

struct Job {
    req: u32,
    account: String,
    id: String,
    source: String,
    revision: String,
    stage: Stage,
    /// The stored copy (a Blob handle) found by the lookup, kept until the job ends.
    stored: u32,
    /// The Blob being turned into a URL.
    blob: u32,
    size: f64,
    mime: String,
    waiters: Vec<u32>,
}

/// This module's state, stored in `App::st`.
#[derive(Default)]
pub struct State {
    memory: Vec<Remembered>,
    jobs: Vec<Job>,
    /// Blob URLs to revoke: `(performance.now() when due, url)`.
    retired: Vec<(f64, String)>,
    crawl: Option<Crawl>,
}

struct Crawl {
    account: String,
    req: u32,
    root: String,
    found: Vec<String>,
}

const T_REVOKE: u32 = crate::tag(Mod::DriveHome, 1);
const MEMORY: usize = 128;
/// The catalogue key space of cached thumbnails: `[thumbs(account), "preview:<fileId>"]` → `{blob, rev}`.
fn thumbs(account: &str) -> String { format!("{account}\u{0}thumb") }

/// A displayable URL for the thumbnail `id` of `account`, independent of Drive's expiring links: from memory,
/// from the copy stored on this device, or fetched (and stored) when the revision changed and Drive is reachable.
// v5: offline-media-v5.js:url (Cache Storage there; the `catalog` store here, ARCHITECTURE §7.3)
pub fn media_url(app: &mut App, host: &mut dyn Host, account: &str, id: &str, source: &str, revision: &str, waiter: u32) -> Media {
    if account.is_empty() || source.starts_with("data:") {
        return Media::Ready(source.to_string());
    }
    let st = &mut app.st.drive_home;
    if let Some(found) = st.memory.iter().find(|m| m.id == id && m.revision == revision) {
        return Media::Ready(found.url.clone());
    }
    if let Some(job) = st.jobs.iter_mut().find(|j| j.id == id && j.revision == revision && j.account == account) {
        job.waiters.push(waiter);
        return Media::Pending;
    }
    let req = app.req(Mod::DriveHome);
    app.st.drive_home.jobs.push(Job { req, account: account.into(), id: id.into(), source: source.into(), revision: revision.into(), stage: Stage::Lookup, stored: 0, blob: 0, size: 0.0, mime: String::new(), waiters: vec![waiter] });
    let mut tx = Tx::new();
    tx.get(Store::Catalog, Key::Pair(&thumbs(account), id));
    store::tx(app, host, req, tx);
    Media::Pending
}

/// Forgets the URLs of the signed-out account. v5: offline-media-v5.js:clear
pub fn clear_media(app: &mut App, host: &mut dyn Host) {
    let st = &mut app.st.drive_home;
    for m in st.memory.drain(..) {
        if m.url.starts_with("blob:") {
            fx::url_revoke(host.out(), &m.url);
        }
    }
    st.crawl = None;
}

fn finish(app: &mut App, host: &mut dyn Host, at: usize, url: Option<String>) {
    let job = app.st.drive_home.jobs.remove(at);
    host.out().obj_release(job.stored);
    if job.blob != job.stored {
        host.out().obj_release(job.blob);
    }
    if let Some(url) = &url {
        let now = host.now();
        let st = &mut app.st.drive_home;
        let mut retire = |old: Remembered| {
            if old.url.starts_with("blob:") && old.url != *url {
                st.retired.push((now + 60000.0, old.url));
            }
        };
        if let Some(i) = st.memory.iter().position(|m| m.id == job.id) {
            retire(st.memory.remove(i));
        }
        st.memory.push(Remembered { id: job.id.clone(), revision: job.revision.clone(), url: url.clone() });
        if st.memory.len() > MEMORY {
            retire(st.memory.remove(0));
        }
        if !st.retired.is_empty() {
            app.sched.timer(host.out(), T_REVOKE, 60000.0);
        }
    }
    for waiter in job.waiters {
        match &url {
            Some(url) => shell::deliver(app, host, waiter, 0, 0, url.as_bytes()),
            None => shell::deliver(app, host, waiter, -1, 0, &[]),
        }
    }
}

fn to_url(app: &mut App, host: &mut dyn Host, at: usize, blob: u32, fresh: bool) {
    let job = &mut app.st.drive_home.jobs[at];
    job.blob = blob;
    job.stage = Stage::Url { fresh };
    fx::blob_url(host.out(), job.req, blob);
}

fn media_io(app: &mut App, host: &mut dyn Host, at: usize, io: &crate::Io) {
    let job = &app.st.drive_home.jobs[at];
    match job.stage {
        Stage::Lookup => {
            let mut reply = Reply::new(io);
            let found = reply.value();
            let (blob, rev) = match found.get("blob") {
                Value::Blob { obj, .. } => (*obj, found.get("rev").text()),
                _ => (0, String::new()),
            };
            let stale = !job.revision.is_empty() && blob != 0 && rev != job.revision;
            let offline = shell::offline(app);
            let job = &mut app.st.drive_home.jobs[at];
            job.stored = blob;
            if (blob == 0 || stale) && !job.source.is_empty() && !offline {
                job.stage = Stage::Fetch;
                let (req, url) = (job.req, job.source.clone());
                fx::fetch(host.out(), req, &Fetch { url: &url, flags: fetch_flag::BLOB | fetch_flag::OMIT_CREDENTIALS, deadline_ms: 6000.0, ..Fetch::default() });
            } else if blob != 0 {
                to_url(app, host, at, blob, false);
            } else {
                let source = job.source.clone();
                finish(app, host, at, (!source.is_empty()).then_some(source));
            }
        }
        Stage::Fetch => {
            if (200..300).contains(&io.status) && io.obj != 0 {
                let response = Response::parse(io.data);
                let job = &mut app.st.drive_home.jobs[at];
                job.size = response.blob_size();
                job.mime = response.header("content-type").unwrap_or("").to_string();
                to_url(app, host, at, io.obj, true);
            } else {
                host.out().obj_release(io.obj);
                // The previous copy when there is one; else the link itself (it may still display without CORS).
                let (stored, source) = (job.stored, job.source.clone());
                if stored != 0 {
                    to_url(app, host, at, stored, false);
                } else {
                    finish(app, host, at, Some(source));
                }
            }
        }
        Stage::Url { fresh } => {
            let url = (io.status >= 0).then(|| String::from_utf8_lossy(io.data).into_owned()).filter(|u| !u.is_empty());
            if fresh && url.is_some() {
                let mut tx = Tx::new();
                tx.put_with(Store::Catalog, Put::Replace, Key::Pair(&thumbs(&job.account), &job.id), |w| {
                    w.obj(2);
                    w.key("blob");
                    w.blob(job.blob, job.size, &job.mime);
                    w.key("rev");
                    w.str(&job.revision);
                });
                let done = app.req(Mod::DriveHome);
                store::tx(app, host, done, tx);
            }
            let fallback = app.st.drive_home.jobs[at].source.clone();
            finish(app, host, at, url.or(Some(fallback)).filter(|u| !u.is_empty()));
        }
    }
}

// ---- the full enumeration ---------------------------------------------------------------------------------------

/// Lists every PDF and folder of the account once, so folders never opened are browsable offline and entries
/// Drive no longer has disappear from the cache. A second call while one runs is ignored.
// v5: offline-library-v5.js:catalogue
pub fn crawl(app: &mut App, host: &mut dyn Host, account: &str) {
    if app.st.drive_home.crawl.is_some() || account.is_empty() || shell::offline(app) {
        return;
    }
    let req = client::request(app, host, Mod::DriveHome, Req::get(&format!("{FILES}/root?fields=id")));
    app.st.drive_home.crawl = Some(Crawl { account: account.into(), req, root: String::new(), found: Vec::new() });
}

fn crawl_page(app: &mut App, host: &mut dyn Host, token: &str) {
    let q = format!("trashed=false and (mimeType='{PDF}' or mimeType='{FOLDER}')");
    let mut pairs = vec![("q", q.as_str()), ("pageSize", "1000"), ("fields", CRAWL_FIELDS)];
    if !token.is_empty() {
        pairs.push(("pageToken", token));
    }
    let req = client::request(app, host, Mod::DriveHome, Req::get(&format!("{FILES}?{}", fx::form(&pairs))));
    if let Some(crawl) = &mut app.st.drive_home.crawl {
        crawl.req = req;
    }
}

fn crawl_io(app: &mut App, host: &mut dyn Host, io: &crate::Io) {
    let account = crate::home::account(app).to_string();
    let Some(crawl) = &mut app.st.drive_home.crawl else { return };
    let body = Response::parse(io.data).text().to_string();
    if !(200..300).contains(&io.status) || crawl.account != account {
        app.st.drive_home.crawl = None;
        return;
    }
    if crawl.root.is_empty() {
        let root = Json::parse(&body).map_or(String::new(), |j| j.text("id").to_string());
        if root.is_empty() {
            app.st.drive_home.crawl = None;
            return;
        }
        crawl.root = root.clone();
        crate::home::root_known(app, host, &root);
        return crawl_page(app, host, "");
    }
    let (mut files, next) = parse_files(&body);
    for file in files.iter_mut() {
        crawl.found.push(file.id.clone());
        // v5: `item.parents?.length ? item.parents.map(id => id === rootId ? 'root' : id) : ['root']`
        let parents: Vec<String> = file.parents.take().unwrap_or_default().into_iter().map(|p| if p == crawl.root { "root".into() } else { p }).collect();
        file.parents = Some(if parents.is_empty() { vec!["root".into()] } else { parents });
    }
    crate::home::crawled(app, host, files, None);
    if !next.is_empty() {
        return crawl_page(app, host, &next);
    }
    // Only a complete successful enumeration can mark an item removed.
    if let Some(mut crawl) = app.st.drive_home.crawl.take() {
        crawl.found.sort_unstable();
        crate::home::crawled(app, host, Vec::new(), Some(crawl.found));
    }
}

pub fn init(_app: &mut App, _host: &mut dyn Host) {}

pub fn event(app: &mut App, host: &mut dyn Host, event: &Event) -> u32 {
    match *event {
        Event::Io(io) => {
            if let Some(at) = app.st.drive_home.jobs.iter().position(|j| j.req == io.req) {
                media_io(app, host, at, &io);
            } else if app.st.drive_home.crawl.as_ref().is_some_and(|c| c.req == io.req) {
                crawl_io(app, host, &io);
            }
        }
        Event::Timer(T_REVOKE) => {
            let now = host.now();
            let st = &mut app.st.drive_home;
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut st.retired).into_iter().partition(|(at, _)| *at <= now);
            st.retired = later;
            for (_, url) in due {
                fx::url_revoke(host.out(), &url);
            }
            if let Some(next) = app.st.drive_home.retired.iter().map(|(at, _)| *at).reduce(f64::min) {
                app.sched.timer(host.out(), T_REVOKE, (next - now).max(1.0));
            }
        }
        _ => {}
    }
    0
}

pub fn test_cmd(app: &mut App, _host: &mut dyn Host, name: &str, args: &str) -> Option<String> {
    let arg = Json::parse(args).unwrap_or(Json::Null);
    Some(match name {
        "url" => {
            let list = match arg.text("list") {
                "recents" => List::Recents,
                "starred" => List::Starred,
                "shared" => List::Shared,
                "folders" => List::Folders,
                _ => List::Files,
            };
            crate::testapi::json_str(&list_url(list, arg.text("search"), if arg.text("folder").is_empty() { "root" } else { arg.text("folder") }))
        }
        "thumbnail" => crate::testapi::json_str(&normalize_thumbnail(arg.as_str().unwrap_or(""))),
        "state" => format!("{{\"media\":{},\"jobs\":{},\"crawling\":{}}}", app.st.drive_home.memory.len(), app.st.drive_home.jobs.len(), app.st.drive_home.crawl.is_some()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(url: &str) -> String {
        let mut out = Vec::new();
        let b = url.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                out.push(u8::from_str_radix(&url[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn list_urls_are_the_5_18_requests_plus_app_properties() {
        assert_eq!(
            decoded(&list_url(List::Recents, "", "root")),
            "https://www.googleapis.com/drive/v3/files?q=mimeType='application/pdf' and trashed=false&orderBy=modifiedTime desc&pageSize=12&fields=files(id,name,mimeType,modifiedTime,createdTime,thumbnailLink,iconLink,parents,starred,ownedByMe,resourceKey,appProperties)"
        );
        assert!(list_url(List::Recents, "", "root").starts_with("https://www.googleapis.com/drive/v3/files?q=mimeType%3D'application%2Fpdf'%20and%20trashed%3Dfalse&orderBy=modifiedTime desc&pageSize=12&fields=files(id%2Cname"));
        assert_eq!(
            decoded(&list_url(List::Starred, "ignored", "root")),
            "https://www.googleapis.com/drive/v3/files?q=starred=true and trashed=false and (mimeType='application/pdf' or mimeType='application/vnd.google-apps.folder')&orderBy=modifiedTime desc&pageSize=12&fields=files(id,name,modifiedTime,createdTime,thumbnailLink,iconLink,mimeType,parents,starred,ownedByMe,resourceKey,folderColorRgb,appProperties)"
        );
        assert!(decoded(&list_url(List::Shared, "it's", "root")).contains("q=(mimeType='application/pdf' or mimeType='application/vnd.google-apps.folder') and sharedWithMe=true and trashed=false and name contains 'it\\'s'&orderBy=modifiedTime desc&pageSize=24&"));
        assert!(decoded(&list_url(List::Folders, "a", "F1")).contains("q='F1' in parents and trashed=false and name contains 'a' and mimeType='application/vnd.google-apps.folder'&orderBy=name&pageSize=100&fields=files(id,name,mimeType,starred,ownedByMe,folderColorRgb,appProperties)"));
        assert!(decoded(&list_url(List::Files, "", "root")).contains("q='root' in parents and trashed=false and mimeType='application/pdf'&orderBy=modifiedTime desc&pageSize=100&"));
    }

    #[test]
    fn files_parse_into_entries_and_round_trip_through_the_store_form() {
        let (files, next) = parse_files(r##"{"nextPageToken":"n2","files":[{"id":"a","name":"A.pdf","mimeType":"application/pdf","modifiedTime":"2026-10-20T08:00:00.000Z","thumbnailLink":"https://lh3.googleusercontent.com/x=s220","parents":["r"],"starred":true,"ownedByMe":false,"resourceKey":"k"},{"id":"s","name":"relay","mimeType":"application/pdf","starred":false,"appProperties":{"ihn6Kind":"relay"}},{"id":"f","name":"F","mimeType":"application/vnd.google-apps.folder","folderColorRgb":"#4986e7","starred":false,"ownedByMe":true}]}"##);
        assert_eq!(next, "n2");
        assert_eq!((files[0].id.as_str(), files[0].starred, files[0].owned_by_me, files[0].parents.clone()), ("a", true, Some(false), Some(vec!["r".to_string()])));
        assert_eq!((files[1].kind.as_str(), files[1].owned_by_me, files[1].parents.clone()), ("relay", None, None));
        assert!(files[2].is_folder());
        for file in &files {
            assert_eq!(&entry_from_value(&entry_value(file)), file);
        }
        let local = Entry { pending: true, trashed: true, local_preview: "data:image/png;base64,AA".into(), ..files[0].clone() };
        assert_eq!(entry_from_value(&entry_value(&local)), local);
        assert_eq!(parse_files("not json"), (Vec::new(), String::new()));
        assert_eq!(parse_files("{}"), (Vec::new(), String::new()));
    }

    #[test]
    fn thumbnails_are_resized_like_5_18() {
        assert_eq!(normalize_thumbnail("https://lh3.googleusercontent.com/drive-storage/x=s220"), "https://lh3.googleusercontent.com/drive-storage/x=s440");
        assert_eq!(normalize_thumbnail("https://x/y?a=sb&b=s12&c=s9"), "https://x/y?a=sb&b=s440&c=s9");
        assert_eq!(normalize_thumbnail("https://drive.google.com/thumbnail?id=1"), "https://drive.google.com/thumbnail?id=1&sz=w312-h440");
        assert_eq!(normalize_thumbnail("https://x/y?k=stay"), "https://x/y?k=stay");
        for same in ["", "data:image/png;base64,AA", "blob:http://x/1", "/local.png"] {
            assert_eq!(normalize_thumbnail(same), same);
        }
    }
}
