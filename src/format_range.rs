use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::ops::RangeInclusive;
use std::path::Path;

use dprint_core::configuration::NewLineKind;
use dprint_core::configuration::resolve_new_line_kind;
use jsonc_parser::ast::Value;
use jsonc_parser::common::Ranged;
use jsonc_parser::tokens::Token;

use super::configuration::Configuration;
use super::format_text::FormatError;
use super::format_text::format_text;
use super::format_text::format_text_inner;
use super::format_text::parse;
use super::format_text::strip_bom;

/// Formats only the part of the text within the provided byte range.
///
/// The range is widened to the lines of the object properties or array elements it touches in the
/// innermost object or array that contains it, and the text outside of those is left as it was.
/// When the line breaks around those members change once formatted, the member holding their
/// object or array is formatted instead, and so on out to the whole file. A range that reaches
/// the brackets of the root value formats the whole file and one outside of the root value
/// formats nothing.
///
/// The `package.json` conventions only reorder the properties of an object when all of them are
/// touched, and only reorder the top level along with the whole file, so that nothing outside of
/// what's touched moves.
pub fn format_text_range(
  path: &Path,
  text: &str,
  range: Range<usize>,
  config: &Configuration,
) -> Result<Option<String>, FormatError> {
  let body = strip_bom(text);
  let bom_len = text.len() - body.len();
  let end = range.end.saturating_sub(bom_len).min(body.len());
  let range = range.start.saturating_sub(bom_len).min(end)..end;
  if range.start == 0 && range.end == body.len() {
    return format_text(path, text, config);
  }

  let parse_result = parse(body)?;
  let Some(root) = &parse_result.value else {
    return format_text(path, text, config);
  };
  let levels = match find_levels(root, &range) {
    Target::File => return format_text(path, text, config),
    Target::Nothing => return Ok(None),
    Target::Levels(levels) => levels,
  };

  let formatted = format_text_inner(path, body, config, Some(&sections_to_reorder(&levels, &range)))?;
  let formatted_parse_result = parse(&formatted)?;
  let formatted_root = formatted_parse_result
    .value
    .as_ref()
    .expect("formatted text should have a value");
  let commas = parse_result
    .tokens
    .iter()
    .flatten()
    .filter(|token| matches!(token.token, Token::Comma))
    .map(|token| token.range.start)
    .collect::<Vec<_>>();
  let result = match find_replacement(&levels, body, &commas, formatted_root, &formatted) {
    Some(replacement) => format!(
      "{}{}{}",
      &text[..bom_len + replacement.original.start],
      with_file_new_lines(&formatted[replacement.formatted], body),
      &text[bom_len + replacement.original.end..]
    ),
    // the layout of the root changes around the range, so the whole file needs formatting
    None => formatted,
  };
  if result == text { Ok(None) } else { Ok(Some(result)) }
}

enum Target<'a> {
  /// The range reaches the root value's brackets or the root isn't an object or array.
  File,
  /// The range is outside the root value or only touches whitespace or comments between members.
  Nothing,
  /// Each object or array from the root to the innermost one containing the range.
  Levels(Vec<Level<'a>>),
}

/// An object or array along with the members of it to format.
struct Level<'a> {
  container: &'a Value<'a>,
  members: Vec<Member<'a>>,
  /// Indexes of the members to format. For all but the innermost level, this is only the member
  /// holding the next level.
  indexes: RangeInclusive<usize>,
}

struct Member<'a> {
  step: Step<'a>,
  range: Range<usize>,
  value: &'a Value<'a>,
}

/// Identifies a member by what it is rather than where it is, so it can be found again in the
/// formatted text even after its properties were reordered.
#[derive(Clone, Copy, PartialEq)]
enum Step<'a> {
  /// Property name along with how many properties with the same name come before it.
  Prop(&'a str, usize),
  Element(usize),
}

struct Replacement {
  original: Range<usize>,
  formatted: Range<usize>,
}

fn find_levels<'a>(root: &'a Value<'a>, range: &Range<usize>) -> Target<'a> {
  if range.end <= root.start() || range.start >= root.end() {
    return Target::Nothing;
  }
  if !is_within_brackets(root, range) {
    return Target::File;
  }
  let mut levels = Vec::new();
  let mut container = root;
  loop {
    let Some(members) = members(container) else {
      return Target::File;
    };
    let mut touched = members
      .iter()
      .enumerate()
      .filter(|(_, member)| touches(&member.range, range))
      .map(|(index, _)| index);
    let Some(first) = touched.next() else {
      return Target::Nothing;
    };
    let last = touched.next_back().unwrap_or(first);
    let inner = Some(members[first].value).filter(|value| first == last && is_within_brackets(value, range));
    levels.push(Level {
      container,
      members,
      indexes: first..=last,
    });
    match inner {
      Some(inner) => container = inner,
      None => return Target::Levels(levels),
    }
  }
}

/// The indexes of the root's properties that the `package.json` conventions may reorder the
/// properties of, which are the ones whose values are entirely within what's touched.
fn sections_to_reorder(levels: &[Level], range: &Range<usize>) -> Vec<usize> {
  let root = &levels[0];
  match levels.get(1) {
    None => root
      .indexes
      .clone()
      .filter(|index| {
        let member = &root.members[*index].range;
        range.start <= member.start && range.end >= member.end
      })
      .collect(),
    Some(section) if levels.len() == 2 && section.indexes == (0..=section.members.len() - 1) => {
      vec![*root.indexes.start()]
    }
    Some(_) => Vec::new(),
  }
}

/// Finds the text to replace in the original and the formatted text to replace it with, starting
/// with the innermost level and moving out towards the root.
fn find_replacement(
  levels: &[Level],
  text: &str,
  commas: &[usize],
  formatted_root: &Value,
  formatted_text: &str,
) -> Option<Replacement> {
  let mut formatted_levels = Vec::with_capacity(levels.len());
  let mut formatted_container = formatted_root;
  for level in levels {
    let formatted_members = members(formatted_container)?;
    let step = level.members[*level.indexes.start()].step;
    let next_container = formatted_members.iter().find(|member| member.step == step)?.value;
    formatted_levels.push((formatted_container, formatted_members));
    formatted_container = next_container;
  }

  levels
    .iter()
    .zip(&formatted_levels)
    .rev()
    .find_map(|(level, (formatted_container, formatted_members))| {
      let mut formatted_indexes = level.members[level.indexes.clone()]
        .iter()
        .map(|member| {
          formatted_members
            .iter()
            .position(|formatted| formatted.step == member.step)
        })
        .collect::<Option<Vec<_>>>()?;
      // the members may be reordered among themselves (ex. the properties of a `package.json`
      // section that are all touched), but not with ones that aren't being replaced
      formatted_indexes.sort_unstable();
      let stays_together = formatted_indexes.windows(2).all(|pair| pair[1] == pair[0] + 1);
      if !stays_together {
        return None;
      }
      let (first, last) = (*level.indexes.start(), *level.indexes.end());
      let (formatted_first, formatted_last) = (formatted_indexes[0], formatted_indexes[formatted_indexes.len() - 1]);
      let original = Gaps::new(level.container, &level.members, first..=last);
      let formatted = Gaps::new(formatted_container, formatted_members, formatted_first..=formatted_last);
      // the text around the replaced text is kept, so the line breaks there need to already be
      // what the formatter wants or the result would be half single and half multi-line
      if original.line_breaks(text) != formatted.line_breaks(formatted_text) {
        return None;
      }
      let edges = original.edges(text, commas);
      // the replaced text holds the separator after the last member and, when it doesn't start at
      // the first member's line, the one before the first, so those need to stay (ex. a member
      // that's moved to the end loses its comma)
      let keeps_next = (last == level.members.len() - 1) == (formatted_last == formatted_members.len() - 1);
      let keeps_previous = edges.line_start || (first == 0) == (formatted_first == 0);
      (keeps_next && keeps_previous).then(|| Replacement {
        original: original.replaced_range(text, edges),
        formatted: formatted.replaced_range(formatted_text, edges),
      })
    })
}

fn members<'a>(value: &'a Value<'a>) -> Option<Vec<Member<'a>>> {
  match value {
    Value::Object(object) => {
      let mut occurrences = HashMap::with_capacity(object.properties.len());
      Some(
        object
          .properties
          .iter()
          .map(|prop| {
            let name = prop.name.as_str();
            let occurrence = occurrences.entry(name).or_insert(0);
            let step = Step::Prop(name, *occurrence);
            *occurrence += 1;
            Member {
              step,
              range: prop.range.start..prop.range.end,
              value: &prop.value,
            }
          })
          .collect(),
      )
    }
    Value::Array(array) => Some(
      array
        .elements
        .iter()
        .enumerate()
        .map(|(index, element)| Member {
          step: Step::Element(index),
          range: element.start()..element.end(),
          value: element,
        })
        .collect(),
    ),
    _ => None,
  }
}

fn is_within_brackets(value: &Value, range: &Range<usize>) -> bool {
  matches!(value, Value::Object(_) | Value::Array(_)) && range.start > value.start() && range.end < value.end()
}

fn touches(member: &Range<usize>, range: &Range<usize>) -> bool {
  if range.is_empty() {
    member.start <= range.start && range.start <= member.end
  } else {
    range.start < member.end && range.end > member.start
  }
}

/// The text between a run of members and what's on either side of them.
struct Gaps {
  /// From the end of the previous member or the opening bracket to the first member.
  before: Range<usize>,
  /// From the last member to the start of the next member or the closing bracket.
  after: Range<usize>,
}

/// Where the replaced text starts and ends.
#[derive(Clone, Copy)]
struct Edges {
  /// Starts at the beginning of the first member's line rather than after what's before it.
  line_start: bool,
  /// Ends at the end of the last member's line rather than at what's after it.
  line_end: bool,
}

impl Gaps {
  fn new(container: &Value, members: &[Member], indexes: RangeInclusive<usize>) -> Self {
    let before_start = match *indexes.start() {
      0 => container.start() + 1, // after the opening bracket
      index => members[index - 1].range.end,
    };
    let after_end = members
      .get(indexes.end() + 1)
      .map(|member| member.range.start)
      .unwrap_or(container.end() - 1); // before the closing bracket
    Gaps {
      before: before_start..members[*indexes.start()].range.start,
      after: members[*indexes.end()].range.end..after_end,
    }
  }

  /// Gets whether there's a line break before the first member and after the last member.
  fn line_breaks(&self, text: &str) -> (bool, bool) {
    (
      text[self.before.clone()].contains('\n'),
      text[self.after.clone()].contains('\n'),
    )
  }

  /// Goes to the start of the first member's line to fix its indentation and to the end of the
  /// last member's line to include its separator and any trailing comment. A member sharing its
  /// line with what's around it, or with the separator on the other side of the line break
  /// (ex. comma-first style), goes to what's around it instead.
  fn edges(&self, text: &str, commas: &[usize]) -> Edges {
    let line_start = text[self.before.clone()]
      .rfind('\n')
      .is_some_and(|index| !has_comma(commas, self.before.start + index..self.before.end));
    let line_end = text[self.after.clone()]
      .find('\n')
      .is_some_and(|index| !has_comma(commas, self.after.start + index..self.after.end));
    Edges { line_start, line_end }
  }

  fn replaced_range(&self, text: &str, edges: Edges) -> Range<usize> {
    let start = match text[self.before.clone()].rfind('\n') {
      Some(index) if edges.line_start => self.before.start + index + 1,
      _ => self.before.start,
    };
    let end = match text[self.after.clone()].find('\n') {
      Some(index) if edges.line_end => {
        let line = &text[self.after.start..self.after.start + index];
        self.after.start + line.trim_end_matches('\r').len()
      }
      _ => self.after.end,
    };
    start..end
  }
}

fn has_comma(commas: &[usize], range: Range<usize>) -> bool {
  let index = commas.partition_point(|comma| *comma < range.start);
  commas.get(index).is_some_and(|comma| *comma < range.end)
}

/// Keeps the line endings of the rest of the file since changing those is up to formatting the
/// whole file.
fn with_file_new_lines<'a>(formatted: &'a str, file_text: &str) -> Cow<'a, str> {
  if !file_text.contains('\n') {
    return Cow::Borrowed(formatted);
  }
  match resolve_new_line_kind(file_text, NewLineKind::Auto) {
    "\r\n" if formatted.contains('\n') && !formatted.contains("\r\n") => Cow::Owned(formatted.replace('\n', "\r\n")),
    "\n" if formatted.contains("\r\n") => Cow::Owned(formatted.replace("\r\n", "\n")),
    _ => Cow::Borrowed(formatted),
  }
}

#[cfg(test)]
mod tests {
  use crate::configuration::ConfigurationBuilder;

  use super::*;

  #[test]
  fn keeps_bom_outside_range() {
    // the spec files can't express this since editors strip the bom
    let config = ConfigurationBuilder::new().build();
    let text = "\u{FEFF}{\n  \"a\":1,\n  \"b\":2\n}\n";
    let start = text.find("\"b\"").unwrap();
    let output = format_text_range(Path::new("/file.json"), text, start..start + 5, &config)
      .unwrap()
      .unwrap();
    assert_eq!(output, "\u{FEFF}{\n  \"a\":1,\n  \"b\": 2\n}\n");
  }

  #[test]
  fn keeps_file_line_endings() {
    // the spec files can't express this since they normalize line endings
    let config = ConfigurationBuilder::new().build();
    let output = format_b(&config, "{\r\n  \"a\":1,\r\n  \"b\":{\r\n\"x\":1}\r\n}\r\n");
    assert_eq!(output, "{\r\n  \"a\":1,\r\n  \"b\": {\r\n    \"x\": 1\r\n  }\r\n}\r\n");

    let config = ConfigurationBuilder::new()
      .new_line_kind(NewLineKind::CarriageReturnLineFeed)
      .build();
    let output = format_b(&config, "{\n  \"a\":1,\n  \"b\":{\n\"x\":1}\n}\n");
    assert_eq!(output, "{\n  \"a\":1,\n  \"b\": {\n    \"x\": 1\n  }\n}\n");
  }

  fn format_b(config: &Configuration, text: &str) -> String {
    let start = text.find("\"b\"").unwrap();
    format_text_range(Path::new("/file.json"), text, start..start + 3, config)
      .unwrap()
      .unwrap()
  }
}
