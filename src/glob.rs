//! Minimal glob matching for the `jsonTrailingCommaFiles` patterns.
//!
//! A regex based matcher would bloat the Wasm plugin, so the common syntax is
//! implemented directly: `*`, `?`, `**`, `[...]` and `{a,b}` (expanded ahead of time).

use std::ops::Range;

/// The most patterns a single pattern may expand to.
const MAX_BRACE_EXPANSIONS: usize = 1_000;
/// The deepest braces may be nested.
const MAX_BRACE_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BraceExpansionError {
  #[error("expands to more than {MAX_BRACE_EXPANSIONS} patterns")]
  TooManyExpansions,
  #[error("has braces nested more than {MAX_BRACE_DEPTH} levels deep")]
  TooDeeplyNested,
}

/// Expands the brace alternatives in a pattern (ex. `{j,t}sconfig.json` becomes
/// `jsconfig.json` and `tsconfig.json`).
///
/// Like bash, braces without a comma between them (ex. `{a}`) and unclosed braces
/// are kept literally. Braces in a character class (ex. `[{]`) are also literal.
pub fn expand_braces(pattern: &str) -> Result<Vec<String>, BraceExpansionError> {
  let closing_indexes = find_closing_indexes(pattern)?;
  expand_braces_in_range(pattern, 0..pattern.len(), &closing_indexes)
}

/// Gets if the pattern matches the end of the path at a path segment boundary
/// (ex. `.vscode/*.json` matches `/project/.vscode/settings.json`).
///
/// Either slash may be used as a separator in the pattern and path. The pattern's
/// braces must already be expanded (see `expand_braces`).
pub fn matches_path_end(pattern: &str, path: &str) -> bool {
  let pattern = pattern.trim_start_matches(is_separator);
  if !is_glob(pattern) {
    return matches_literal_path_end(pattern, path);
  }

  if pattern.split(is_separator).any(|segment| segment == "**") {
    matches_globstar_path_end(pattern, path)
  } else {
    // without a globstar only the trailing path segments can match, so no allocation is necessary
    let mut path_segments = path.rsplit(is_separator).filter(|s| !s.is_empty());
    pattern
      .rsplit(is_separator)
      .filter(|s| !s.is_empty())
      .all(|pattern_segment| {
        path_segments
          .next()
          .is_some_and(|path_segment| matches_segment(pattern_segment, path_segment))
      })
  }
}

/// Finds the byte index of the closing brace or bracket for each opening brace
/// or character class, which allows skipping over them in linear time.
fn find_closing_indexes(pattern: &str) -> Result<Vec<Option<usize>>, BraceExpansionError> {
  let bytes = pattern.as_bytes();
  let mut closing_indexes = vec![None; bytes.len()];
  let mut open_braces = Vec::new();
  let mut may_close_class = true;
  let mut i = 0;
  while i < bytes.len() {
    match bytes[i] {
      b'[' if may_close_class => match find_char_class_end(&pattern[i + 1..]) {
        Some(end) => {
          let end = i + 1 + end;
          closing_indexes[i] = Some(end);
          i = end + 1;
          continue;
        }
        // no later class can be closed either
        None => may_close_class = false,
      },
      b'{' => open_braces.push(i),
      b'}' => {
        if let Some(open) = open_braces.pop() {
          closing_indexes[open] = Some(i);
        }
      }
      _ => {}
    }
    i += 1;
  }

  // unclosed braces are literal, so only the closed groups count toward the depth
  let mut open_group_ends = Vec::new();
  for (i, closing_index) in closing_indexes.iter().enumerate() {
    if bytes[i] == b'{'
      && let Some(close) = closing_index
    {
      while open_group_ends.last().is_some_and(|end| end < &i) {
        open_group_ends.pop();
      }
      open_group_ends.push(*close);
      if open_group_ends.len() > MAX_BRACE_DEPTH {
        return Err(BraceExpansionError::TooDeeplyNested);
      }
    }
  }
  Ok(closing_indexes)
}

fn expand_braces_in_range(
  pattern: &str,
  range: Range<usize>,
  closing_indexes: &[Option<usize>],
) -> Result<Vec<String>, BraceExpansionError> {
  let bytes = pattern.as_bytes();
  let mut results = vec![String::new()];
  let mut literal_start = range.start;
  let mut i = range.start;
  while i < range.end {
    let Some(close) = closing_indexes[i] else {
      i += 1;
      continue;
    };
    if bytes[i] == b'[' {
      i = close + 1;
      continue;
    }

    let alternative_ranges = split_alternatives(pattern, i + 1..close, closing_indexes);
    if alternative_ranges.len() == 1 {
      // the braces are literal, but groups within them are still expanded
      i += 1;
      continue;
    }

    let mut alternatives = Vec::new();
    for alternative_range in alternative_ranges {
      alternatives.extend(expand_braces_in_range(pattern, alternative_range, closing_indexes)?);
    }
    if results.len() * alternatives.len() > MAX_BRACE_EXPANSIONS {
      return Err(BraceExpansionError::TooManyExpansions);
    }
    let prefix = &pattern[literal_start..i];
    results = results
      .iter()
      .flat_map(|result| {
        alternatives
          .iter()
          .map(move |alternative| format!("{result}{prefix}{alternative}"))
      })
      .collect();
    i = close + 1;
    literal_start = i;
  }

  let suffix = &pattern[literal_start..range.end];
  for result in &mut results {
    result.push_str(suffix);
  }
  Ok(results)
}

/// Gets the ranges between the commas of a brace group that aren't within a
/// nested brace group or character class.
fn split_alternatives(pattern: &str, range: Range<usize>, closing_indexes: &[Option<usize>]) -> Vec<Range<usize>> {
  let bytes = pattern.as_bytes();
  let mut ranges = Vec::new();
  let mut start = range.start;
  let mut i = range.start;
  while i < range.end {
    if let Some(close) = closing_indexes[i] {
      i = close + 1;
      continue;
    }
    if bytes[i] == b',' {
      ranges.push(start..i);
      start = i + 1;
    }
    i += 1;
  }
  ranges.push(start..range.end);
  ranges
}

fn is_glob(pattern: &str) -> bool {
  pattern.contains(['*', '?', '['])
}

fn is_separator(c: char) -> bool {
  c == '/' || c == '\\'
}

fn matches_literal_path_end(pattern: &str, path: &str) -> bool {
  let (pattern, path) = (pattern.as_bytes(), path.as_bytes());
  let Some(start) = path.len().checked_sub(pattern.len()) else {
    return false;
  };
  let is_separator_byte = |b: u8| b == b'/' || b == b'\\';
  (start == 0 || is_separator_byte(path[start - 1]))
    && path[start..].iter().zip(pattern).all(|(&path_byte, &pattern_byte)| {
      path_byte == pattern_byte || (is_separator_byte(path_byte) && is_separator_byte(pattern_byte))
    })
}

/// Matches a pattern containing a globstar (`**`) against the end of the path.
///
/// This works like `matches_segment`, but with whole segments instead of chars,
/// globstars instead of stars, and an implicit leading globstar.
fn matches_globstar_path_end(pattern: &str, path: &str) -> bool {
  let mut pattern_segments = pattern
    .split(is_separator)
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>();
  // a trailing globstar must match at least one segment (ex. `a/**` doesn't match a file named `a`)
  if pattern_segments.last() == Some(&"**") {
    pattern_segments.insert(pattern_segments.len() - 1, "*");
  }
  let path_segments = path.split(is_separator).filter(|s| !s.is_empty()).collect::<Vec<_>>();

  let mut pattern_index = 0;
  let mut path_index = 0;
  // the pattern index after the last globstar and the path index that globstar would consume next
  let mut backtrack = (0, 0);
  loop {
    if pattern_segments.get(pattern_index) == Some(&"**") {
      pattern_index += 1;
      backtrack = (pattern_index, path_index);
      continue;
    }

    match (pattern_segments.get(pattern_index), path_segments.get(path_index)) {
      (Some(pattern_segment), Some(path_segment)) if matches_segment(pattern_segment, path_segment) => {
        pattern_index += 1;
        path_index += 1;
        continue;
      }
      (None, None) => return true,
      _ => {}
    }

    // mismatch, so have the last globstar consume one more segment
    let (star_pattern_index, star_path_index) = backtrack;
    if star_path_index >= path_segments.len() {
      return false;
    }
    pattern_index = star_pattern_index;
    path_index = star_path_index + 1;
    backtrack = (pattern_index, path_index);
  }
}

/// Matches a single path segment against a pattern segment.
fn matches_segment(pattern: &str, text: &str) -> bool {
  let mut pattern_rest = pattern;
  let mut text_rest = text;
  // the pattern after the last star and the text that star would consume next
  let mut backtrack: Option<(&str, &str)> = None;
  loop {
    if let Some(after_star) = pattern_rest.strip_prefix('*') {
      pattern_rest = after_star.trim_start_matches('*');
      backtrack = Some((pattern_rest, text_rest));
      continue;
    }

    match text_rest.chars().next() {
      Some(c) => {
        if let Some(next_pattern) = match_char(pattern_rest, c) {
          pattern_rest = next_pattern;
          text_rest = &text_rest[c.len_utf8()..];
          continue;
        }
      }
      None if pattern_rest.is_empty() => return true,
      None => {}
    }

    // mismatch, so have the last star consume one more char
    let Some((star_pattern, star_text)) = backtrack else {
      return false;
    };
    let Some(c) = star_text.chars().next() else {
      return false;
    };
    pattern_rest = star_pattern;
    text_rest = &star_text[c.len_utf8()..];
    backtrack = Some((pattern_rest, text_rest));
  }
}

/// Matches the char against the start of the pattern, returning the rest of the
/// pattern when it matches.
fn match_char(pattern: &str, c: char) -> Option<&str> {
  let mut chars = pattern.chars();
  match chars.next()? {
    '?' => Some(chars.as_str()),
    '[' => {
      let class = chars.as_str();
      match find_char_class_end(class) {
        Some(end) => matches_char_class(&class[..end], c).then_some(&class[end + 1..]),
        // an unclosed bracket is literal
        None => (c == '[').then_some(class),
      }
    }
    pattern_char => (pattern_char == c).then_some(chars.as_str()),
  }
}

/// Matches the char against the text between a character class's brackets
/// (ex. `abc`, `a-z` or `!abc`).
fn matches_char_class(class: &str, c: char) -> bool {
  let (negated, class) = match class.strip_prefix(['!', '^']) {
    Some(rest) => (true, rest),
    None => (false, class),
  };
  let mut chars = class.chars();
  let mut matched = false;
  while let Some(start) = chars.next() {
    let mut end = start;
    let mut lookahead = chars.clone();
    // a dash at the end is literal
    if lookahead.next() == Some('-')
      && let Some(range_end) = lookahead.next()
    {
      end = range_end;
      chars = lookahead;
    }
    matched |= start <= c && c <= end;
  }
  matched != negated
}

/// Gets the byte index of the bracket closing a character class, where the text
/// is after the opening bracket. A closing bracket first in the class is literal
/// (ex. `[]]` or `[!]]`).
fn find_char_class_end(class: &str) -> Option<usize> {
  let body_start = if class.starts_with(['!', '^']) { 1 } else { 0 };
  let first_char = class[body_start..].chars().next()?;
  let search_start = body_start + first_char.len_utf8();
  class[search_start..].find(']').map(|index| search_start + index)
}

#[cfg(test)]
mod test {
  use super::*;

  #[test]
  fn literal() {
    assert!(matches_path_end("tsconfig.json", "/tsconfig.json"));
    assert!(matches_path_end("tsconfig.json", "tsconfig.json"));
    assert!(matches_path_end("tsconfig.json", "/a/b/tsconfig.json"));
    assert!(matches_path_end("tsconfig.json", "C:\\a\\tsconfig.json"));
    assert!(!matches_path_end("tsconfig.json", "/a/mytsconfig.json"));
    assert!(!matches_path_end("tsconfig.json", "/a/tsconfig.json5"));
    assert!(matches_path_end(".vscode/settings.json", "/a/.vscode/settings.json"));
    assert!(matches_path_end(
      ".vscode/settings.json",
      "C:\\a\\.vscode\\settings.json"
    ));
    assert!(!matches_path_end(".vscode/settings.json", "/a/vscode/settings.json"));
    assert!(!matches_path_end(".vscode/settings.json", "settings.json"));
    // leading and backslash separators in the pattern
    assert!(matches_path_end("/tsconfig.json", "/a/tsconfig.json"));
    assert!(matches_path_end("\\.vscode\\settings.json", "/a/.vscode/settings.json"));
    assert!(!matches_path_end("/tsconfig.json", "/a/mytsconfig.json"));
  }

  #[test]
  fn star() {
    assert!(matches_path_end("tsconfig*.json", "/a/tsconfig.json"));
    assert!(matches_path_end("tsconfig*.json", "/a/tsconfig.lib.prod.json"));
    assert!(!matches_path_end("tsconfig*.json", "/a/tsconfig.jsonc"));
    assert!(!matches_path_end("tsconfig*.json", "/tsconfig/a.json"));
    assert!(matches_path_end("*", "/a/b"));
    assert!(!matches_path_end("*", "/"));
    assert!(matches_path_end("*.json", "/a/b.json"));
    assert!(matches_path_end("*.json", "/a/.json"));
    assert!(matches_path_end("**.json", "/a/b.json"));
    assert!(matches_path_end("a*b*c", "/abbbcbc"));
    assert!(!matches_path_end("a*b*c", "/abbbcb"));
    assert!(matches_path_end(".vscode/*.json", "/a/.vscode/tasks.json"));
    assert!(matches_path_end(".vscode/*.json", "C:\\a\\.vscode\\tasks.json"));
    assert!(matches_path_end("\\.vscode\\*.json", "/a/.vscode/tasks.json"));
    assert!(!matches_path_end(".vscode/*.json", "/a/.vscode/sub/tasks.json"));
    assert!(matches_path_end("*/settings.json", "/a/settings.json"));
    // the root isn't a segment
    assert!(!matches_path_end("*/settings.json", "/settings.json"));
  }

  #[test]
  fn question_mark() {
    assert!(matches_path_end("?sconfig.json", "/a/jsconfig.json"));
    assert!(matches_path_end("?sconfig.json", "/a/\u{1F600}sconfig.json"));
    assert!(!matches_path_end("?sconfig.json", "/a/sconfig.json"));
    assert!(!matches_path_end("a?b", "/a/b"));
  }

  #[test]
  fn char_class() {
    assert!(matches_path_end("[jt]sconfig.json", "/jsconfig.json"));
    assert!(matches_path_end("[jt]sconfig.json", "/tsconfig.json"));
    assert!(!matches_path_end("[jt]sconfig.json", "/psconfig.json"));
    assert!(matches_path_end("[!jt]sconfig.json", "/psconfig.json"));
    assert!(!matches_path_end("[^jt]sconfig.json", "/tsconfig.json"));
    assert!(matches_path_end("file[0-9].json", "/file5.json"));
    assert!(!matches_path_end("file[0-9].json", "/filea.json"));
    assert!(matches_path_end("file[\u{E9}-\u{EA}].json", "/file\u{EA}.json"));
    assert!(matches_path_end("file[a-].json", "/file-.json"));
    assert!(matches_path_end("file[]].json", "/file].json"));
    assert!(matches_path_end("file[]-].json", "/file-.json"));
    assert!(matches_path_end("file[!]].json", "/filea.json"));
    assert!(!matches_path_end("file[!]].json", "/file].json"));
    assert!(matches_path_end("[[]id].json", "/[id].json"));
    // unclosed is literal
    assert!(matches_path_end("file[a.json", "/file[a.json"));
    assert!(matches_path_end("*[a.json", "/file[a.json"));
    assert!(matches_path_end("file[].json", "/file[].json"));
  }

  #[test]
  fn globstar() {
    assert!(matches_path_end("**/settings.json", "/settings.json"));
    assert!(matches_path_end("**/settings.json", "/a/b/settings.json"));
    assert!(matches_path_end(".vscode/**/*.json", "/a/.vscode/settings.json"));
    assert!(matches_path_end(".vscode/**/*.json", "/a/.vscode/b/c/settings.json"));
    assert!(matches_path_end(
      ".vscode/**/*.json",
      "C:\\a\\.vscode\\b\\settings.json"
    ));
    assert!(!matches_path_end(".vscode/**/*.json", "/a/vscode/b/settings.json"));
    assert!(!matches_path_end(".vscode/**/*.json", "/a/.vscode/b/settings.jsonc"));
    assert!(matches_path_end("a/**/**/b", "/x/a/b"));
    assert!(matches_path_end("a/**/b/**/c", "/x/a/b/b/y/c"));
    assert!(!matches_path_end("a/**/b", "/x/a/c"));
    assert!(matches_path_end("**", "/a"));
    assert!(matches_path_end("a/**", "/x/a/b/c"));
    // a trailing globstar must match a segment
    assert!(!matches_path_end("a/**", "/x/a"));
  }

  #[test]
  fn globstar_does_not_backtrack_exponentially() {
    let path = format!("/{}", ["a"; 60].join("/"));
    assert!(!matches_path_end("**/**/**/**/**/**/**/**/**/**/x", &path));
    assert!(!matches_path_end("**/a/**/a/**/a/**/a/**/a/**/a/**/x", &path));
    assert!(matches_path_end("**/a/**/a/**/a/**/a/**/a/**/a/**/a", &path));
  }

  #[test]
  fn braces() {
    assert_eq!(expand_braces("tsconfig.json").unwrap(), vec!["tsconfig.json"]);
    assert_eq!(
      expand_braces("{j,t}sconfig.json").unwrap(),
      vec!["jsconfig.json", "tsconfig.json"]
    );
    assert_eq!(expand_braces("{a,b}/{c,d}").unwrap(), vec!["a/c", "a/d", "b/c", "b/d"]);
    assert_eq!(expand_braces("x{a,b{c,d}}y").unwrap(), vec!["xay", "xbcy", "xbdy"]);
    assert_eq!(
      expand_braces("tsconfig{,.lib}.json").unwrap(),
      vec!["tsconfig.json", "tsconfig.lib.json"]
    );
    // literal braces
    assert_eq!(expand_braces("{a}").unwrap(), vec!["{a}"]);
    assert_eq!(expand_braces("a{}b").unwrap(), vec!["a{}b"]);
    assert_eq!(expand_braces("{a{b,c}}").unwrap(), vec!["{ab}", "{ac}"]);
    assert_eq!(expand_braces("{a,b").unwrap(), vec!["{a,b"]);
    assert_eq!(expand_braces("{a{b,c}").unwrap(), vec!["{ab", "{ac"]);
    assert_eq!(expand_braces("a}{b,c}").unwrap(), vec!["a}b", "a}c"]);
    // braces and commas in a character class
    assert_eq!(expand_braces("[{}]").unwrap(), vec!["[{}]"]);
    assert_eq!(expand_braces("{[,],b}").unwrap(), vec!["[,]", "b"]);
    assert_eq!(expand_braces("{a,[}]}").unwrap(), vec!["a", "[}]"]);
    assert_eq!(expand_braces("[a{b,c}").unwrap(), vec!["[ab", "[ac"]);
  }

  #[test]
  fn braces_limits() {
    assert_eq!(expand_braces(&"{a,b}".repeat(9)).unwrap().len(), 512);
    assert_eq!(
      expand_braces(&"{a,b}".repeat(10)),
      Err(BraceExpansionError::TooManyExpansions)
    );
    assert_eq!(
      expand_braces(&format!("{}{}", "{a,".repeat(16), "}".repeat(16)))
        .unwrap()
        .len(),
      17
    );
    assert_eq!(
      expand_braces(&format!("{}{}", "{a,".repeat(17), "}".repeat(17))),
      Err(BraceExpansionError::TooDeeplyNested)
    );
    assert_eq!(expand_braces(&"{".repeat(100_000)).unwrap().len(), 1);
    assert_eq!(expand_braces(&"[".repeat(100_000)).unwrap().len(), 1);
  }
}
