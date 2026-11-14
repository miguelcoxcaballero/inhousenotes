//! Offline decoration and sync badges of document cards, from the persisted `docs` flags and the sync activity
//! other units report (ARCHITECTURE §6.6).
// v5: offline-library-v5.js:decorate / syncErrorText

use super::cards::esc;
use super::{sig, Slot};
use crate::dom::Prop;
use crate::{shell, App, Host, Node};

/// A failed upload of a document, as the sync or export unit reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyncError {
    /// The document, or empty for an error that concerns every pending one.
    pub id: String,
    /// HTTP status or one of 5.18.0's codes (`remote-deleted`, `needs-merge`).
    pub code: String,
    pub message: String,
    pub phase: String,
}

/// What Home knows about running sync work (5.18.0 `syncState`).
#[derive(Clone, Debug, PartialEq)]
pub struct Sync {
    /// `idle`, `checking`, `building`, `merging`, `preserving`, `uploading`, or anything else for "Syncing…".
    pub phase: String,
    /// The document being worked on.
    pub id: String,
    pub errors: Vec<SyncError>,
}

impl Default for Sync {
    fn default() -> Sync { Sync { phase: "idle".into(), id: String::new(), errors: Vec::new() } }
}

/// The decoration of one document card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deco {
    pub available: bool,
    pub pinned: bool,
    /// Offline and not on this device: greyed out.
    pub unavailable: bool,
    /// `(data-phase, text, title)`.
    pub badge: Option<(String, String, String)>,
    /// A click on the badge does nothing while the document is being worked on.
    pub active: bool,
    pub error: bool,
}

impl Deco {
    /// The attributes of the card element.
    pub fn attrs(&self) -> String {
        format!(
            "{} data-offline-available=\"{}\" data-offline-pinned=\"{}\"",
            if self.unavailable { " aria-disabled=\"true\" title=\"Connect to download this document\"" } else { "" },
            self.available,
            self.pinned
        )
    }
    pub fn badge_html(&self) -> String {
        self.badge.as_ref().map_or(String::new(), |(phase, text, title)| {
            format!("<button type=\"button\" class=\"document-sync-status\" aria-live=\"polite\" data-phase=\"{}\" title=\"{}\">{}</button>", esc(phase), esc(title), esc(text))
        })
    }
}

// v5: offline-library-v5.js:syncErrorText
pub fn error_text(error: &SyncError) -> String {
    let text = error.message.as_str();
    let lower = text.to_ascii_lowercase();
    // `/auth|sign.?in|refresh token|access token|token expired/i`
    let sign_in = lower.match_indices("sign").any(|(at, _)| {
        let rest = &lower[at + 4..];
        rest.starts_with("in") || rest.chars().next().is_some_and(|c| c != '\n' && rest[c.len_utf8()..].starts_with("in"))
    });
    let auth = sign_in || ["auth", "refresh token", "access token", "token expired"].iter().any(|w| lower.contains(w));
    match error.code.as_str() {
        _ if error.code == "401" || auth => "Reconnect Google Drive to upload. Your local copy is safe.".into(),
        "403" if lower.contains("quota") || lower.contains("storage") => "Google Drive storage is full. Your local copy is safe.".into(),
        "403" => "Drive denied access. Check editing permission.".into(),
        "404" | "remote-deleted" => "The Drive file is missing or unavailable. Your local copy is safe.".into(),
        "412" => "The document changed on Drive. Retrying the merge.".into(),
        "needs-merge" => "Waiting to verify the Drive revision. Your local copy is safe.".into(),
        _ => {
            // `text.replace(/Drive API error \d+:.*$/s, 'Drive request failed').slice(0, 160)`
            let mut shown = text.to_string();
            if let Some(at) = text.find("Drive API error ") {
                let rest = &text[at + 16..];
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                if digits > 0 && rest[digits..].starts_with(':') {
                    shown = format!("{}Drive request failed", &text[..at]);
                }
            }
            let cut: String = String::from_utf16_lossy(&shown.encode_utf16().take(160).collect::<Vec<_>>());
            format!("Upload failed: {}", if cut.is_empty() { "Check your connection and retry." } else { &cut })
        }
    }
}

/// The decoration document `id` gets now.
pub fn deco(app: &App, id: &str) -> Deco {
    let st = &app.st.home;
    let doc = st.catalog.doc(id).copied().unwrap_or_default();
    let offline = shell::offline(app);
    let unavailable = offline && !doc.available;
    let active = st.sync.phase != "idle" && st.sync.id == id;
    let error = st.sync.errors.iter().find(|e| e.id.is_empty() || e.id == id);
    let mut out = Deco { available: doc.available, pinned: doc.pinned, unavailable, badge: None, active, error: false };
    if !doc.pending && !active {
        return out;
    }
    let (phase, text) = if active {
        let text = match st.sync.phase.as_str() {
            "checking" => "Checking…",
            "building" => "Preparing…",
            "merging" => "Merging…",
            "preserving" => "Keeping both versions…",
            "uploading" => "Uploading…",
            _ => "Syncing…",
        };
        (st.sync.phase.clone(), text)
    } else if error.is_some() {
        ("error".to_string(), "Retry upload")
    } else {
        ("pending".to_string(), if offline { "Saved offline" } else { "Waiting to upload" })
    };
    out.error = !active && error.is_some();
    let title = match error {
        Some(e) => error_text(e),
        None => text.to_string(),
    };
    out.badge = Some((phase, text.to_string(), title));
    out
}

/// Brings a card from decoration `old` to `new`; returns the badge's node (`Node::NONE` without a badge).
pub fn apply(host: &mut dyn Host, card: Node, badge: Node, old: &Deco, new: &Deco) -> Node {
    let out = host.out();
    if old.unavailable != new.unavailable {
        out.class(card, "offline-unavailable", new.unavailable);
        out.attr(card, "aria-disabled", new.unavailable.then_some("true"));
        if new.unavailable {
            out.prop(card, "title", Prop::Str("Connect to download this document"));
        } else {
            out.attr(card, "title", None);
        }
    }
    if old.available != new.available {
        out.attr(card, "data-offline-available", Some(if new.available { "true" } else { "false" }));
    }
    if old.pinned != new.pinned {
        out.attr(card, "data-offline-pinned", Some(if new.pinned { "true" } else { "false" }));
    }
    if old.badge == new.badge {
        return badge;
    }
    match &new.badge {
        None => {
            out.remove(badge);
            out.release(badge);
            Node::NONE
        }
        Some((phase, text, title)) => {
            let node = if badge.is_none() { out.insert_html(card, Node::NONE, &new.badge_html()) } else { badge };
            out.attr(node, "data-phase", Some(phase));
            out.text(node, text);
            out.prop(node, "title", Prop::Str(title));
            node
        }
    }
}

/// Every document card follows the flags and the sync activity again.
// v5: offline-library-v5.js:setSyncProgress (`querySelectorAll('[data-drive-item-id]').forEach(decorate)`)
pub fn refresh(app: &mut App, host: &mut dyn Host) {
    for slot in Slot::ALL {
        for i in 0..app.st.home.slots[slot as usize].cards.len() {
            let card = &app.st.home.slots[slot as usize].cards[i];
            if card.entry.is_folder() || app.st.home.catalog.is_folder(&card.id) {
                continue;
            }
            let (node, badge, old, new) = (card.node, card.badge, card.deco.clone(), deco(app, &card.id));
            if old != new {
                let badge = apply(host, node, badge, &old, &new);
                let card = &mut app.st.home.slots[slot as usize].cards[i];
                card.badge = badge;
                card.deco = new;
            }
        }
    }
}

/// The sync or export unit reports its activity: `phase` `idle` when nothing runs.
pub fn set_sync(app: &mut App, host: &mut dyn Host, phase: &str, id: &str) {
    let sync = &mut app.st.home.sync;
    if sync.phase != phase || sync.id != id {
        sync.phase = phase.into();
        sync.id = id.into();
        refresh(app, host);
    }
}

/// Replaces the upload errors (empty = everything retried or done).
pub fn set_errors(app: &mut App, host: &mut dyn Host, errors: Vec<SyncError>) {
    if app.st.home.sync.errors != errors {
        app.st.home.sync.errors = errors;
        refresh(app, host);
    }
}

/// A click on a card's badge. v5: offline-library-v5.js:decorate `badge.onclick`
pub fn clicked(app: &mut App, host: &mut dyn Host, id: &str, deco: &Deco) {
    if deco.active {
        return;
    }
    if deco.error {
        // The Offline dialogs unit shows `#offline-sync-error` for this document.
        shell::signal(app, host, sig::SYNC_ERROR, id);
    } else if !shell::offline(app) {
        shell::signal(app, host, sig::RETRY_SYNC, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_texts_are_5_18s() {
        let e = |code: &str, message: &str| error_text(&SyncError { code: code.into(), message: message.into(), ..Default::default() });
        assert_eq!(e("401", ""), "Reconnect Google Drive to upload. Your local copy is safe.");
        assert_eq!(e("", "Access token expired"), "Reconnect Google Drive to upload. Your local copy is safe.");
        assert_eq!(e("403", "storageQuotaExceeded"), "Google Drive storage is full. Your local copy is safe.");
        assert_eq!(e("403", "no"), "Drive denied access. Check editing permission.");
        assert_eq!(e("remote-deleted", ""), "The Drive file is missing or unavailable. Your local copy is safe.");
        assert_eq!(e("412", ""), "The document changed on Drive. Retrying the merge.");
        assert_eq!(e("needs-merge", ""), "Waiting to verify the Drive revision. Your local copy is safe.");
        assert_eq!(e("500", "Drive API error 500: {\"error\":1}"), "Upload failed: Drive request failed");
        assert_eq!(e("", ""), "Upload failed: Check your connection and retry.");
        assert_eq!(e("", &"x".repeat(200)).len(), "Upload failed: ".len() + 160);
    }

    #[test]
    fn the_markup_of_a_badge() {
        let deco = Deco { badge: Some(("pending".into(), "Waiting to upload".into(), "Waiting to upload".into())), available: true, ..Default::default() };
        assert_eq!(deco.badge_html(), "<button type=\"button\" class=\"document-sync-status\" aria-live=\"polite\" data-phase=\"pending\" title=\"Waiting to upload\">Waiting to upload</button>");
        assert_eq!(deco.attrs(), " data-offline-available=\"true\" data-offline-pinned=\"false\"");
        assert_eq!(Deco { unavailable: true, ..Default::default() }.attrs(), " aria-disabled=\"true\" title=\"Connect to download this document\" data-offline-available=\"false\" data-offline-pinned=\"false\"");
    }
}
