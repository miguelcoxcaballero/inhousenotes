//! Spec parsing with precise, actionable error messages: file, line and
//! column, the offending source line with a caret, and "did you mean"
//! suggestions for misspelt keys, types and names.

use std::fmt;

use super::spec::Spec;

/// Error from loading or building a landmark.
#[derive(Clone, Debug)]
pub struct Error {
    pub message: String,
    /// `file:line:col` when known.
    pub location: Option<String>,
    /// Source line with a caret under the problem.
    pub snippet: Option<String>,
}

impl Error {
    pub fn msg(m: impl Into<String>) -> Self {
        Error { message: m.into(), location: None, snippet: None }
    }

    pub fn at(mut self, location: String, snippet: Option<String>) -> Self {
        if self.location.is_none() {
            self.location = Some(location);
            self.snippet = snippet;
        }
        self
    }

    /// Prefixes context ("element `nave` > openings[1]: ...").
    pub fn context(mut self, ctx: &str) -> Self {
        self.message = format!("{ctx}: {}", self.message);
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.location {
            Some(l) => write!(f, "{l}: {}", self.message)?,
            None => write!(f, "{}", self.message)?,
        }
        if let Some(s) = &self.snippet {
            write!(f, "\n{s}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// 1-based line and column of a byte offset.
pub fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map(|s| s.chars().count()).unwrap_or(0) + 1;
    (line, col)
}

/// The source line at `offset` with a caret marker under `len` chars.
pub fn snippet(text: &str, offset: usize, len: usize) -> String {
    let (line, col) = line_col(text, offset);
    let src = text.lines().nth(line - 1).unwrap_or("");
    let num = format!("{line}");
    let pad = " ".repeat(num.len());
    let width = len.clamp(1, src.chars().count().saturating_sub(col - 1).max(1));
    format!("{pad} |\n{num} | {src}\n{pad} | {}{}", " ".repeat(col - 1), "^".repeat(width))
}

/// Levenshtein distance.
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let c = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + c);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// `"; did you mean `x`?"` for the closest candidate, or `""`.
pub fn did_you_mean(word: &str, candidates: &[&str]) -> String {
    let w = word.to_ascii_lowercase();
    let best = candidates
        .iter()
        .map(|c| {
            let d = distance(&w, &c.to_ascii_lowercase());
            let bonus = if c.starts_with(&w) || w.starts_with(*c) { 0 } else { 1 };
            (d * 2 + bonus, *c)
        })
        .min_by_key(|(d, _)| *d);
    match best {
        Some((d, c)) if d / 2 <= (w.len() / 3).max(1) + 1 && c != w => format!("; did you mean `{c}`?"),
        _ => String::new(),
    }
}

/// Adds a suggestion to serde's "unknown field `x`, expected one of `a`,
/// `b`" and "unknown variant" messages.
fn enrich(message: &str) -> String {
    for head in ["unknown field `", "unknown variant `"] {
        if let Some(rest) = message.find(head).map(|i| &message[i + head.len()..]) {
            let Some(end) = rest.find('`') else { continue };
            let word = &rest[..end];
            let expected: Vec<&str> = rest[end + 1..].split('`').skip(1).step_by(2).collect();
            let hint = did_you_mean(word, &expected);
            if !hint.is_empty() && !message.contains("did you mean") {
                return format!("{message}{hint}");
            }
        }
    }
    message.to_string()
}

/// When serde could only point at a whole table, find the key the message
/// names inside that table.
fn refine(text: &str, start: usize, message: &str) -> Option<(usize, usize)> {
    let key = ["unknown field `", "missing field `", "invalid type", "invalid value"].iter().find_map(|h| {
        let i = message.find(h)?;
        let rest = &message[i + h.len()..];
        if h.starts_with("unknown") || h.starts_with("missing") { rest.split('`').next().map(|s| s.to_string()) } else { None }
    })?;
    let line_start = text[..start.min(text.len())].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let head = text[line_start..].lines().next().unwrap_or("");
    if !head.trim_start().starts_with('[') {
        return None;
    }
    let mut off = line_start + head.len() + 1;
    for line in text.get(off..)?.lines() {
        let t = line.trim_start();
        if t.starts_with("[[") || (t.starts_with('[') && !t.starts_with("[[")) && !t.contains('=') {
            break;
        }
        if let Some(rest) = t.strip_prefix(key.as_str())
            && rest.trim_start().starts_with('=')
        {
            let col = line.len() - t.len();
            return Some((off + col, key.len()));
        }
        off += line.len() + 1;
    }
    None
}

/// Parses `landmark.toml` text. `origin` names the file in messages.
pub fn parse_spec(text: &str, origin: &str) -> Result<Spec> {
    match toml::from_str::<Spec>(text) {
        Ok(s) => Ok(s),
        Err(e) => {
            let msg = enrich(e.message().trim());
            let mut err = Error::msg(msg);
            if let Some(span) = e.span() {
                // Underline the key or value (one line at most).
                let mut len = text[span.start.min(text.len())..span.end.min(text.len())].lines().next().map(|s| s.chars().count()).unwrap_or(1);
                let mut at = span.start;
                if let Some((o, l)) = refine(text, span.start, e.message()) {
                    at = o;
                    len = l;
                }
                let (l, c) = line_col(text, at);
                err = err.at(format!("{origin}:{l}:{c}"), Some(snippet(text, at, len)));
            } else {
                err = err.at(origin.to_string(), None);
            }
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions() {
        assert_eq!(did_you_mean("radus", &["radius", "center", "height"]), "; did you mean `radius`?");
        assert_eq!(did_you_mean("hemispere", &["hemisphere", "onion"]), "; did you mean `hemisphere`?");
        assert_eq!(did_you_mean("zzzzzz", &["radius"]), "");
    }

    #[test]
    fn line_and_column() {
        let t = "a = 1\nbb = 2\n  ccc = 3";
        assert_eq!(line_col(t, 0), (1, 1));
        assert_eq!(line_col(t, 6), (2, 1));
        assert_eq!(line_col(t, t.find("ccc").unwrap()), (3, 3));
    }

    #[test]
    fn unknown_key_in_element_points_at_the_key() {
        let t = r#"
[landmark]
name = "x"

[[element]]
type = "dome"
center = [0, 0]
radus = 4.0
"#;
        let e = parse_spec(t, "landmark.toml").unwrap_err();
        let s = e.to_string();
        assert!(s.contains("landmark.toml:8:1"), "{s}");
        assert!(s.contains("did you mean `radius`"), "{s}");
        assert!(s.contains("^^^^^"), "{s}");
    }

    #[test]
    fn unknown_type_and_bad_value() {
        let t = "[landmark]\nname = \"x\"\n[[element]]\ntype = \"cylindre\"\n";
        let s = parse_spec(t, "f").unwrap_err().to_string();
        assert!(s.contains("did you mean `cylinder`"), "{s}");
        let t = "[landmark]\nname = \"x\"\n[[element]]\ntype = \"roof\"\nshape = \"hipp\"\n";
        let s = parse_spec(t, "f").unwrap_err().to_string();
        assert!(s.contains("did you mean `hip`") && s.contains("f:5:"), "{s}");
        let t = "[landmark]\nname = \"x\"\n[[element]]\nheight = 3\ntype = \"column\"\nradius = \"wide\"\n";
        let s = parse_spec(t, "f").unwrap_err().to_string();
        assert!(s.contains("f:6:"), "{s}");
    }
}
