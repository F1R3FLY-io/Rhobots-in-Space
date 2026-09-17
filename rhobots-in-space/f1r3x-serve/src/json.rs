//! A string buffer and the same escaping `f1r3x::json` uses.
//!
//! `f1r3x::json` builds its output by `format!` and `join`, which is fine at
//! its sizes. A scene frame here carries a structural tree per card, so it is
//! an order of magnitude larger and is built once per round; it goes into one
//! growing buffer instead.
//!
//! Nothing in this crate parses JSON either. Requests are a line protocol, for
//! the same reason the FFI's are: so that no parser is needed on this side of
//! the wall.

/// A growable output buffer.
pub struct Buf(String);

impl Default for Buf {
    fn default() -> Self {
        Buf::new()
    }
}

impl Buf {
    pub fn new() -> Buf {
        Buf(String::with_capacity(4096))
    }
    pub fn with_capacity(n: usize) -> Buf {
        Buf(String::with_capacity(n))
    }
    /// Append text that is already valid JSON, or is structural punctuation.
    pub fn raw(&mut self, s: &str) {
        self.0.push_str(s);
    }
    /// Append a quoted, escaped string.
    pub fn str(&mut self, s: &str) {
        self.0.push('"');
        self.0.push_str(&esc(s));
        self.0.push('"');
    }
    pub fn num(&mut self, n: u64) {
        self.0.push_str(&n.to_string());
    }
    pub fn bool(&mut self, b: bool) {
        self.0.push_str(if b { "true" } else { "false" });
    }
    /// `"key":` — the caller writes the value.
    pub fn key(&mut self, k: &str) {
        self.0.push('"');
        self.0.push_str(k);
        self.0.push_str("\":");
    }
    pub fn field_str(&mut self, k: &str, v: &str) {
        self.key(k);
        self.str(v);
    }
    pub fn field_num(&mut self, k: &str, v: u64) {
        self.key(k);
        self.num(v);
    }
    pub fn field_bool(&mut self, k: &str, v: bool) {
        self.key(k);
        self.bool(v);
    }
    /// `"key":<already-JSON>`
    pub fn field_raw(&mut self, k: &str, v: &str) {
        self.key(k);
        self.raw(v);
    }
    pub fn comma(&mut self) {
        self.0.push(',');
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_string(self) -> String {
        self.0
    }
}

/// JSON string escaping. Identical to `f1r3x::json::esc`; kept here so this
/// crate does not reach into another's private conventions for something this
/// small.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// A tagged frame: `{"t":"<tag>", …}`. The body writer appends its own fields,
/// each preceded by a comma.
pub fn frame(tag: &str, body: impl FnOnce(&mut Buf)) -> String {
    let mut b = Buf::new();
    b.raw("{\"t\":\"");
    b.raw(tag);
    b.raw("\"");
    body(&mut b);
    b.raw("}");
    b.into_string()
}

/// An error frame, in the same shape the FFI uses for a failed request so a
/// client has one error path and not two.
pub fn error_frame(code: &str, message: &str) -> String {
    frame("error", |b| {
        b.comma();
        b.field_str("code", code);
        b.comma();
        b.field_str("message", message);
    })
}
