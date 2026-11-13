//! Home cards as HTML: the A4 sheet of a document, the manila folder, skeletons and deferred placeholders, with the
//! texts 5.18.0 prints on them.

use super::badges::Deco;
use crate::{Host, IntlOp};
use ihn_core::catalog::{parse_time, Entry};
use ihn_core::jsfmt;
use ihn_core::palette::{self, Palette};

/// Text or attribute content.
pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

// JS strings are UTF-16: lengths and slices count code units.
fn units(text: &str) -> Vec<u16> { text.encode_utf16().collect() }
fn from_units(units: &[u16]) -> String { String::from_utf16_lossy(units) }

/// JS `String.prototype.trim`.
fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| matches!(c, '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'))
}

// v5: app-v5.js:formatDriveCardFileName
pub fn format_name(name: &str, fallback: &str) -> String {
    let trimmed = js_trim(name);
    let raw = units(if trimmed.is_empty() { fallback } else { trimmed });
    const MAX: usize = 38;
    if raw.len() <= MAX {
        return from_units(&raw);
    }
    if let Some(dot) = raw.iter().rposition(|u| *u == b'.' as u16).filter(|d| *d > 0 && *d < raw.len() - 1) {
        let ext = from_units(&raw[dot..]);
        let suffix = units(&format!("... {}", if ext.eq_ignore_ascii_case(".pdf") { ".pdf".to_string() } else { ext }));
        if MAX > suffix.len() + 6 {
            return from_units(&[&raw[..MAX - suffix.len()], &suffix[..]].concat());
        }
    }
    format!("{}...", from_units(&raw[..MAX - 3]))
}

/// The printed name: `.pdf` dropped, long names shortened.
// v5: app-v5.js:applyDriveTitleWithSharedIcon
pub fn display_name(e: &Entry, fallback: &str) -> String {
    let name = if e.name.is_empty() { fallback } else { &e.name };
    let u = units(name);
    let stem = if u.len() >= 4 && from_units(&u[u.len() - 4..]).eq_ignore_ascii_case(".pdf") { from_units(&u[..u.len() - 4]) } else { name.to_string() };
    format_name(if stem.is_empty() { name } else { &stem }, fallback)
}

const SHARED_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" class="drive-shared-icon"><circle cx="12" cy="8" r="2.2"></circle><circle cx="6.8" cy="9" r="1.8"></circle><circle cx="17.2" cy="9" r="1.8"></circle><path d="M8.5 17c.4-2 1.9-3.5 3.5-3.5s3.1 1.5 3.5 3.5"></path><path d="M3.8 17c.3-1.5 1.5-2.7 3-2.9"></path><path d="M20.2 17c-.3-1.5-1.5-2.7-3-2.9"></path></svg>"#;

/// `(class, title attribute, inner HTML)` of a `.drive-card-title`.
fn title(e: &Entry, fallback: &str) -> (&'static str, String, String) {
    let name = if e.name.is_empty() { fallback } else { &e.name };
    let shown = esc(&display_name(e, fallback));
    if e.shared() {
        ("drive-card-title is-shared", esc(name), format!("{SHARED_ICON}<span class=\"drive-card-title-text\">{shown}</span>"))
    } else {
        ("drive-card-title", esc(name), shown)
    }
}

/// The clock of one render pass: local "today" and its year, asked from the browser once.
pub struct Today {
    now: f64,
    day: Option<String>,
}

impl Today {
    pub fn new(host: &dyn Host) -> Today { Today { now: host.time(), day: None } }
}

fn local_day(host: &mut dyn Host, ms: f64) -> String {
    // `YYYY-MM-DD` in the local time zone.
    host.intl(IntlOp::Date, &format!("{}\0date\0\"en-CA\"\0", jsfmt::num(ms)))
}

// v5: app-v5.js:formatDriveShortDate
pub fn short_date(host: &mut dyn Host, today: &mut Today, value: &str) -> String {
    let Some(ms) = parse_time(value) else { return String::new() };
    if today.day.is_none() {
        today.day = Some(local_day(host, today.now));
    }
    let (now, day) = (today.day.clone().unwrap_or_default(), local_day(host, ms));
    let date = |host: &mut dyn Host, method: &str, options: &str| host.intl(IntlOp::Date, &format!("{}\0{method}\0\0{options}", jsfmt::num(ms)));
    if day == now {
        date(host, "time", r#"{"hour":"2-digit","minute":"2-digit"}"#)
    } else if day.get(..4) == now.get(..4) {
        date(host, "date", r#"{"month":"short","day":"numeric"}"#)
    } else {
        date(host, "date", r#"{"month":"short","day":"numeric","year":"numeric"}"#)
    }
}

// v5: app-v5.js:describeDriveFile
pub fn describe(host: &mut dyn Host, today: &mut Today, e: &Entry) -> String {
    if e.pending {
        return "Saved on this device".into();
    }
    let date = short_date(host, today, if e.modified.is_empty() { &e.created } else { &e.modified });
    match (e.shared(), date.is_empty()) {
        (true, false) => format!("Shared · {date}"),
        (true, true) => "Shared with you".into(),
        (false, false) => format!("Edited {date}"),
        (false, true) => "PDF".into(),
    }
}

// v5: app-v5.js:createDriveItemMenu (the dropdown's buttons are rendered when it opens, by the card-menu unit)
const MENU: &str = "<div class=\"drive-card-menu\"><button type=\"button\" class=\"drive-card-more\" title=\"More actions\" aria-label=\"More actions\">
                <svg viewBox=\"0 0 24 24\" width=\"16\" height=\"16\" fill=\"currentColor\" aria-hidden=\"true\">
                    <circle cx=\"12\" cy=\"12\" r=\"2\"></circle>
                    <circle cx=\"12\" cy=\"5\" r=\"2\"></circle>
                    <circle cx=\"12\" cy=\"19\" r=\"2\"></circle>
                </svg>
            </button><div class=\"drive-card-dropdown hidden\"></div></div>";
const FILE_ICON: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="drive-card-icon"><path d="M6 2h8l4 4v16a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z"/><path d="M14 2v6h6"/></svg>"#;

/// `children[..]` from a file card to its `.drive-card-menu` (the image replaces the icon, so the path holds).
pub const FILE_MENU_PATH: &[u8] = &[0, 1, 1];
/// From a folder card to the `.drive-card-menu` on its cover.
pub const FOLDER_MENU_PATH: &[u8] = &[0, 2, 0, 1];
/// From a folder card to its `.drive-folder-art` and from the art to its cover.
pub const ART_PATH: &[u8] = &[0];
pub const COVER_PATH: &[u8] = &[2];

/// What a sheet currently shows of its thumbnail (kept by the list so a snapshot can repeat it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    pub url: String,
    pub loading: bool,
    pub landscape: bool,
    /// `--sheet-ratio` (already `toFixed(4)`), empty for A4.
    pub ratio: String,
}

/// The children of a file card.
// v5: app-v5.js:createDriveFileCard / createDrivePreview / createDriveCardInfo
pub fn file_inner(e: &Entry, sub: &str, sheet: &Sheet, deco: &Deco) -> String {
    let (class, full, text) = title(e, "Untitled");
    let mut preview = String::from("drive-card-preview");
    for (on, name) in [(sheet.loading, " is-loading"), (!sheet.url.is_empty(), " is-loaded"), (sheet.landscape, " is-landscape")] {
        if on {
            preview.push_str(name);
        }
    }
    let picture = if sheet.url.is_empty() { FILE_ICON.to_string() } else { image(e, &sheet.url) };
    format!(
        "<div class=\"{preview}\">{picture}<div class=\"drive-card-info\"><div class=\"drive-card-text\"><div class=\"{class}\" title=\"{full}\">{text}</div><div class=\"drive-card-sub\" data-file-sub=\"true\">{}</div></div>{MENU}</div></div>{}{}",
        esc(sub),
        if e.starred { "<span class=\"drive-sheet-flag\" aria-hidden=\"true\"></span>" } else { "" },
        deco.badge_html(),
    )
}

/// The displayed thumbnail.
pub fn image(e: &Entry, url: &str) -> String { format!("<img decoding=\"async\" alt=\"{} preview\" src=\"{}\">", esc(&e.name), esc(url)) }

/// A whole file card.
pub fn file_card(e: &Entry, sub: &str, sheet: &Sheet, deco: &Deco, arriving: bool) -> String {
    let style = if sheet.ratio.is_empty() { String::new() } else { format!(" style=\"--sheet-ratio: {};\"", sheet.ratio) };
    format!(
        "<div class=\"drive-card{}{}\" tabindex=\"0\"{}{style} data-drive-item-id=\"{}\" data-drive-item-fp=\"{}\">{}</div>",
        if arriving { " drive-folder-item-arriving" } else { "" },
        if deco.unavailable { " offline-unavailable" } else { "" },
        deco.attrs(),
        esc(&e.id),
        esc(&e.fingerprint()),
        file_inner(e, sub, sheet, deco),
    )
}

// v5: app-v5.js:createDriveFolderArt
fn name_size(name: &str) -> Option<String> {
    // `name.split(/\s+/)`: the longest run without white space, in UTF-16 units.
    let longest = name.split(|c: char| c.is_whitespace() || c == '\u{feff}').map(|w| w.encode_utf16().count()).max().unwrap_or(0);
    (longest > 10).then(|| format!("{}cqi", jsfmt::fixed((120.0 / longest as f64).max(6.0), 2)))
}

pub fn palette_style(p: &Palette) -> String { p.vars().iter().map(|(k, v)| format!("{k}: {v};")).collect::<Vec<_>>().join(" ") }

/// The `.drive-folder-art` of a folder: `extra_class` / `extra_style` are the ghost's additions.
pub fn folder_art(e: &Entry, night: bool, menu: bool, extra_class: &str, extra_attrs: &str, extra_style: &str, contents: bool) -> String {
    let color = palette::resolve_color(&e.folder_color);
    let (class, full, text) = title(e, "Folder");
    let mut style = name_size(&e.name).map_or(String::new(), |s| format!("--folder-name-size: {s}; "));
    style.push_str(&palette_style(&palette::palette(&color, night, false)));
    style.push_str(extra_style);
    format!(
        "<div class=\"drive-folder-art{extra_class}\" style=\"{style}\" data-folder-color=\"{color}\"{extra_attrs}><span class=\"drive-folder-back\" aria-hidden=\"true\"></span><span class=\"drive-folder-sheets\" aria-hidden=\"true\"></span>{}<span class=\"drive-folder-cover\"><span class=\"drive-folder-cover-face\"><span class=\"drive-folder-print\"><span class=\"{class}\" title=\"{full}\">{text}</span><span class=\"drive-folder-rule\" aria-hidden=\"true\"></span></span>{}</span><span class=\"drive-folder-cover-inside\" aria-hidden=\"true\"></span></span></div>",
        if contents { "<div class=\"drive-folder-contents\"></div>" } else { "" },
        if menu { MENU } else { "" },
    )
}

/// A whole folder card.
// v5: app-v5.js:createDriveFolderCard
pub fn folder_card(e: &Entry, night: bool, arriving: bool) -> String {
    format!(
        "<div class=\"drive-folder-card{}\" tabindex=\"0\" data-drive-item-id=\"{}\" data-drive-item-fp=\"{}\">{}</div>",
        if arriving { " drive-folder-item-arriving" } else { "" },
        esc(&e.id),
        esc(&e.fingerprint()),
        folder_art(e, night, true, "", "", "", false),
    )
}

/// The slot a card of an opened folder holds until it is near the viewport.
// v5: app-v5.js:createDeferredDriveFolderCard
pub fn deferred_card(e: &Entry, arriving: bool, deco: Option<&Deco>) -> String {
    format!(
        "<div class=\"{} drive-card-deferred{}{}\" style=\"aspect-ratio: 210 / 297;\" tabindex=\"0\" aria-label=\"{}\"{} data-drive-item-id=\"{}\" data-drive-item-fp=\"{}\">{}</div>",
        if e.is_folder() { "drive-folder-card" } else { "drive-card" },
        if arriving { " drive-folder-item-arriving" } else { "" },
        if deco.is_some_and(|d| d.unavailable) { " offline-unavailable" } else { "" },
        esc(if e.name.is_empty() { "Document" } else { &e.name }),
        deco.map_or(String::new(), Deco::attrs),
        esc(&e.id),
        esc(&e.fingerprint()),
        deco.map_or(String::new(), Deco::badge_html),
    )
}

// v5: app-v5.js:buildDriveCardSkeleton / buildDriveFolderSkeleton
pub const CARD_SKELETON: &str = "<div class=\"drive-card-skeleton\" aria-hidden=\"true\"><div class=\"skeleton-bar title\"></div><div class=\"skeleton-preview\"></div></div>";
pub const FOLDER_SKELETON: &str = "<div class=\"drive-folder-card-skeleton\" aria-hidden=\"true\"><div class=\"skeleton-icon\"></div><div class=\"skeleton-bar\"></div></div>";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FakeHost;
    use ihn_core::catalog::{FOLDER, PDF};

    fn file(name: &str) -> Entry { Entry { id: "f1".into(), name: name.into(), mime: PDF.into(), owned_by_me: Some(true), ..Default::default() } }

    #[test]
    fn names_are_shortened_like_5_18() {
        assert_eq!(format_name("Notes", "Untitled"), "Notes");
        assert_eq!(format_name("   ", "Untitled"), "Untitled");
        let long = "A very long notebook name that keeps going and going";
        assert_eq!(format_name(long, "Untitled"), "A very long notebook name that keep...");
        assert_eq!(format_name(&format!("{long}.PDF"), "x"), "A very long notebook name that... .pdf");
        assert_eq!(format_name(&format!("{long}.docx"), "x"), "A very long notebook name tha... .docx");
        // An extension too long to leave six characters of the name is not kept.
        assert_eq!(format_name("abcdefgh.a-very-long-extension-of-thirty-chars", "x"), "abcdefgh.a-very-long-extension-of-t...");
        assert_eq!(display_name(&file("Journal.pdf"), "Untitled"), "Journal");
        assert_eq!(display_name(&file("Journal.PDF"), "Untitled"), "Journal");
        assert_eq!(display_name(&file(".pdf"), "Untitled"), ".pdf");
        assert_eq!(display_name(&file(""), "Untitled"), "Untitled");
        assert_eq!(esc("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
    }

    #[test]
    fn the_sub_line_describes_the_file() {
        let mut host = FakeHost::default();
        let mut today = Today::new(&host);
        assert_eq!(describe(&mut host, &mut today, &Entry { pending: true, ..file("a") }), "Saved on this device");
        assert_eq!(describe(&mut host, &mut today, &file("a")), "PDF");
        assert_eq!(describe(&mut host, &mut today, &Entry { owned_by_me: Some(false), ..file("a") }), "Shared with you");
        // The FakeHost answers a date request with its first field: the epoch milliseconds.
        let edited = Entry { modified: "2026-10-30T09:00:00.000Z".into(), ..file("a") };
        assert_eq!(describe(&mut host, &mut today, &edited), "Edited 1793350800000");
        assert_eq!(describe(&mut host, &mut today, &Entry { owned_by_me: Some(false), created: "2026-10-30T09:00:00Z".into(), ..file("a") }), "Shared · 1793350800000");
    }

    #[test]
    fn a_file_card_is_the_markup_of_create_drive_file_card() {
        let html = file_card(&Entry { starred: true, ..file("A <b>.pdf") }, "PDF", &Sheet::default(), &Deco::default(), false);
        assert!(html.starts_with("<div class=\"drive-card\" tabindex=\"0\" data-offline-available=\"false\" data-offline-pinned=\"false\" data-drive-item-id=\"f1\" data-drive-item-fp=\"f1|A &lt;b&gt;.pdf|1||application/pdf\"><div class=\"drive-card-preview\"><svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" class=\"drive-card-icon\">"), "{html}");
        assert!(html.contains("<div class=\"drive-card-title\" title=\"A &lt;b&gt;.pdf\">A &lt;b&gt;</div><div class=\"drive-card-sub\" data-file-sub=\"true\">PDF</div></div><div class=\"drive-card-menu\"><button type=\"button\" class=\"drive-card-more\""));
        assert!(html.ends_with("<div class=\"drive-card-dropdown hidden\"></div></div></div></div><span class=\"drive-sheet-flag\" aria-hidden=\"true\"></span></div>"));
        let shared = file_card(&Entry { owned_by_me: Some(false), ..file("S.pdf") }, "Shared with you", &Sheet { url: "blob:x".into(), landscape: true, ratio: "1.4148".into(), loading: false }, &Deco::default(), true);
        assert!(shared.starts_with("<div class=\"drive-card drive-folder-item-arriving\" tabindex=\"0\" data-offline-available=\"false\" data-offline-pinned=\"false\" style=\"--sheet-ratio: 1.4148;\""));
        assert!(shared.contains("<div class=\"drive-card-preview is-loaded is-landscape\"><img decoding=\"async\" alt=\"S.pdf preview\" src=\"blob:x\"><div class=\"drive-card-info\">"));
        assert!(shared.contains("<div class=\"drive-card-title is-shared\" title=\"S.pdf\"><svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\" class=\"drive-shared-icon\">"));
        assert!(shared.contains("<span class=\"drive-card-title-text\">S</span></div>"));
    }

    #[test]
    fn a_folder_card_prints_its_name_on_the_cover_with_the_palette() {
        let folder = Entry { id: "d1".into(), name: "Extraordinarily long".into(), mime: FOLDER.into(), folder_color: "#4986E7".into(), ..Default::default() };
        let html = folder_card(&folder, false, false);
        assert!(html.starts_with("<div class=\"drive-folder-card\" tabindex=\"0\" data-drive-item-id=\"d1\" data-drive-item-fp=\"d1|Extraordinarily long|0|#4986E7|application/vnd.google-apps.folder\"><div class=\"drive-folder-art\" style=\"--folder-name-size: 8.00cqi; --folder-color: #4986e7; --folder-cover: #528ce8; --folder-inside: #90b2e8; --folder-tab: #1c64d7; --folder-ink: #1f1a14; --folder-hl: rgba(0, 0, 0, 0.07); --folder-hl-strong: rgba(0, 0, 0, 0.12); --folder-line: rgba(0, 0, 0, 0.25);\" data-folder-color=\"#4986e7\"><span class=\"drive-folder-back\" aria-hidden=\"true\"></span>"), "{html}");
        assert!(html.contains("<span class=\"drive-folder-print\"><span class=\"drive-card-title\" title=\"Extraordinarily long\">Extraordinarily long</span><span class=\"drive-folder-rule\" aria-hidden=\"true\"></span></span><div class=\"drive-card-menu\">"));
        assert!(html.ends_with("</div></span><span class=\"drive-folder-cover-inside\" aria-hidden=\"true\"></span></span></div></div>"));
        // A short name keeps the stylesheet's size; an uncoloured folder is manila.
        let plain = folder_art(&Entry { name: "Maths".into(), mime: FOLDER.into(), ..Default::default() }, true, false, " drive-folder-ghost", " inert", " left: 1px;", true);
        assert!(plain.starts_with("<div class=\"drive-folder-art drive-folder-ghost\" style=\"--folder-color: #c6a261;"));
        assert!(plain.contains("left: 1px;\" data-folder-color=\"#e3c58e\" inert>") && plain.contains("<div class=\"drive-folder-contents\"></div><span class=\"drive-folder-cover\">") && !plain.contains("drive-card-menu"));
        assert_eq!(name_size("one two").as_deref(), None);
        assert_eq!(name_size("abcdefghijklmnopqrstuvwxyz").as_deref(), Some("6.00cqi"));
        assert_eq!(name_size("a abcdefghijklm").as_deref(), Some("9.23cqi"));
    }

    #[test]
    fn deferred_slots_reserve_an_a4_cell() {
        let html = deferred_card(&file("A.pdf"), true, Some(&Deco::default()));
        assert_eq!(html, "<div class=\"drive-card drive-card-deferred drive-folder-item-arriving\" style=\"aspect-ratio: 210 / 297;\" tabindex=\"0\" aria-label=\"A.pdf\" data-offline-available=\"false\" data-offline-pinned=\"false\" data-drive-item-id=\"f1\" data-drive-item-fp=\"f1|A.pdf|0||application/pdf\"></div>");
        assert!(deferred_card(&Entry { mime: FOLDER.into(), ..Default::default() }, false, None).starts_with("<div class=\"drive-folder-card drive-card-deferred\" style=\"aspect-ratio: 210 / 297;\" tabindex=\"0\" aria-label=\"Document\" data-drive-item-id"));
    }
}
