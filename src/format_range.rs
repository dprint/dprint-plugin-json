use std::ops::Range;
use std::path::Path;

use jsonc_parser::ast::Value;
use jsonc_parser::common::Ranged;

use super::configuration::Configuration;
use super::format_text::FormatError;
use super::format_text::format_text;
use super::format_text::format_text_inner;
use super::format_text::parse;
use super::format_text::strip_bom;

/// Formats only the part of the text within the provided byte range.
///
/// The range is widened to the object properties or array elements it touches in the innermost
/// object or array that contains it, and the text outside of those is left as it was. When those
/// members don't keep their order once formatted (ex. the `package.json` conventions reorder
/// them) or their object or array switches between single and multi-line, the whole object or
/// array is formatted instead. A range that reaches the brackets of the root value formats the
/// whole file.
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
  let selection = match find_target(root, &range) {
    Target::File => return format_text(path, text, config),
    Target::Nothing => return Ok(None),
    Target::Members(selection) => selection,
  };

  let formatted = format_text_inner(path, body, config)?;
  let formatted_parse_result = parse(&formatted)?;
  let Some(formatted_root) = &formatted_parse_result.value else {
    return format_text(path, text, config);
  };
  let Some((original_range, formatted_range)) = find_replacement(&selection, body, formatted_root, &formatted) else {
    return format_text(path, text, config);
  };

  let result = format!(
    "{}{}{}",
    &text[..bom_len + original_range.start],
    &formatted[formatted_range],
    &text[bom_len + original_range.end..]
  );
  if result == text { Ok(None) } else { Ok(Some(result)) }
}

enum Target<'a> {
  /// The range reaches the root value's brackets or the root isn't an object or array.
  File,
  /// The range only touches whitespace or comments between members.
  Nothing,
  Members(Selection<'a>),
}

struct Selection<'a> {
  /// Steps from the root to the object or array holding the members.
  path: Vec<Step<'a>>,
  container: &'a Value<'a>,
  /// Steps to each touched member within the container, in order.
  members: Vec<Step<'a>>,
  /// Range from the start of the first touched member to the end of the last.
  range: Range<usize>,
}

/// Identifies a member by what it is rather than where it is, so it can be found again in the
/// formatted text even after its properties were reordered.
#[derive(Clone, Copy, PartialEq)]
enum Step<'a> {
  /// Property name along with how many properties with the same name come before it.
  Prop(&'a str, usize),
  Element(usize),
}

struct Member<'a> {
  step: Step<'a>,
  range: Range<usize>,
  value: &'a Value<'a>,
}

fn find_target<'a>(root: &'a Value<'a>, range: &Range<usize>) -> Target<'a> {
  if !is_within_brackets(root, range) {
    return Target::File;
  }
  let mut path = Vec::new();
  let mut container = root;
  loop {
    let Some(members) = members(container) else {
      return Target::File;
    };
    let touched = members
      .into_iter()
      .filter(|member| touches(&member.range, range))
      .collect::<Vec<_>>();
    match touched.as_slice() {
      [] => return Target::Nothing,
      [member] if is_container(member.value) && is_within_brackets(member.value, range) => {
        path.push(member.step);
        container = member.value;
      }
      [first, .., last] | [first @ last] => {
        return Target::Members(Selection {
          range: first.range.start..last.range.end,
          members: touched.iter().map(|member| member.step).collect(),
          path,
          container,
        });
      }
    }
  }
}

/// Finds the range in the original text to replace and the formatted text to replace it with.
fn find_replacement(
  selection: &Selection,
  text: &str,
  formatted_root: &Value,
  formatted_text: &str,
) -> Option<(Range<usize>, Range<usize>)> {
  let mut formatted_container = formatted_root;
  for step in &selection.path {
    formatted_container = members(formatted_container)?
      .into_iter()
      .find(|member| member.step == *step)?
      .value;
  }

  let formatted_members = members(formatted_container)?;
  let indexes = selection
    .members
    .iter()
    .map(|step| formatted_members.iter().position(|member| member.step == *step))
    .collect::<Option<Vec<_>>>()?;
  let keeps_order = indexes.windows(2).all(|pair| pair[1] == pair[0] + 1);
  let first_index = indexes[0];
  let last_index = *indexes.last()?;
  // the text around the spliced members is kept, so the line breaks there need to already be
  // what the formatter wants or the result would be half single and half multi-line
  let original_members = members(selection.container)?;
  let original_first_index = original_members
    .iter()
    .position(|member| member.step == selection.members[0])?;
  let original_last_index = original_first_index + selection.members.len() - 1;
  let keeps_lines = line_breaks_around(
    selection.container,
    &original_members,
    original_first_index..original_last_index,
    text,
  ) == line_breaks_around(
    formatted_container,
    &formatted_members,
    first_index..last_index,
    formatted_text,
  );
  if keeps_order && keeps_lines {
    let start = formatted_members[first_index].range.start;
    let end = formatted_members[last_index].range.end;
    Some((selection.range.clone(), start..end))
  } else if selection.path.is_empty() {
    // the container is the root, so the whole file needs formatting
    None
  } else {
    Some((range_of(selection.container), range_of(formatted_container)))
  }
}

fn members<'a>(value: &'a Value<'a>) -> Option<Vec<Member<'a>>> {
  match value {
    Value::Object(object) => {
      let mut members: Vec<Member<'a>> = Vec::with_capacity(object.properties.len());
      for prop in &object.properties {
        let name = prop.name.as_str();
        let occurrence = members
          .iter()
          .filter(|member| matches!(member.step, Step::Prop(other, _) if other == name))
          .count();
        members.push(Member {
          step: Step::Prop(name, occurrence),
          range: prop.range.start..prop.range.end,
          value: &prop.value,
        });
      }
      Some(members)
    }
    Value::Array(array) => Some(
      array
        .elements
        .iter()
        .enumerate()
        .map(|(index, element)| Member {
          step: Step::Element(index),
          range: range_of(element),
          value: element,
        })
        .collect(),
    ),
    _ => None,
  }
}

fn is_container(value: &Value) -> bool {
  matches!(value, Value::Object(_) | Value::Array(_))
}

fn is_within_brackets(value: &Value, range: &Range<usize>) -> bool {
  is_container(value) && range.start > value.start() && range.end < value.end()
}

fn touches(member: &Range<usize>, range: &Range<usize>) -> bool {
  if range.is_empty() {
    member.start <= range.start && range.start <= member.end
  } else {
    range.start < member.end && range.end > member.start
  }
}

/// Gets whether there's a line break before the first member and after the last member of the
/// provided inclusive range of member indexes.
fn line_breaks_around(container: &Value, members: &[Member], indexes: Range<usize>, text: &str) -> (bool, bool) {
  let first = &members[indexes.start];
  let last = &members[indexes.end];
  let before_start = match indexes.start {
    0 => container.start(),
    index => members[index - 1].range.end,
  };
  let after_end = members
    .get(indexes.end + 1)
    .map(|member| member.range.start)
    .unwrap_or(container.end());
  (
    text[before_start..first.range.start].contains('\n'),
    text[last.range.end..after_end].contains('\n'),
  )
}

fn range_of(value: &Value) -> Range<usize> {
  value.start()..value.end()
}

#[cfg(test)]
mod tests {
  use std::path::Path;

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
}
