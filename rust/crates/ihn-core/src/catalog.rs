//! Home catalogue: the Drive entries cached for an account, merged with what Drive lists, ordered and cut into
//! Home's views (ARCHITECTURE §13). Sync files never leave this module. Implemented by W2.5 (BUILD-PLAN.md).
// v5: offline-library-v5.js (compareEntries, folder, combine, home, recentItems, observed), offline-store-v5.js:cacheEntries

use std::cmp::Ordering;

pub const FOLDER: &str = "application/vnd.google-apps.folder";
pub const PDF: &str = "application/pdf";

/// A Drive file or folder as Home lists it. Empty text = the field was not in the listing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub modified: String,
    pub created: String,
    pub thumbnail: String,
    pub icon: String,
    pub parents: Option<Vec<String>>,
    pub starred: bool,
    /// `None`: Drive did not say (`ownedByMe === false` is what makes a card "shared").
    pub owned_by_me: Option<bool>,
    pub resource_key: String,
    pub folder_color: String,
    pub trashed: bool,
    /// Created on this device and not on Drive yet (`docs.flags.created == false`).
    pub pending: bool,
    /// Data URL of page 1 written by this device.
    pub local_preview: String,
    /// `appProperties.ihn6Kind`: set on sync files, which Home never shows.
    pub kind: String,
}

impl Entry {
    pub fn is_folder(&self) -> bool { self.mime == FOLDER }
    pub fn shared(&self) -> bool { self.owned_by_me == Some(false) }
    /// `Date.parse(modifiedTime) || 0`.
    pub fn modified_ms(&self) -> f64 { parse_time(&self.modified).unwrap_or(0.0) }
    /// The fields a card draws; `modifiedTime` is deliberately not one of them.
    // v5: app-v5.js:driveItemFingerprint
    pub fn fingerprint(&self) -> String {
        format!("{}|{}|{}|{}|{}", self.id, self.name, self.starred as u8, self.folder_color, self.mime)
    }
    /// `{...previous, ...entry}` where `entry` carries only the fields its listing asked for; a newer
    /// `modifiedTime` drops the local preview.
    // v5: offline-store-v5.js:cacheEntries
    pub fn merged(previous: Option<&Entry>, new: &Entry) -> Entry {
        let Some(old) = previous else { return new.clone() };
        let pick = |a: &String, b: &String| if a.is_empty() { b.clone() } else { a.clone() };
        let newer = !old.pending && parse_time(&new.modified).is_some_and(|t| parse_time(&old.modified).is_none_or(|o| t > o));
        Entry {
            id: new.id.clone(),
            name: pick(&new.name, &old.name),
            mime: pick(&new.mime, &old.mime),
            modified: pick(&new.modified, &old.modified),
            created: pick(&new.created, &old.created),
            thumbnail: pick(&new.thumbnail, &old.thumbnail),
            icon: pick(&new.icon, &old.icon),
            parents: new.parents.clone().or_else(|| old.parents.clone()),
            starred: new.starred,
            owned_by_me: new.owned_by_me.or(old.owned_by_me),
            resource_key: pick(&new.resource_key, &old.resource_key),
            folder_color: pick(&new.folder_color, &old.folder_color),
            trashed: new.trashed,
            pending: new.pending || old.pending,
            local_preview: if newer { String::new() } else { pick(&new.local_preview, &old.local_preview) },
            kind: pick(&new.kind, &old.kind),
        }
    }
    /// Every field the listing carried is already what is stored.
    fn same_as_stored(&self, stored: &Entry) -> bool { Entry::merged(Some(stored), self) == *stored }
}

/// `Date.parse` of an RFC 3339 time (`2026-10-20T08:00:00.000Z`, offsets allowed), in ms.
pub fn parse_time(text: &str) -> Option<f64> {
    let b = text.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let s = text.get(from..from + len)?;
        s.bytes().all(|c| c.is_ascii_digit()).then(|| s.parse().ok()).flatten()
    };
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from 1970-01-01 (civil calendar).
    let (yy, mm) = (if mo <= 2 { y - 1 } else { y }, if mo <= 2 { mo + 9 } else { mo - 3 });
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    let mut ms = days as f64 * 86_400_000.0;
    if b.len() == 10 {
        return Some(ms);
    }
    if !matches!(b[10], b'T' | b't' | b' ') || b.len() < 16 || b[13] != b':' {
        return None;
    }
    ms += (num(11, 2)? * 3_600_000 + num(14, 2)? * 60_000) as f64;
    let mut i = 16;
    if b.get(i) == Some(&b':') {
        ms += (num(i + 1, 2)? * 1000) as f64;
        i += 3;
        if b.get(i) == Some(&b'.') {
            let digits = text[i + 1..].bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                return None;
            }
            let frac: f64 = format!("0.{}", &text[i + 1..i + 1 + digits]).parse().ok()?;
            ms += (frac * 1000.0).floor();
            i += 1 + digits;
        }
    }
    match b.get(i) {
        None => Some(ms), // local time in JS; Drive always sends a zone
        Some(b'Z') | Some(b'z') if i + 1 == b.len() => Some(ms),
        Some(sign @ (b'+' | b'-')) if b.len() == i + 6 && b[i + 3] == b':' => {
            let offset = (num(i + 1, 2)? * 60 + num(i + 4, 2)?) as f64 * 60_000.0;
            Some(if *sign == b'+' { ms - offset } else { ms + offset })
        }
        _ => None,
    }
}

/// What Home needs to know about a locally stored document (`docs` store).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DocInfo {
    pub last_opened_at: f64,
    pub pinned: bool,
    /// The document has chunks on this device.
    pub available: bool,
    /// `flags.relay_pending || !flags.created`: own changes Drive does not have yet.
    pub pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Recents,
    Starred,
    Shared,
}

impl View {
    pub fn name(self) -> &'static str {
        match self {
            View::Recents => "recents",
            View::Starred => "starred",
            View::Shared => "shared",
        }
    }
}

/// The last Drive order per Home row (`kv` `home-views`); `None` = never listed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Views {
    pub recents: Option<Vec<String>>,
    pub starred: Option<Vec<String>>,
    pub shared: Option<Vec<String>>,
}

impl Views {
    pub fn get(&self, view: View) -> &Option<Vec<String>> {
        match view {
            View::Recents => &self.recents,
            View::Starred => &self.starred,
            View::Shared => &self.shared,
        }
    }
    fn set(&mut self, view: View, ids: Vec<String>) {
        *match view {
            View::Recents => &mut self.recents,
            View::Starred => &mut self.starred,
            View::Shared => &mut self.shared,
        } = Some(ids);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Home {
    pub recents: Vec<Entry>,
    pub starred: Vec<Entry>,
    pub shared: Vec<Entry>,
}

/// What [`Catalog::observe`] asks the caller to persist.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observed {
    /// The merged entries (`catalog` store).
    pub entries: Vec<Entry>,
    /// The view whose order changed, to write `home-views`.
    pub views: bool,
}

/// `name.localeCompare(other)`, supplied by the host.
pub type NameOrder<'a> = &'a mut dyn FnMut(&str, &str) -> Ordering;
/// `name.toLowerCase().includes(needle)` for the search box, supplied by the host (`None`: no search).
pub type NameFilter<'a> = Option<&'a mut dyn FnMut(&str) -> bool>;

// One stable merge sort for every list (the std sorts cost a copy of their code per call site).
fn sort_indices(order: &mut [u32], scratch: &mut Vec<u32>, cmp: &mut dyn FnMut(u32, u32) -> Ordering) {
    let n = order.len();
    if n < 2 {
        return;
    }
    let (left, right) = order.split_at_mut(n / 2);
    sort_indices(left, scratch, cmp);
    sort_indices(right, scratch, cmp);
    scratch.clear();
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        if cmp(right[j], left[i]) == Ordering::Less {
            scratch.push(right[j]);
            j += 1;
        } else {
            scratch.push(left[i]);
            i += 1;
        }
    }
    scratch.extend_from_slice(&left[i..]);
    scratch.extend_from_slice(&right[j..]);
    order.copy_from_slice(scratch);
}

/// `list.sort(cmp)` (stable).
pub fn sort_entries(list: &mut Vec<Entry>, cmp: &mut dyn FnMut(&Entry, &Entry) -> Ordering) {
    let mut order: Vec<u32> = (0..list.len() as u32).collect();
    sort_indices(&mut order, &mut Vec::new(), &mut |a, b| cmp(&list[a as usize], &list[b as usize]));
    let mut items: Vec<Option<Entry>> = std::mem::take(list).into_iter().map(Some).collect();
    list.extend(order.into_iter().filter_map(|i| items[i as usize].take()));
}

fn by_id(a: &Entry, b: &Entry) -> Ordering { a.id.cmp(&b.id) }

#[derive(Default)]
pub struct Catalog {
    /// As stored (raw parents), trashed ones included, sorted by id.
    stored: Vec<Entry>,
    /// Active (not trashed, not hidden) entries with resolved parents, sorted by id.
    active: Vec<Entry>,
    /// `(parent, index into active)`, sorted.
    by_parent: Vec<(String, u32)>,
    /// Sorted by id.
    docs: Vec<(String, DocInfo)>,
    pub views: Views,
    /// `kv.relay-folder`: the sync folder, never listed.
    sync_folder: String,
    /// The Drive id of "My Drive" (`kv.drive-root`), shown as `root`.
    root_id: String,
    pub loaded: bool,
}

impl Catalog {
    // v5: offline-library-v5.js:reload
    pub fn load(&mut self, mut entries: Vec<Entry>, mut docs: Vec<(String, DocInfo)>, views: Views, sync_folder: String, root_id: String) {
        sort_entries(&mut entries, &mut by_id);
        docs.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        self.stored = entries;
        self.docs = docs;
        self.views = views;
        self.sync_folder = sync_folder;
        self.root_id = root_id;
        self.loaded = true;
        self.index();
    }

    // v5: offline-library-v5.js:logout
    pub fn clear(&mut self) { *self = Catalog::default(); }

    pub fn set_root(&mut self, id: &str) {
        if self.root_id != id {
            self.root_id = id.to_string();
            self.index();
        }
    }
    pub fn set_sync_folder(&mut self, id: &str) {
        if self.sync_folder != id {
            self.sync_folder = id.to_string();
            self.index();
        }
    }
    pub fn root_id(&self) -> &str { &self.root_id }
    pub fn len(&self) -> usize { self.stored.len() }
    pub fn is_empty(&self) -> bool { self.stored.is_empty() }

    /// A sync file or the sync folder: never listed, searched or shown offline.
    pub fn hidden(&self, e: &Entry) -> bool {
        !e.kind.is_empty() || (!self.sync_folder.is_empty() && (e.id == self.sync_folder || e.parents.as_ref().is_some_and(|p| p.contains(&self.sync_folder))))
    }
    /// A Drive listing without sync files.
    pub fn visible(&self, mut list: Vec<Entry>) -> Vec<Entry> {
        list.retain(|e| !e.id.is_empty() && !self.hidden(e));
        list
    }

    // v5: offline-library-v5.js:indexCatalogue
    fn index(&mut self) {
        let mut active: Vec<Entry> = self.stored.iter().filter(|e| !e.trashed && !self.hidden(e)).cloned().collect();
        self.by_parent.clear();
        for (i, e) in active.iter_mut().enumerate() {
            let mut parents: Vec<String> = e.parents.take().unwrap_or_else(|| vec!["root".into()]);
            for p in parents.iter_mut() {
                if !self.root_id.is_empty() && *p == self.root_id {
                    *p = "root".into();
                }
            }
            for (k, p) in parents.iter().enumerate() {
                if !parents[..k].contains(p) {
                    self.by_parent.push((p.clone(), i as u32));
                }
            }
            e.parents = Some(parents);
        }
        self.by_parent.sort_unstable();
        self.active = active;
    }

    fn stored_at(&self, id: &str) -> Result<usize, usize> { self.stored.binary_search_by(|e| e.id.as_str().cmp(id)) }
    pub fn entry(&self, id: &str) -> Option<&Entry> { self.active.binary_search_by(|e| e.id.as_str().cmp(id)).ok().map(|i| &self.active[i]) }
    pub fn doc(&self, id: &str) -> Option<&DocInfo> { self.docs.binary_search_by(|d| d.0.as_str().cmp(id)).ok().map(|i| &self.docs[i].1) }
    pub fn set_doc(&mut self, id: &str, info: Option<DocInfo>) {
        match (self.docs.binary_search_by(|d| d.0.as_str().cmp(id)), info) {
            (Ok(i), Some(info)) => self.docs[i].1 = info,
            (Ok(i), None) => drop(self.docs.remove(i)),
            (Err(i), Some(info)) => self.docs.insert(i, (id.to_string(), info)),
            (Err(_), None) => {}
        }
    }
    /// Documents with changes Drive does not have (the queue line of the Offline documents dialog).
    pub fn pending_count(&self) -> usize { self.docs.iter().filter(|d| d.1.pending).count() }
    pub fn is_folder(&self, id: &str) -> bool { self.stored_at(id).is_ok_and(|i| self.stored[i].is_folder()) }

    /// Folders first by name, then files newest first; ties by id.
    // v5: offline-library-v5.js:compareEntries
    pub fn compare_entries(a: &Entry, b: &Entry, names: NameOrder) -> Ordering {
        let (af, bf) = (a.is_folder(), b.is_folder());
        if af != bf {
            return if af { Ordering::Less } else { Ordering::Greater };
        }
        let order = if af { names(&a.name, &b.name) } else { b.modified_ms().partial_cmp(&a.modified_ms()).unwrap_or(Ordering::Equal) };
        order.then_with(|| js_cmp(&a.id, &b.id))
    }

    /// Most recently opened on this device first, then newest, then id.
    // v5: offline-library-v5.js:compareRecents
    pub fn compare_recents(&self, a: &Entry, b: &Entry) -> Ordering {
        let opened = |e: &Entry| self.doc(&e.id).map_or(0.0, |d| d.last_opened_at);
        let by = |x: f64, y: f64| y.partial_cmp(&x).unwrap_or(Ordering::Equal);
        by(opened(a), opened(b)).then_with(|| by(a.modified_ms(), b.modified_ms())).then_with(|| js_cmp(&a.id, &b.id))
    }

    /// The cached directory `id` (`root` for My Drive), filtered by the search box.
    // v5: offline-library-v5.js:folder
    pub fn folder(&self, id: &str, mut filter: NameFilter, names: NameOrder) -> Vec<Entry> {
        let key = if id.is_empty() { "root" } else { id };
        let from = self.by_parent.partition_point(|p| p.0.as_str() < key);
        let mut list = Vec::new();
        for (_, i) in self.by_parent[from..].iter().take_while(|p| p.0 == key) {
            let e = &self.active[*i as usize];
            if filter.as_mut().is_none_or(|f| f(&e.name)) {
                list.push(e.clone());
            }
        }
        sort_entries(&mut list, &mut |a, b| Self::compare_entries(a, b, names));
        list
    }

    /// A Drive listing of `parent` with this device's not-yet-uploaded entries in front of it.
    // v5: offline-library-v5.js:combine
    pub fn combine(&self, remote: Vec<Entry>, parent: &str, filter: NameFilter, names: NameOrder) -> Vec<Entry> {
        let mut list = self.folder(parent, filter, names);
        list.retain(|e| e.pending);
        let local = list.len();
        for e in self.visible(remote) {
            if !list[..local].iter().any(|l| l.id == e.id) {
                list.push(e);
            }
        }
        sort_entries(&mut list, &mut |a, b| Self::compare_entries(a, b, names));
        list
    }

    /// Recents: the given list plus every pending PDF, most recently opened first, 12 at most.
    // v5: offline-library-v5.js:recentItems
    pub fn recent_items(&self, remote: Vec<Entry>) -> Vec<Entry> {
        let mut list: Vec<Entry> = Vec::new();
        let pending = self.active.iter().filter(|e| e.mime == PDF && e.pending).cloned();
        for e in self.visible(remote).into_iter().chain(pending) {
            // `byId.set(id, entry)`: a later entry replaces an earlier one in its place.
            match list.iter().position(|l| l.id == e.id) {
                Some(i) => list[i] = e,
                None => list.push(e),
            }
        }
        sort_entries(&mut list, &mut |a, b| self.compare_recents(a, b));
        list.truncate(12);
        list
    }

    fn ordered(&self, keep: &dyn Fn(&Entry) -> bool, n: usize) -> Vec<Entry> {
        let mut list: Vec<Entry> = self.active.iter().filter(|e| keep(e)).cloned().collect();
        sort_entries(&mut list, &mut |a, b| self.compare_recents(a, b));
        list.truncate(n);
        list
    }

    fn view(&self, v: View, keep: &dyn Fn(&Entry) -> bool, n: usize) -> Vec<Entry> {
        match self.views.get(v) {
            Some(ids) => ids.iter().filter_map(|id| self.entry(id).cloned()).collect(),
            None => self.ordered(keep, n),
        }
    }

    /// Home's three rows from the cache: the saved Drive order when there is one, else what the entries say.
    // v5: offline-library-v5.js:home
    pub fn home(&self) -> Home {
        Home {
            recents: self.recent_items(self.view(View::Recents, &|e| e.mime == PDF, 12)),
            starred: self.view(View::Starred, &|e| e.starred, 12),
            shared: self.view(View::Shared, &Entry::shared, 24),
        }
    }

    fn store(&mut self, e: Entry) {
        match self.stored_at(&e.id) {
            Ok(i) => self.stored[i] = e,
            Err(i) => self.stored.insert(i, e),
        }
    }

    /// Records a Drive listing (`items` already without sync files): `None` when everything is already cached
    /// in this order; otherwise the merged entries to persist. `parent` fills the parents of items whose
    /// listing did not ask for them.
    // v5: offline-library-v5.js:observed / alreadyObserved
    pub fn observe(&mut self, items: &[Entry], parent: Option<&str>, view: Option<View>) -> Option<Observed> {
        let ids: Vec<String> = items.iter().map(|e| e.id.clone()).collect();
        let views = view.is_some_and(|v| self.views.get(v).as_ref() != Some(&ids));
        let mut entries = Vec::new();
        let mut changed = !self.loaded || views;
        for item in items {
            let mut e = item.clone();
            if e.parents.is_none() {
                e.parents = parent.map(|p| vec![p.to_string()]);
            }
            let stored = self.stored_at(&e.id).ok().map(|i| &self.stored[i]);
            changed |= !stored.is_some_and(|s| e.same_as_stored(s));
            entries.push(Entry::merged(stored, &e));
        }
        if !changed {
            return None;
        }
        for e in &entries {
            self.store(e.clone());
        }
        if let Some(v) = view {
            self.views.set(v, ids);
        }
        self.index();
        Some(Observed { entries, views })
    }

    /// After a complete enumeration of Drive (`found`: its ids, sorted): cached entries it did not contain are
    /// marked removed.
    // v5: offline-library-v5.js:catalogue
    pub fn mark_missing(&mut self, found: &[String]) -> Vec<Entry> {
        let mut changed = Vec::new();
        for e in self.stored.iter_mut() {
            if !e.pending && !e.trashed && found.binary_search(&e.id).is_err() {
                e.trashed = true;
                changed.push(e.clone());
            }
        }
        if !changed.is_empty() {
            self.index();
        }
        changed
    }
}

/// `a < b ? -1 : a > b ? 1 : 0` on JS strings (UTF-16 code units).
fn js_cmp(a: &str, b: &str) -> Ordering { a.encode_utf16().cmp(b.encode_utf16()) }

/// Test command hook: `core.catalog.parse_time` with a JSON string → ms or `null`.
pub fn test_cmd(name: &str, args: &str) -> Option<String> {
    match name {
        "parse_time" => Some(parse_time(args.trim_matches('"')).map_or("null".into(), crate::jsfmt::num)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(a: &str, b: &str) -> Ordering { a.to_lowercase().cmp(&b.to_lowercase()) }
    fn file(id: &str, name: &str, day: u32) -> Entry {
        Entry { id: id.into(), name: name.into(), mime: PDF.into(), modified: format!("2026-10-{day:02}T08:00:00.000Z"), parents: Some(vec!["root-id".into()]), owned_by_me: Some(true), ..Default::default() }
    }
    fn folder(id: &str, name: &str) -> Entry { Entry { id: id.into(), name: name.into(), mime: FOLDER.into(), owned_by_me: Some(true), ..Default::default() } }
    fn ids(list: &[Entry]) -> Vec<&str> { list.iter().map(|e| e.id.as_str()).collect() }
    fn loaded(entries: Vec<Entry>) -> Catalog {
        let mut c = Catalog::default();
        c.load(entries, Vec::new(), Views::default(), "sync".into(), "root-id".into());
        c
    }

    #[test]
    fn times_parse_like_date_parse() {
        assert_eq!(parse_time("1970-01-01T00:00:00.000Z"), Some(0.0));
        assert_eq!(parse_time("2026-10-30T10:00:00Z"), Some(1_793_354_400_000.0));
        assert_eq!(parse_time("2026-10-30T10:00:00.5Z"), Some(1_793_354_400_500.0));
        assert_eq!(parse_time("2026-10-30T12:00:00+02:00"), Some(1_793_354_400_000.0));
        assert_eq!(parse_time("2024-02-29T00:00:00Z"), Some(1_709_164_800_000.0));
        assert_eq!(parse_time("2026-10-30"), Some(1_793_318_400_000.0));
        assert_eq!(parse_time(""), None);
        assert_eq!(parse_time("yesterday"), None);
        assert_eq!(Entry::default().modified_ms(), 0.0);
    }

    #[test]
    fn a_folder_lists_folders_by_name_then_files_newest_first() {
        let c = loaded(vec![
            file("b", "Beta.pdf", 20), file("a", "Alpha.pdf", 22), file("c", "Gamma.pdf", 22),
            Entry { parents: None, ..folder("f2", "zeta") }, Entry { parents: Some(vec!["root".into()]), ..folder("f1", "Alpha") },
            Entry { parents: Some(vec!["f1".into()]), ..file("n", "Nested.pdf", 25) },
            Entry { trashed: true, ..file("t", "Binned.pdf", 30) },
        ]);
        assert_eq!(ids(&c.folder("root", None, &mut names)), ["f1", "f2", "a", "c", "b"]);
        assert_eq!(ids(&c.folder("", None, &mut names)), ["f1", "f2", "a", "c", "b"]);
        assert_eq!(ids(&c.folder("f1", None, &mut names)), ["n"]);
        assert_eq!(ids(&c.folder("root", Some(&mut |n: &str| n.to_lowercase().contains("alp")), &mut names)), ["f1", "a"]);
        assert!(c.folder("missing", None, &mut names).is_empty());
    }

    #[test]
    fn sync_files_are_never_listed() {
        let mut c = loaded(vec![
            file("a", "A.pdf", 20), folder("sync", "Inhouse Notes sync"),
            Entry { parents: Some(vec!["sync".into()]), ..file("r", "relay", 21) },
            Entry { kind: "relay".into(), ..file("k", "tagged", 22) },
        ]);
        assert_eq!(ids(&c.folder("root", None, &mut names)), ["a"]);
        assert!(c.folder("sync", None, &mut names).is_empty());
        assert_eq!(ids(&c.home().recents), ["a"]);
        let listing = vec![file("a", "A.pdf", 20), folder("sync", "x"), Entry { kind: "archive".into(), ..file("z", "z", 1) }, Entry::default()];
        assert_eq!(ids(&c.visible(listing.clone())), ["a"]);
        assert_eq!(ids(&c.combine(listing.clone(), "root", None, &mut names)), ["a"]);
        assert_eq!(ids(&c.recent_items(listing)), ["a"]);
        // The folder id may become known later.
        c.set_sync_folder("f9");
        assert!(c.hidden(&folder("f9", "x")) && !c.hidden(&folder("sync", "x")));
    }

    #[test]
    fn recents_put_the_last_opened_first_and_keep_twelve() {
        let mut entries: Vec<Entry> = (1..=14).map(|d| file(&format!("d{d:02}"), "x", d)).collect();
        entries.push(Entry { pending: true, modified: String::new(), ..file("new", "New.pdf", 1) });
        let mut c = loaded(entries.clone());
        c.set_doc("d03", Some(DocInfo { last_opened_at: 5.0, ..Default::default() }));
        c.set_doc("d01", Some(DocInfo { last_opened_at: 9.0, ..Default::default() }));
        let remote: Vec<Entry> = entries[4..14].iter().rev().cloned().collect();
        let list = c.recent_items(remote);
        assert_eq!(list.len(), 11);
        assert_eq!(ids(&list)[..3], ["d14", "d13", "d12"]);
        assert_eq!(*ids(&list).last().unwrap(), "new");
        let home = c.home();
        assert_eq!(home.recents.len(), 12);
        assert_eq!(ids(&home.recents)[..4], ["d01", "d03", "d14", "d13"]);
    }

    #[test]
    fn saved_views_keep_drives_order_and_fall_back_to_the_entries() {
        let mut c = loaded(vec![
            Entry { starred: true, ..file("a", "A.pdf", 20) }, file("b", "B.pdf", 21),
            Entry { owned_by_me: Some(false), ..file("s", "Shared.pdf", 22) }, Entry { starred: true, ..folder("f", "Folder") },
        ]);
        let home = c.home();
        assert_eq!(ids(&home.starred), ["a", "f"]);
        assert_eq!(ids(&home.shared), ["s"]);
        assert_eq!(ids(&home.recents), ["s", "b", "a"]);
        c.views = Views { recents: Some(vec!["b".into(), "gone".into()]), starred: Some(vec!["f".into(), "a".into()]), shared: Some(vec![]) };
        let home = c.home();
        assert_eq!(ids(&home.starred), ["f", "a"]);
        assert!(home.shared.is_empty());
        assert_eq!(ids(&home.recents), ["b"]);
    }

    #[test]
    fn observing_merges_field_by_field_and_reports_only_changes() {
        let mut c = loaded(vec![Entry { local_preview: "data:x".into(), thumbnail: "t1".into(), ..file("a", "A.pdf", 20) }]);
        // The folder listing carries no parents and no times.
        let listed = Entry { id: "f".into(), name: "Folder".into(), mime: FOLDER.into(), owned_by_me: Some(true), ..Default::default() };
        let seen = c.observe(&[listed.clone()], Some("root"), None).unwrap();
        assert_eq!(seen.entries[0].parents, Some(vec!["root".to_string()]));
        assert!(!seen.views);
        assert_eq!(c.observe(&[listed], Some("root"), None), None);
        // Same data, new view order: only the view is written.
        let a = Entry { thumbnail: String::new(), ..file("a", "A.pdf", 20) };
        let seen = c.observe(&[a.clone()], None, Some(View::Recents)).unwrap();
        assert!(seen.views);
        assert_eq!((seen.entries[0].thumbnail.as_str(), seen.entries[0].local_preview.as_str()), ("t1", "data:x"));
        assert_eq!(c.observe(&[a], None, Some(View::Recents)), None);
        // A newer modifiedTime drops the local preview; starring is taken as listed.
        let newer = Entry { starred: true, ..file("a", "A2.pdf", 21) };
        let seen = c.observe(&[newer], None, Some(View::Recents)).unwrap();
        assert_eq!((seen.entries[0].name.as_str(), seen.entries[0].local_preview.as_str(), seen.entries[0].starred, seen.views), ("A2.pdf", "", true, false));
        assert_eq!(ids(&c.folder("root", None, &mut names)), ["f", "a"]);
        assert_eq!(c.views.recents, Some(vec!["a".to_string()]));
    }

    #[test]
    fn a_complete_enumeration_marks_what_drive_no_longer_has() {
        let mut c = loaded(vec![file("a", "A.pdf", 20), file("b", "B.pdf", 21), Entry { pending: true, ..file("p", "P.pdf", 22) }]);
        let gone = c.mark_missing(&["a".to_string()]);
        assert_eq!(ids(&gone), ["b"]);
        assert!(gone[0].trashed);
        assert_eq!(ids(&c.folder("root", None, &mut names)), ["p", "a"]);
        assert!(c.mark_missing(&["a".to_string()]).is_empty());
    }

    #[test]
    fn the_root_alias_can_arrive_after_the_entries() {
        let mut c = Catalog::default();
        c.load(vec![file("a", "A.pdf", 20)], Vec::new(), Views::default(), String::new(), String::new());
        assert!(c.folder("root", None, &mut names).is_empty());
        c.set_root("root-id");
        assert_eq!(ids(&c.folder("root", None, &mut names)), ["a"]);
        assert_eq!(c.entry("a").unwrap().parents, Some(vec!["root".to_string()]));
    }

    #[test]
    fn fingerprint_ignores_the_modified_time() {
        let a = file("a", "A.pdf", 20);
        assert_eq!(a.fingerprint(), "a|A.pdf|0||application/pdf");
        assert_eq!(a.fingerprint(), file("a", "A.pdf", 25).fingerprint());
        assert_ne!(a.fingerprint(), Entry { starred: true, ..a.clone() }.fingerprint());
        assert_eq!(Catalog::compare_entries(&file("a", "x", 1), &file("b", "x", 1), &mut names), Ordering::Less);
    }
}
