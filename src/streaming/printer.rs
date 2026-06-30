//! The emit engine for the streaming formatter: turns a validated token stream
//! into formatted bytes. Tracks the output column for unicode-aware width
//! decisions and places comments positionally.

use dprint_core::configuration::NewLineKind;

use crate::configuration::{Configuration, TrailingCommaKind};
use super::{Kind, Token, is_close, is_comment, is_open};

/// Previous emitted token kind, for picking single-line separators.
#[derive(PartialEq)]
enum Prev {
  Open,
  Comma,
  Comment,
  Value,
}

fn char_width(bytes: &[u8]) -> usize {
  // unicode display width (UAX#11), matching dprint-core's line measurement.
  // Fast path: printable ASCII is 1 column/byte (the overwhelmingly common case
  // in JSON), so skip the per-char table lookup entirely.
  if bytes.is_ascii() {
    return bytes.len();
  }
  match std::str::from_utf8(bytes) {
    Ok(s) => unicode_width::UnicodeWidthStr::width(s),
    Err(_) => bytes.len(),
  }
}

struct Printer<'a> {
  src: &'a [u8],
  toks: &'a [Token],
  out: Vec<u8>,
  col: usize,
  line_width: usize,
  indent_width: usize,
  use_tabs: bool,
  is_jsonc: bool,
  any_comments: bool,
  force_space_after_slashes: bool,
  ignore_text: &'a str,
  trailing_commas: TrailingCommaKind,
  array_prefer_single_line: bool,
  object_prefer_single_line: bool,
  newline: &'static [u8],
}

impl<'a> Printer<'a> {
  // ---- low-level emit (tracks column) ----
  fn emit(&mut self, b: &[u8]) {
    if let Some(pos) = b.iter().rposition(|&c| c == b'\n') {
      self.col = char_width(&b[pos + 1..]);
    } else {
      self.col += char_width(b);
    }
    self.out.extend_from_slice(b);
  }

  fn nl(&mut self) {
    self.out.extend_from_slice(self.newline);
    self.col = 0;
  }

  fn space(&mut self) {
    self.out.push(b' ');
    self.col += 1;
  }

  fn indent(&mut self, level: usize) {
    let cols = level * self.indent_width;
    if self.use_tabs {
      for _ in 0..level {
        self.out.push(b'\t');
      }
    } else {
      for _ in 0..cols {
        self.out.push(b' ');
      }
    }
    self.col += cols;
  }

  // ---- string / key / comment rendering ----
  fn render_string(&self, t: &Token, buf: &mut Vec<u8>) {
    let raw = &self.src[t.start..t.end];
    let quote = raw[0];
    let inner = &raw[1..raw.len() - 1];
    buf.push(b'"');
    if quote == b'"' {
      buf.extend_from_slice(inner);
    } else {
      let mut k = 0;
      while k < inner.len() {
        if inner[k] == b'\\' && k + 1 < inner.len() && inner[k + 1] == b'\'' {
          buf.push(b'\'');
          k += 2;
        } else if inner[k] == b'"' {
          buf.push(b'\\');
          buf.push(b'"');
          k += 1;
        } else {
          buf.push(inner[k]);
          k += 1;
        }
      }
    }
    buf.push(b'"');
  }

  fn render_key(&self, t: &Token, buf: &mut Vec<u8>) {
    if t.kind == Kind::String {
      self.render_string(t, buf);
    } else {
      buf.push(b'"');
      buf.extend_from_slice(&self.src[t.start..t.end]);
      buf.push(b'"');
    }
  }

  fn render_comment(&self, t: &Token, buf: &mut Vec<u8>) {
    if t.kind == Kind::Line {
      // text after the leading //
      let text = &self.src[t.start + 2..t.end];
      let non_slash = text.iter().take_while(|&&c| c == b'/').count();
      let start = if self.force_space_after_slashes && text.get(non_slash) == Some(&b' ') {
        non_slash + 1
      } else {
        non_slash
      };
      let rest = text[start..].trim_ascii_end();
      buf.extend_from_slice(b"//");
      buf.extend_from_slice(&text[..non_slash]); // extra slashes
      if !rest.is_empty() {
        if self.force_space_after_slashes {
          buf.push(b' ');
        }
        buf.extend_from_slice(rest);
      }
    } else {
      // block: /* + per-line trailing-trim (keep last line ws) + */
      let inner = &self.src[t.start + 2..t.end - 2];
      buf.extend_from_slice(b"/*");
      let s = String::from_utf8_lossy(inner);
      let lines: Vec<&str> = s.split('\n').collect();
      for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
          buf.push(b'\n');
        }
        if idx + 1 == lines.len() {
          buf.extend_from_slice(line.as_bytes()); // keep last line as-is
        } else {
          buf.extend_from_slice(line.trim_end().as_bytes());
        }
      }
      buf.extend_from_slice(b"*/");
    }
  }

  fn emit_comment(&mut self, idx: usize) {
    let mut buf = Vec::new();
    self.render_comment(&self.toks[idx], &mut buf);
    self.emit(&buf);
  }

  /// Place a comment on its own line at `level`, optionally preceded by a blank.
  fn emit_own_line_comment(&mut self, idx: usize, level: usize, blank: bool) {
    self.nl();
    if blank {
      self.nl();
    }
    self.indent(level);
    self.emit_comment(idx);
  }

  fn is_ignore(&self, idx: usize) -> bool {
    let t = &self.toks[idx];
    let text = if t.kind == Kind::Line {
      &self.src[t.start + 2..t.end]
    } else {
      &self.src[t.start + 2..t.end - 2]
    };
    text_has_dprint_ignore(text, self.ignore_text.as_bytes())
  }

  // ---- flat (single-line) rendering ----
  fn flat_value(&self, i: usize, buf: &mut Vec<u8>) -> usize {
    match self.toks[i].kind {
      Kind::String => {
        self.render_string(&self.toks[i], buf);
        i + 1
      }
      Kind::Word => {
        buf.extend_from_slice(&self.src[self.toks[i].start..self.toks[i].end]);
        i + 1
      }
      Kind::ObjOpen => self.flat_container(i, buf, false),
      Kind::ArrOpen => self.flat_container(i, buf, true),
      _ => i + 1,
    }
  }

  fn flat_container(&self, i: usize, buf: &mut Vec<u8>, is_array: bool) -> usize {
    // Walk tokens emitting commas positionally so a leading comment after a
    // comma (`[], /* c */ key`) keeps its place. `prev` tracks the previous
    // emitted token kind to pick the separator. Objects pad with `{ ` / ` }`
    // only when they hold a member; comment-only objects render `{/*a*/}`.
    buf.push(if is_array { b'[' } else { b'{' });
    let mut idx = i + 1;
    let mut prev = Prev::Open;
    loop {
      let k = self.toks[idx].kind;
      if is_close(k) {
        break;
      }
      if k == Kind::Comma {
        // drop a trailing comma (nothing but comments before the close)
        let mut j = idx + 1;
        while is_comment(self.toks[j].kind) {
          j += 1;
        }
        if !is_close(self.toks[j].kind) {
          buf.push(b',');
          prev = Prev::Comma;
        }
        idx += 1;
        continue;
      }
      if is_comment(k) {
        if prev != Prev::Open {
          buf.push(b' ');
        }
        self.render_comment(&self.toks[idx], buf);
        prev = Prev::Comment;
        idx += 1;
        continue;
      }
      // member
      match prev {
        Prev::Open => {
          if !is_array {
            buf.push(b' ');
          }
        }
        Prev::Comma | Prev::Comment => buf.push(b' '),
        Prev::Value => buf.extend_from_slice(b", "), // consecutive values (no source comma)
      }
      if is_array {
        idx = self.flat_value(idx, buf);
      } else {
        self.render_key(&self.toks[idx], buf);
        buf.extend_from_slice(b": ");
        idx = self.flat_value(self.object_value_index(idx), buf);
      }
      prev = Prev::Value;
    }
    if !is_array && prev == Prev::Value {
      buf.push(b' ');
    }
    buf.push(if is_array { b']' } else { b'}' });
    idx + 1
  }

  // ---- structure helpers ----
  fn object_value_index(&self, key_idx: usize) -> usize {
    let mut j = key_idx + 1;
    while is_comment(self.toks[j].kind) {
      j += 1;
    }
    // colon
    j += 1;
    while is_comment(self.toks[j].kind) {
      j += 1;
    }
    j
  }

  /// Index just after the value starting at `i`.
  fn skip_value_index(&self, i: usize) -> usize {
    match self.toks[i].kind {
      Kind::ObjOpen | Kind::ArrOpen => {
        let mut depth = 0;
        let mut idx = i;
        loop {
          let k = self.toks[idx].kind;
          if is_open(k) {
            depth += 1;
          } else if is_close(k) {
            depth -= 1;
            if depth == 0 {
              return idx + 1;
            }
          }
          idx += 1;
        }
      }
      _ => i + 1,
    }
  }

  /// Did the source put the first member on a later line than the open token?
  /// dprint keys this off the first element, so leading comments are skipped —
  /// but a newline before such a comment still counts (it precedes the element).
  fn originally_multiline(&self, open: usize) -> bool {
    let mut idx = open + 1;
    loop {
      if self.toks[idx].nl_before > 0 {
        return true;
      }
      if is_comment(self.toks[idx].kind) {
        idx += 1; // same-line leading comment; keep looking for the element
      } else {
        return false; // reached the first element / close on the open's line
      }
    }
  }

  /// Any direct-level comment that prevents a single-line rendering. A line
  /// comment (eats to EOL) or an own-line block comment always forces. A
  /// same-line block comment forces only when it sits immediately after the
  /// open of a multi-line container — there it is a leading comment of the
  /// first element (which is on a later line) and renders on its own line;
  /// anywhere else a same-line block stays inline and the container may still
  /// collapse.
  fn has_forcing_comment(&self, open: usize) -> bool {
    if !self.any_comments {
      return false;
    }
    let mut depth = 0;
    let mut idx = open;
    loop {
      let k = self.toks[idx].kind;
      if is_open(k) {
        depth += 1;
      } else if is_close(k) {
        depth -= 1;
        if depth == 0 {
          break;
        }
      } else if depth == 1 && is_comment(k) && (k == Kind::Line || self.toks[idx].nl_before > 0) {
        return true;
      }
      idx += 1;
    }
    let first = &self.toks[open + 1];
    first.kind == Kind::Block && first.nl_before == 0 && self.originally_multiline(open)
  }

  /// Will the value at `i` render multi-line regardless of width? Mirrors
  /// dprint-core's separated-values forcing.
  fn structurally_multiline(&self, i: usize) -> bool {
    match self.toks[i].kind {
      Kind::ObjOpen => {
        if self.toks[i + 1].kind == Kind::ObjClose {
          return self.originally_multiline(i);
        }
        if self.has_forcing_comment(i) {
          return true;
        }
        if !self.object_prefer_single_line && self.originally_multiline(i) {
          return true;
        }
        let mut idx = i + 1;
        loop {
          let k = self.toks[idx].kind;
          if k == Kind::ObjClose {
            return false;
          }
          if k == Kind::Comma || is_comment(k) {
            idx += 1;
            continue;
          }
          let val = self.object_value_index(idx);
          if self.structurally_multiline(val) {
            return true;
          }
          idx = self.skip_value_index(val);
        }
      }
      Kind::ArrOpen => {
        if self.toks[i + 1].kind == Kind::ArrClose {
          return false;
        }
        if self.has_forcing_comment(i) {
          return true;
        }
        if !self.array_prefer_single_line && self.originally_multiline(i) {
          return true;
        }
        let mut idx = i + 1;
        loop {
          let k = self.toks[idx].kind;
          if k == Kind::ArrClose {
            return false;
          }
          if k == Kind::Comma || is_comment(k) {
            idx += 1;
            continue;
          }
          if k == Kind::ObjOpen {
            // An object element that itself renders multi-line (a "breaker", or
            // an empty `{\n}`) makes the array span multiple lines too — either
            // inline mode or a full break — so the array is multi-line.
            if self.object_renders_multiline(idx) {
              return true;
            }
          } else if self.structurally_multiline(idx) {
            return true;
          }
          idx = self.skip_value_index(idx);
        }
      }
      _ => false,
    }
  }

  fn trailing_comma_for_last(&self, had_comma: bool) -> bool {
    match self.trailing_commas {
      TrailingCommaKind::Always => true,
      TrailingCommaKind::Never => false,
      TrailingCommaKind::Jsonc => self.is_jsonc,
      TrailingCommaKind::Maintain => had_comma,
    }
  }

  // ---- main value emit ----
  fn print_value(&mut self, i: usize, level: usize, trailing: usize) -> usize {
    let kind = self.toks[i].kind;
    if !is_open(kind) {
      let mut s = Vec::new();
      let next = self.flat_value(i, &mut s);
      self.emit(&s);
      return next;
    }

    let is_array = kind == Kind::ArrOpen;
    let empty_no_comments = is_close(self.toks[i + 1].kind);
    if empty_no_comments {
      let multiline = !is_array && self.originally_multiline(i);
      if !multiline {
        self.emit(if is_array { b"[]" } else { b"{}" });
        return i + 2;
      }
      self.emit(if is_array { b"[" } else { b"{" });
      self.nl();
      self.indent(level);
      self.emit(if is_array { b"]" } else { b"}" });
      return i + 2;
    }

    if is_array {
      return self.print_array(i, level, trailing);
    }

    // object: flat or full multi-line. Skip the flat render entirely when a
    // comment already forces multi-line; otherwise render it once and reuse it
    // for both the width check and the emit.
    if !self.has_forcing_comment(i) {
      let mut flat = Vec::new();
      let next = self.flat_value(i, &mut flat);
      let fits = self.col + char_width(&flat) + trailing <= self.line_width;
      if fits && !self.structurally_multiline(i) {
        self.emit(&flat);
        return next;
      }
    }

    let preserve_blanks = !self.object_prefer_single_line && self.originally_multiline(i);
    self.print_container_multiline(i, level, false, preserve_blanks)
  }

  /// Does the object value at `idx` render multi-line (empty `{\n}`, or a
  /// non-empty object forced by content/comments)?
  fn object_renders_multiline(&self, idx: usize) -> bool {
    if self.toks[idx + 1].kind == Kind::ObjClose {
      self.originally_multiline(idx)
    } else {
      self.structurally_multiline(idx) || self.has_forcing_comment(idx)
    }
  }

  /// Is `idx` a non-empty object that will render multi-line? Such an element
  /// can stay "inline multi-line" inside an otherwise single-line array.
  fn is_breaker(&self, idx: usize) -> bool {
    self.toks[idx].kind == Kind::ObjOpen && self.toks[idx + 1].kind != Kind::ObjClose && self.object_renders_multiline(idx)
  }

  /// Arrays have three layouts: flat, full multi-line (one element per line),
  /// and "inline" (single-line array whose object elements break internally at
  /// the array's own indent). dprint uses inline only when every multi-line
  /// element is a non-empty object and they form a contiguous suffix.
  fn print_array(&mut self, open: usize, level: usize, trailing: usize) -> usize {
    let mut force_full =
      self.has_forcing_comment(open) || (!self.array_prefer_single_line && self.originally_multiline(open));
    let mut breakers = Vec::new(); // (element_idx, is_breaker)
    let mut idx = open + 1;
    loop {
      let k = self.toks[idx].kind;
      if k == Kind::ArrClose {
        break;
      }
      if k == Kind::Comma || is_comment(k) {
        idx += 1;
        continue;
      }
      let breaker = self.is_breaker(idx);
      let el_multiline = is_open(k) && (self.structurally_multiline(idx) || self.has_forcing_comment(idx));
      if el_multiline && !breaker {
        // a multi-line array or empty object element can't stay inline
        force_full = true;
      }
      breakers.push(breaker);
      idx = self.skip_value_index(idx);
    }
    let after = idx + 1;

    if !force_full {
      if let Some(first) = breakers.iter().position(|&b| b) {
        if breakers[first..].iter().all(|&b| b) {
          self.render_array_inline(open, level);
          return after;
        }
        // a non-breaker follows a breaker → must fully break
      } else {
        // no breakers: flat if it fits, otherwise full break
        let mut s = Vec::new();
        self.flat_value(open, &mut s);
        if self.col + char_width(&s) + trailing <= self.line_width {
          self.emit(&s);
          return after;
        }
      }
    }

    let preserve_blanks = !self.array_prefer_single_line && self.originally_multiline(open);
    self.print_container_multiline(open, level, true, preserve_blanks)
  }

  fn render_array_inline(&mut self, open: usize, level: usize) {
    self.emit(b"[");
    let mut idx = open + 1;
    let mut prev = Prev::Open;
    loop {
      let k = self.toks[idx].kind;
      if k == Kind::ArrClose {
        break;
      }
      if k == Kind::Comma {
        // drop a trailing comma (single-line mode never keeps one)
        let mut j = idx + 1;
        while is_comment(self.toks[j].kind) {
          j += 1;
        }
        if !is_close(self.toks[j].kind) {
          self.emit(b",");
          prev = Prev::Comma;
        }
        idx += 1;
        continue;
      }
      if is_comment(k) {
        if prev != Prev::Open {
          self.space();
        }
        self.emit_comment(idx);
        prev = Prev::Comment;
        idx += 1;
        continue;
      }
      // element
      match prev {
        Prev::Open => {}
        Prev::Comma | Prev::Comment => self.space(),
        Prev::Value => self.emit(b", "),
      }
      if self.is_breaker(idx) {
        idx = self.print_value(idx, level, 0); // multi-line object at the array's own level
      } else {
        let mut s = Vec::new();
        idx = self.flat_value(idx, &mut s);
        self.emit(&s);
      }
      prev = Prev::Value;
    }
    self.emit(b"]");
  }

  fn print_container_multiline(&mut self, open: usize, level: usize, is_array: bool, preserve_blanks: bool) -> usize {
    self.emit(if is_array { b"[" } else { b"{" });
    let inner = level + 1;
    let mut idx = open + 1;

    // Only a LINE comment on the open token's line trails it (dprint's
    // gen_first_line_trailing_comment). A block comment there is a leading
    // comment of the first member and goes on its own line.
    if self.toks[idx].kind == Kind::Line && self.toks[idx].nl_before == 0 {
      self.space();
      self.emit_comment(idx);
      idx += 1;
    }

    let mut started = false;
    let mut pending_ignore = false;
    // Once a comment is placed on its own line as a statement, following
    // comments stay on their own lines even if they shared a source line with
    // it (dprint separates statement comments with newlines). A comment that
    // trails a value/comma/brace on the same line glues instead. At the start
    // of a container body we're already in statement position.
    let mut last_was_statement = true;
    loop {
      let k = self.toks[idx].kind;
      if is_close(k) {
        break;
      }
      if k == Kind::Comma {
        idx += 1; // stray separator (members consume their own comma)
        continue;
      }
      if is_comment(k) {
        let own_line = self.toks[idx].nl_before > 0 || last_was_statement;
        if own_line {
          let blank = started && preserve_blanks && self.toks[idx].nl_before >= 2;
          self.emit_own_line_comment(idx, inner, blank);
          started = true;
          last_was_statement = true;
          pending_ignore = self.is_ignore(idx);
        } else {
          self.space();
          self.emit_comment(idx);
          last_was_statement = false;
        }
        idx += 1;
        continue;
      }
      // member
      self.nl();
      if started && preserve_blanks && self.toks[idx].nl_before >= 2 {
        self.nl();
      }
      self.indent(inner);
      started = true;
      last_was_statement = false;
      idx = self.emit_member(idx, inner, is_array, pending_ignore);
      pending_ignore = false;
    }

    self.nl();
    self.indent(level);
    self.emit(if is_array { b"]" } else { b"}" });
    idx + 1
  }

  /// Emit one member (array element or object property) plus its comma, and the
  /// own-line comments that sit between the value and the comma. Returns the
  /// index positioned just after the consumed comma (or after the value when
  /// there is none) — trailing same-line comments are left to the caller.
  fn emit_member(&mut self, idx: usize, level: usize, is_array: bool, pending_ignore: bool) -> usize {
    let value_idx = if is_array { idx } else { self.object_value_index(idx) };
    let value_end = self.skip_value_index(value_idx);

    // analyse the gap after the value: own-line comments, then maybe a comma
    let mut j = value_end;
    while is_comment(self.toks[j].kind) {
      j += 1;
    }
    let src_comma = self.toks[j].kind == Kind::Comma;
    let comma_idx = j;
    // is this the last member? (next non-comment after the comma is a close)
    let mut k = if src_comma { comma_idx + 1 } else { comma_idx };
    while is_comment(self.toks[k].kind) {
      k += 1;
    }
    let is_last = is_close(self.toks[k].kind);
    let emit_comma = if is_last { self.trailing_comma_for_last(src_comma) } else { true };
    let trailing = if emit_comma { 1 } else { 0 };

    if pending_ignore {
      // emit the node verbatim from source (key/element start .. value end)
      let raw = self.src[self.toks[idx].start..self.toks[value_end - 1].end].to_vec();
      self.emit(&raw);
    } else if is_array {
      self.print_value(value_idx, level, trailing);
    } else {
      self.emit_object_property(idx, value_idx, level, trailing);
    }

    // own-line comments between the value and the comma (e.g. `1 // c\n ,`)
    if src_comma {
      let mut g = value_end;
      while g < comma_idx {
        if is_comment(self.toks[g].kind) {
          if self.toks[g].nl_before == 0 {
            self.space();
            self.emit_comment(g);
          } else {
            self.emit_own_line_comment(g, level, false);
          }
        }
        g += 1;
      }
      if emit_comma {
        if self.toks[comma_idx].nl_before == 0 {
          self.emit(b",");
        } else {
          self.nl();
          self.indent(level);
          self.emit(b",");
        }
      }
      comma_idx + 1
    } else {
      if emit_comma {
        self.emit(b",");
      }
      value_end
    }
  }

  fn emit_object_property(&mut self, key_idx: usize, value_idx: usize, level: usize, trailing: usize) {
    let mut kb = Vec::new();
    self.render_key(&self.toks[key_idx], &mut kb);
    self.emit(&kb);
    self.emit(b":");

    // comments between key and value (the colon is glued above)
    let mut gap = Vec::new();
    for g in key_idx + 1..value_idx {
      if is_comment(self.toks[g].kind) {
        gap.push(g);
      }
    }
    let value_own_line = gap.iter().any(|&g| self.toks[g].nl_before > 0)
      || gap.last().map(|&g| self.toks[g].kind == Kind::Line).unwrap_or(false);

    for &g in &gap {
      if self.toks[g].nl_before > 0 {
        self.emit_own_line_comment(g, level, false);
      } else {
        self.space();
        self.emit_comment(g);
      }
    }

    if value_own_line {
      self.nl();
      self.indent(level);
    } else {
      self.space();
    }
    self.print_value(value_idx, level, trailing);
  }

  // ---- root ----
  fn emit_root(&mut self) {
    let len = self.toks.len();
    let mut idx = 0;
    let mut started = false;

    // leading own-line comments
    while idx < len && is_comment(self.toks[idx].kind) {
      if started {
        self.nl();
        if self.toks[idx].nl_before >= 2 {
          self.nl();
        }
      }
      self.emit_comment(idx);
      started = true;
      idx += 1;
    }

    // root value
    if idx < len && !is_comment(self.toks[idx].kind) && !is_close(self.toks[idx].kind) && self.toks[idx].kind != Kind::Comma {
      if started {
        self.nl();
      }
      self.print_value(idx, 0, 0);
      idx = self.skip_value_index(idx);
    }

    // trailing comments after the value
    while idx < len && is_comment(self.toks[idx].kind) {
      if self.toks[idx].nl_before == 0 {
        self.space();
        self.emit_comment(idx);
      } else {
        self.emit_own_line_comment(idx, 0, self.toks[idx].nl_before >= 2);
      }
      idx += 1;
    }

    debug_assert_eq!(idx, len, "validate() should have rejected leftover tokens");
    // final newline
    if !self.out.is_empty() && self.out.last() != Some(&b'\n') {
      self.nl();
    }
  }
}

fn text_has_dprint_ignore(text: &[u8], searching: &[u8]) -> bool {
  if searching.is_empty() {
    return false;
  }
  let mut i = 0;
  while i + searching.len() <= text.len() {
    if &text[i..i + searching.len()] == searching {
      let before_ok = i == 0 || !text[i - 1].is_ascii_alphanumeric();
      let after = i + searching.len();
      let after_ok = after >= text.len() || !text[after].is_ascii_alphanumeric();
      if before_ok && after_ok {
        return true;
      }
    }
    i += 1;
  }
  false
}

/// Validates the token stream as JSON/JSONC grammar in a single pass, so the
/// formatter needs no separate parser. Word tokens are checked only by their
/// first character — enough to reject genuine garbage (`&`, a zero-width space)

fn resolve_newline(src: &[u8], kind: NewLineKind) -> &'static [u8] {
  match kind {
    NewLineKind::LineFeed => b"\n",
    NewLineKind::CarriageReturnLineFeed => b"\r\n",
    NewLineKind::Auto => {
      let mut found_slash_n = false;
      for c in src.iter().rev() {
        if found_slash_n {
          return if *c == b'\r' { b"\r\n" } else { b"\n" };
        }
        if *c == b'\n' {
          found_slash_n = true;
        }
      }
      b"\n"
    }
  }
}

/// Format JSON/JSONC bytes directly, no parser. Validates the grammar itself
/// and returns `Err` with a byte range + message on a syntax error. Works on

/// Build a printer and format the (already validated) token stream.
pub(super) fn format(src: &[u8], toks: &[Token], config: &Configuration, is_jsonc: bool) -> Vec<u8> {
  let mut p = Printer {
    src,
    toks,
    out: Vec::with_capacity(src.len()),
    col: 0,
    line_width: config.line_width as usize,
    indent_width: config.indent_width as usize,
    use_tabs: config.use_tabs,
    is_jsonc,
    any_comments: toks.iter().any(|t| is_comment(t.kind)),
    force_space_after_slashes: config.comment_line_force_space_after_slashes,
    ignore_text: &config.ignore_node_comment_text,
    trailing_commas: config.trailing_commas,
    array_prefer_single_line: config.array_prefer_single_line,
    object_prefer_single_line: config.object_prefer_single_line,
    newline: resolve_newline(src, config.new_line_kind),
  };
  p.emit_root();
  p.out
}
