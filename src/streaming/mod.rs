//! Format JSON/JSONC directly off a stream of bytes (issue #30).
//!
//! Single pass tokenize + recursive emit, no AST and no dprint-core IR/print
//! engine. Works on `&[u8]` so invalid UTF-8 inside strings passes through.
//!
//! Comments are handled positionally: each token records how many newlines
//! preceded it, which is enough to tell a same-line trailing comment from an
//! own-line one (and to place a comma that sits on its own line after a
//! comment). Line width uses unicode display width (UAX#11), matching
//! dprint-core, with an ASCII fast path.

use crate::configuration::Configuration;

mod printer;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
  ObjOpen,
  ObjClose,
  ArrOpen,
  ArrClose,
  Comma,
  Colon,
  String,
  Word, // number / true / false / null / bare word key
  Line, // // comment
  Block, // /* */ comment
}

#[derive(Clone, Copy)]
pub(crate) struct Token {
  pub(crate) kind: Kind,
  pub(crate) start: usize,
  pub(crate) end: usize,
  pub(crate) nl_before: u32,
}

/// A syntax error found by the streaming formatter (it validates as it goes
/// instead of delegating to a parser). `start`/`end` are byte offsets.
#[derive(Debug)]
pub struct StreamError {
  pub start: usize,
  pub end: usize,
  pub message: &'static str,
}

fn tokenize(src: &[u8]) -> Result<Vec<Token>, StreamError> {
  let mut toks = Vec::new();
  let mut i = 0;
  let mut nl_before = 0u32;
  let n = src.len();
  while i < n {
    let b = src[i];
    match b {
      b' ' | b'\t' | b'\r' => i += 1,
      b'\n' => {
        nl_before += 1;
        i += 1;
      }
      b'/' => {
        // comment
        if i + 1 >= n || !matches!(src[i + 1], b'/' | b'*') {
          return Err(StreamError { start: i, end: i + 1, message: "Unexpected token" });
        }
        match src[i + 1] {
          b'/' => {
            let start = i;
            i += 2;
            while i < n && src[i] != b'\n' {
              i += 1;
            }
            // exclude a trailing \r
            let mut end = i;
            if end > start + 2 && src[end - 1] == b'\r' {
              end -= 1;
            }
            toks.push(Token { kind: Kind::Line, start, end, nl_before });
          }
          _ => {
            let start = i;
            i += 2;
            loop {
              if i + 1 >= n {
                return Err(StreamError { start, end: n, message: "Unterminated comment block" });
              }
              if src[i] == b'*' && src[i + 1] == b'/' {
                i += 2;
                break;
              }
              i += 1;
            }
            toks.push(Token { kind: Kind::Block, start, end: i, nl_before });
          }
        }
        nl_before = 0;
      }
      b'{' | b'}' | b'[' | b']' | b',' | b':' => {
        let kind = match b {
          b'{' => Kind::ObjOpen,
          b'}' => Kind::ObjClose,
          b'[' => Kind::ArrOpen,
          b']' => Kind::ArrClose,
          b',' => Kind::Comma,
          _ => Kind::Colon,
        };
        toks.push(Token { kind, start: i, end: i + 1, nl_before });
        nl_before = 0;
        i += 1;
      }
      b'"' | b'\'' => {
        let quote = b;
        let start = i;
        i += 1;
        loop {
          if i >= n {
            return Err(StreamError { start, end: n, message: "Unterminated string literal" });
          }
          let c = src[i];
          if c == b'\\' {
            i += 2;
          } else if c == quote {
            i += 1;
            break;
          } else {
            i += 1;
          }
        }
        toks.push(Token { kind: Kind::String, start, end: i, nl_before });
        nl_before = 0;
      }
      _ => {
        let start = i;
        while i < n {
          let c = src[i];
          if c.is_ascii_whitespace() || matches!(c, b'{' | b'}' | b'[' | b']' | b',' | b':' | b'"' | b'\'' | b'/') {
            break;
          }
          i += 1;
        }
        if i == start {
          return Err(StreamError { start, end: start + 1, message: "Unexpected token" });
        }
        toks.push(Token { kind: Kind::Word, start, end: i, nl_before });
        nl_before = 0;
      }
    }
  }
  Ok(toks)
}

pub(crate) fn is_comment(k: Kind) -> bool {
  matches!(k, Kind::Line | Kind::Block)
}

pub(crate) fn is_close(k: Kind) -> bool {
  matches!(k, Kind::ObjClose | Kind::ArrClose)
}

pub(crate) fn is_open(k: Kind) -> bool {
  matches!(k, Kind::ObjOpen | Kind::ArrOpen)
}

struct Validator<'a> {
  toks: &'a [Token],
  src: &'a [u8],
  i: usize,
}

impl Validator<'_> {
  fn skip_comments(&mut self) {
    while self.i < self.toks.len() && is_comment(self.toks[self.i].kind) {
      self.i += 1;
    }
  }

  fn eof(&self) -> StreamError {
    StreamError {
      start: self.src.len(),
      end: self.src.len(),
      message: "Unexpected end of text",
    }
  }

  fn first_char(&self, t: &Token) -> Option<char> {
    std::str::from_utf8(&self.src[t.start..t.end]).ok().and_then(|s| s.chars().next())
  }

  fn check_word(&self, t: &Token) -> Result<(), StreamError> {
    let ok = matches!(self.first_char(t), Some(c) if c.is_alphanumeric() || matches!(c, '_' | '$' | '+' | '-' | '.'));
    if ok {
      Ok(())
    } else {
      let len = self.first_char(t).map(|c| c.len_utf8()).unwrap_or(1);
      Err(StreamError { start: t.start, end: t.start + len, message: "Unexpected token" })
    }
  }

  fn unexpected(&self, t: &Token) -> StreamError {
    StreamError { start: t.start, end: t.end, message: "Unexpected token" }
  }

  fn value(&mut self) -> Result<(), StreamError> {
    self.skip_comments();
    if self.i >= self.toks.len() {
      return Err(self.eof());
    }
    let t = self.toks[self.i];
    match t.kind {
      Kind::String => {
        self.i += 1;
        Ok(())
      }
      Kind::Word => {
        self.check_word(&t)?;
        self.i += 1;
        Ok(())
      }
      Kind::ObjOpen => self.object(),
      Kind::ArrOpen => self.array(),
      _ => Err(self.unexpected(&t)),
    }
  }

  fn object(&mut self) -> Result<(), StreamError> {
    self.i += 1; // {
    loop {
      self.skip_comments();
      if self.i >= self.toks.len() {
        return Err(self.eof());
      }
      let t = self.toks[self.i];
      if t.kind == Kind::ObjClose {
        self.i += 1;
        return Ok(());
      }
      // key
      match t.kind {
        Kind::String => {}
        Kind::Word => self.check_word(&t)?,
        _ => return Err(self.unexpected(&t)),
      }
      self.i += 1;
      // colon
      self.skip_comments();
      match self.toks.get(self.i) {
        Some(c) if c.kind == Kind::Colon => self.i += 1,
        Some(c) => return Err(self.unexpected(c)),
        None => return Err(self.eof()),
      }
      self.value()?;
      // comma or close
      self.skip_comments();
      match self.toks.get(self.i) {
        Some(c) if c.kind == Kind::Comma => self.i += 1,
        Some(c) if c.kind == Kind::ObjClose => {
          self.i += 1;
          return Ok(());
        }
        Some(c) => return Err(self.unexpected(c)),
        None => return Err(self.eof()),
      }
    }
  }

  fn array(&mut self) -> Result<(), StreamError> {
    self.i += 1; // [
    loop {
      self.skip_comments();
      if self.i >= self.toks.len() {
        return Err(self.eof());
      }
      if self.toks[self.i].kind == Kind::ArrClose {
        self.i += 1;
        return Ok(());
      }
      self.value()?;
      self.skip_comments();
      match self.toks.get(self.i) {
        Some(c) if c.kind == Kind::Comma => self.i += 1,
        Some(c) if c.kind == Kind::ArrClose => {
          self.i += 1;
          return Ok(());
        }
        Some(c) => return Err(self.unexpected(c)),
        None => return Err(self.eof()),
      }
    }
  }
}

fn validate(toks: &[Token], src: &[u8]) -> Result<(), StreamError> {
  let mut v = Validator { toks, src, i: 0 };
  v.skip_comments();
  if v.i >= toks.len() {
    return Ok(()); // empty file or comments only
  }
  v.value()?;
  v.skip_comments();
  if v.i < toks.len() {
    let t = &toks[v.i];
    return Err(StreamError {
      start: t.start,
      end: t.end,
      message: "Text cannot contain more than one JSON value",
    });
  }
  Ok(())
}

/// Format JSON/JSONC bytes directly, no parser. Validates the grammar itself
/// and returns `Err` with a byte range + message on a syntax error. Works on
/// arbitrary bytes — invalid UTF-8 inside strings is preserved.
pub fn format_streaming(src: &[u8], config: &Configuration, is_jsonc: bool) -> Result<Vec<u8>, StreamError> {
  let toks = tokenize(src)?;
  validate(&toks, src)?;
  Ok(printer::format(src, &toks, config, is_jsonc))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::configuration::{ConfigurationBuilder, resolve_config};
  use dprint_core::configuration::{ConfigKeyMap, GlobalConfiguration};

  fn default_config() -> Configuration {
    let _ = ConfigurationBuilder::new();
    resolve_config(ConfigKeyMap::new(), &GlobalConfiguration::default()).config
  }

  #[test]
  fn formats_invalid_utf8_in_strings() {
    // issue #5: a string holding bytes that are not valid UTF-8. Formatting it
    // off a byte stream must not error or mangle the bytes (a parser that
    // assumes UTF-8 cannot do this).
    let mut input = b"{\n\"a\":\"".to_vec();
    input.extend_from_slice(&[0xed, 0xa0, 0xb4]); // lone surrogate, invalid UTF-8
    input.extend_from_slice(b"\"}");
    let out = format_streaming(&input, &default_config(), false).expect("should format");
    // bytes preserved verbatim inside the string (input was written multi-line)
    let mut expected = b"{\n  \"a\": \"".to_vec();
    expected.extend_from_slice(&[0xed, 0xa0, 0xb4]);
    expected.extend_from_slice(b"\"\n}\n");
    assert_eq!(out, expected);
  }

  #[test]
  fn reports_syntax_error_without_a_parser() {
    let err = format_streaming(b"{ &*&* }", &default_config(), false).err().unwrap();
    assert_eq!(err.message, "Unexpected token");
    assert_eq!((err.start, err.end), (2, 3)); // the `&`
  }
}
