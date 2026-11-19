//! A small JSON reader for the texts the shell and auth exchange with the browser: stored tokens and profiles,
//! Google token responses, Drive `about`, test-command arguments. Writing uses `testapi::json_str`.

/// A parsed JSON value. Objects keep their members in source order; a repeated key resolves to its last value,
/// as `JSON.parse` does.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// `JSON.parse(text)`; `None` where it would throw.
    pub fn parse(text: &str) -> Option<Json> {
        let mut p = Parser { s: text.as_bytes(), i: 0 };
        let value = p.value(0)?;
        p.ws();
        (p.i == p.s.len()).then_some(value)
    }
    /// `value[key]` of an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(members) => members.iter().rev().find(|m| m.0 == key).map(|m| &m.1),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn items(&self) -> &[Json] {
        match self {
            Json::Arr(items) => items,
            _ => &[],
        }
    }
    /// `value[key]` when it is a non-empty string (JS: `value.key || ''`).
    pub fn text(&self, key: &str) -> &str { self.get(key).and_then(Json::as_str).unwrap_or("") }
    /// JS truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Json::Null => false,
            Json::Bool(b) => *b,
            Json::Num(n) => *n != 0.0 && !n.is_nan(),
            Json::Str(s) => !s.is_empty(),
            _ => true,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn eat(&mut self, word: &[u8]) -> Option<()> {
        self.s[self.i..].starts_with(word).then(|| self.i += word.len())
    }
    fn value(&mut self, depth: u32) -> Option<Json> {
        self.ws();
        match *self.s.get(self.i)? {
            b'n' => self.eat(b"null").map(|_| Json::Null),
            b't' => self.eat(b"true").map(|_| Json::Bool(true)),
            b'f' => self.eat(b"false").map(|_| Json::Bool(false)),
            b'"' => self.string().map(Json::Str),
            b'[' | b'{' if depth >= 64 => None,
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if items.is_empty() && self.eat(b"]").is_some() {
                        return Some(Json::Arr(items));
                    }
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b"]").is_some() {
                        return Some(Json::Arr(items));
                    }
                    self.eat(b",")?;
                }
            }
            b'{' => {
                self.i += 1;
                let mut members = Vec::new();
                loop {
                    self.ws();
                    if members.is_empty() && self.eat(b"}").is_some() {
                        return Some(Json::Obj(members));
                    }
                    let key = self.string()?;
                    self.ws();
                    self.eat(b":")?;
                    members.push((key, self.value(depth + 1)?));
                    self.ws();
                    if self.eat(b"}").is_some() {
                        return Some(Json::Obj(members));
                    }
                    self.eat(b",")?;
                }
            }
            _ => self.number().map(Json::Num),
        }
    }
    /// A JSON number. Integers up to 2^53 (timestamps, sizes, counts) are exact; a fraction or exponent is
    /// scaled with f64 arithmetic, within one unit in the last place of `JSON.parse`. That is enough for what the
    /// shell reads and keeps the float parser of `core` out of the binary.
    fn number(&mut self) -> Option<f64> {
        let negative = self.eat(b"-").is_some();
        let digits = |p: &mut Self, value: &mut f64| {
            let start = p.i;
            while let Some(d @ b'0'..=b'9') = p.s.get(p.i) {
                *value = *value * 10.0 + (d - b'0') as f64;
                p.i += 1;
            }
            p.i - start
        };
        let (mut value, at) = (0.0, self.i);
        let int_digits = digits(self, &mut value);
        // No leading '+', no bare '.', no leading zeros.
        if int_digits == 0 || (int_digits > 1 && self.s[at] == b'0') {
            return None;
        }
        let mut exponent = 0i32;
        if self.eat(b".").is_some() {
            let n = digits(self, &mut value);
            if n == 0 {
                return None;
            }
            exponent -= n as i32;
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            let minus = match self.s.get(self.i) {
                Some(b'-') => {
                    self.i += 1;
                    true
                }
                Some(b'+') => {
                    self.i += 1;
                    false
                }
                _ => false,
            };
            let mut e = 0.0;
            if digits(self, &mut e) == 0 {
                return None;
            }
            exponent += if minus { -(e.min(400.0) as i32) } else { e.min(400.0) as i32 };
        }
        for _ in 0..exponent.abs().min(700) {
            value = if exponent < 0 { value / 10.0 } else { value * 10.0 };
        }
        Some(if negative { -value } else { value })
    }
    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(h, 16).ok()
    }
    fn string(&mut self) -> Option<String> {
        self.eat(b"\"")?;
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'"' | b'\\' | b'/' => e as char,
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xd800..0xdc00).contains(&cp) && self.s[self.i..].starts_with(b"\\u") {
                                let save = self.i;
                                self.i += 2;
                                let low = self.hex4()?;
                                if (0xdc00..0xe000).contains(&low) {
                                    cp = 0x10000 + ((cp - 0xd800) << 10) + (low - 0xdc00);
                                } else {
                                    self.i = save;
                                }
                            }
                            char::from_u32(cp).unwrap_or('\u{fffd}')
                        }
                        _ => return None,
                    };
                    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
                0..=0x1f => return None,
                c => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_the_shell_reads() {
        let v = Json::parse(r#" {"token":"ya29.a","expiry":1793354400000,"grantedScopes":"a b","n":[1,-2.5e1,true,null],"s":"\u00e9\ud83d\ude00\n\"","token":"last"} "#).unwrap();
        assert_eq!(v.text("token"), "last");
        assert_eq!(v.get("expiry").and_then(Json::as_f64), Some(1_793_354_400_000.0));
        assert_eq!(v.get("n").unwrap().items(), [Json::Num(1.0), Json::Num(-25.0), Json::Bool(true), Json::Null]);
        assert_eq!(v.text("s"), "é😀\n\"");
        assert_eq!((v.text("missing"), v.get("n").unwrap().text("x")), ("", ""));
        assert!(v.truthy() && !Json::Num(0.0).truthy() && !Json::Str(String::new()).truthy() && Json::Arr(vec![]).truthy());
        assert_eq!(Json::parse("[0,-0.5,1e3,2.5E-1,12.75,9007199254740991]").unwrap().items(), [0.0, -0.5, 1000.0, 0.25, 12.75, 9007199254740991.0].map(Json::Num));
        assert_eq!(Json::parse("[]"), Some(Json::Arr(vec![])));
        assert_eq!(Json::parse("{}"), Some(Json::Obj(vec![])));
    }

    #[test]
    fn rejects_what_json_parse_rejects() {
        for bad in ["", "{", "[1,]", "{\"a\":1,}", "01", "+1", "1.", ".5", "-", "1e", "1e+", "nul", "\"a", "\"\\x\"", "1 2", "{'a':1}", "\"\t\""] {
            assert_eq!(Json::parse(bad), None, "{bad:?}");
        }
        assert!(Json::parse(&("[".repeat(65) + &"]".repeat(65))).is_none());
        assert!(Json::parse(&("[".repeat(64) + &"]".repeat(64))).is_some());
    }
}
