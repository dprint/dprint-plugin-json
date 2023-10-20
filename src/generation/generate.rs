use super::super::configuration::Configuration;
use super::context::Context;
use super::token_finder::TokenFinder;
use dprint_core::formatting::ir_helpers::SingleLineOptions;
use dprint_core::formatting::*;
use jsonc_parser::ast::*;
use jsonc_parser::common::Range;
use jsonc_parser::common::Ranged;
use jsonc_parser::tokens::TokenAndRange;
use std::collections::HashSet;
use std::rc::Rc;
use text_lines::TextLines;

use crate::configuration::*;

pub fn generate(
  parse_result: jsonc_parser::ParseResult,
  text: &str,
  config: &Configuration,
  is_jsonc: bool,
) -> PrintItems {
  let comments = parse_result.comments.unwrap();
  let tokens = parse_result.tokens.unwrap();
  let node_value = parse_result.value;
  let text_info = TextLines::new(text);
  let mut context = Context {
    config,
    text,
    text_info,
    is_jsonc,
    handled_comments: HashSet::new(),
    parent_stack: Vec::new(),
    current_node: None,
    comments: &comments,
    token_finder: TokenFinder::new(&tokens),
  };

  let mut items = PrintItems::new();
  if let Some(node_value) = &node_value {
    items.extend(gen_node(node_value.into(), &mut context));
    items.extend(gen_trailing_comments_as_statements(node_value, &mut context));
  } else if let Some(comments) = comments.get(&0) {
    items.extend(gen_comments_as_statements(comments.iter(), None, &mut context));
  }
  items.push_condition(conditions::if_true(
    "endOfFileNewLine",
    Rc::new(|context| Some(context.writer_info.column_number > 0 || context.writer_info.line_number > 0)),
    Signal::NewLine.into(),
  ));

  items
}

fn gen_node<'a>(node: Node<'a, 'a>, context: &mut Context<'a, '_>) -> PrintItems {
  gen_node_with_inner(node, context, |items, _| items)
}

fn gen_node_with_inner<'a>(
  node: Node<'a, 'a>,
  context: &mut Context<'a, '_>,
  inner_gen: impl FnOnce(PrintItems, &mut Context<'a, '_>) -> PrintItems,
) -> PrintItems {
  // store info
  let past_current_node = context.current_node.replace(node.clone());
  let parent_end = past_current_node.as_ref().map(|n| n.end());
  let node_end = node.end();
  let is_root = past_current_node.is_none();

  if let Some(past_current_node) = past_current_node {
    context.parent_stack.push(past_current_node);
  }

  // generate item
  let mut items = PrintItems::new();

  // get the leading comments
  if let Some(comments) = context.comments.get(&node.start()) {
    items.extend(gen_comments_as_leading(&node, comments.iter(), context));
  }

  // generate the node
  if has_ignore_comment(&node, context) {
    items.push_str(""); // force the current line indentation
    items.extend(inner_gen(
      ir_helpers::gen_from_raw_string(node.text(context.text)),
      context,
    ));
  } else {
    items.extend(inner_gen(gen_node_inner(&node, context), context))
  }

  // get the trailing comments
  if is_root || parent_end.is_some() && parent_end.unwrap() != node_end {
    if let Some(comments) = context.comments.get(&node_end) {
      items.extend(gen_comments_as_trailing(&node, comments.iter(), context));
    }
  }

  context.current_node = context.parent_stack.pop();

  return items;

  #[inline]
  fn gen_node_inner<'a>(node: &Node<'a, 'a>, context: &mut Context<'a, '_>) -> PrintItems {
    match node {
      Node::Array(node) => gen_array(node, context),
      Node::BooleanLit(node) => node.value.to_string().into(),
      Node::NullKeyword(_) => "null".into(),
      Node::NumberLit(node) => node.value.to_string().into(),
      Node::Object(node) => gen_object(node, context),
      Node::ObjectProp(node) => gen_object_prop(node, context),
      Node::StringLit(node) => gen_string_lit(node, context),
      Node::WordLit(node) => gen_word_lit(node, context),
    }
  }
}

fn gen_array<'a>(node: &'a Array<'a>, context: &mut Context<'a, '_>) -> PrintItems {
  let force_multi_lines = !context.config.array_prefer_single_line
    && (should_break_up_single_line(node, context)
      || context.text_info.line_index(node.start())
        < node
          .elements
          .first()
          .map(|p| context.text_info.line_index(p.start()))
          .unwrap_or_else(|| context.text_info.line_index(node.start())));

  gen_surrounded_by_tokens(
    |context| {
      let mut items = PrintItems::new();
      items.extend(gen_comma_separated_values(
        GenCommaSeparatedValuesOptions {
          nodes: node.elements.iter().map(|x| Some(x.into())).collect(),
          prefer_hanging: false,
          force_use_new_lines: force_multi_lines,
          allow_blank_lines: true,
          single_line_space_at_start: false,
          single_line_space_at_end: false,
          custom_single_line_separator: None,
          multi_line_options: ir_helpers::MultiLineOptions::surround_newlines_indented(),
          force_possible_newline_at_start: false,
        },
        context,
      ));
      items
    },
    GenSurroundedByTokensOptions {
      open_token: "[",
      close_token: "]",
      range: node.range,
      first_member: node.elements.first().map(|f| f.range()),
      prefer_single_line_when_empty: true,
    },
    context,
  )
}

fn gen_object<'a>(obj: &'a Object, context: &mut Context<'a, '_>) -> PrintItems {
  let force_multi_lines = !context.config.object_prefer_single_line
    && (should_break_up_single_line(obj, context)
      || context.text_info.line_index(obj.start())
        < obj
          .properties
          .first()
          .map(|p| context.text_info.line_index(p.start()))
          .unwrap_or_else(|| context.text_info.line_index(obj.end())));

  gen_surrounded_by_tokens(
    |context| {
      let mut items = PrintItems::new();
      items.extend(gen_comma_separated_values(
        GenCommaSeparatedValuesOptions {
          nodes: obj.properties.iter().map(|x| Some(Node::ObjectProp(x))).collect(),
          prefer_hanging: false,
          force_use_new_lines: force_multi_lines,
          allow_blank_lines: true,
          single_line_space_at_start: true,
          single_line_space_at_end: true,
          custom_single_line_separator: None,
          multi_line_options: ir_helpers::MultiLineOptions::surround_newlines_indented(),
          force_possible_newline_at_start: false,
        },
        context,
      ));
      items
    },
    GenSurroundedByTokensOptions {
      open_token: "{",
      close_token: "}",
      range: obj.range,
      first_member: obj.properties.first().map(|f| &f.range),
      prefer_single_line_when_empty: false,
    },
    context,
  )
}

fn gen_object_prop<'a>(node: &'a ObjectProp, context: &mut Context<'a, '_>) -> PrintItems {
  let mut items = PrintItems::new();
  items.extend(gen_node((&node.name).into(), context));
  items.push_str(": ");
  items.extend(gen_node((&node.value).into(), context));

  items
}

fn gen_string_lit<'a>(node: &'a StringLit, context: &mut Context<'a, '_>) -> PrintItems {
  let text = node.text(context.text);
  let is_double_quotes = text.starts_with('"');
  let mut items = PrintItems::new();
  let text = &text[1..text.len() - 1];
  let text = if is_double_quotes {
    text.replace("\\\"", "\"")
  } else {
    text.replace("\\'", "'")
  };
  items.push_str("\"");
  items.push_string(text.replace('"', "\\\""));
  items.push_str("\"");
  items
}

fn gen_word_lit<'a>(node: &'a WordLit<'a>, _: &mut Context<'a, '_>) -> PrintItems {
  // this will be a property name that's not a string literal
  let mut items = PrintItems::new();
  items.push_str("\"");
  items.push_string(node.value.to_string());
  items.push_str("\"");
  items
}

struct GenCommaSeparatedValuesOptions<'a> {
  nodes: Vec<Option<Node<'a, 'a>>>,
  prefer_hanging: bool,
  force_use_new_lines: bool,
  allow_blank_lines: bool,
  single_line_space_at_start: bool,
  single_line_space_at_end: bool,
  custom_single_line_separator: Option<PrintItems>,
  multi_line_options: ir_helpers::MultiLineOptions,
  force_possible_newline_at_start: bool,
}

fn gen_comma_separated_values<'a>(
  opts: GenCommaSeparatedValuesOptions<'a>,
  context: &mut Context<'a, '_>,
) -> PrintItems {
  let nodes = opts.nodes;
  let indent_width = context.config.indent_width;
  let compute_lines_span = opts.allow_blank_lines && opts.force_use_new_lines; // save time otherwise
  ir_helpers::gen_separated_values(
    |_| {
      let mut generated_nodes = Vec::new();
      let nodes_count = nodes.len();
      for (i, value) in nodes.into_iter().enumerate() {
        let (allow_inline_multi_line, allow_inline_single_line) = if let Some(value) = &value {
          (value.kind() == NodeKind::Object, false)
        } else {
          (false, false)
        };
        let lines_span = if compute_lines_span {
          value.as_ref().map(|x| ir_helpers::LinesSpan {
            start_line: context.start_line_with_comments(x),
            end_line: context.end_line_with_comments(x),
          })
        } else {
          None
        };
        let items = ir_helpers::new_line_group({
          let is_final_node = i == nodes_count - 1;
          let should_have_comma = match context.config.trailing_commas {
            TrailingCommaKind::Always => true,
            TrailingCommaKind::Maintain => {
              if is_final_node {
                match &value {
                  Some(value) => context.token_finder.get_next_token_if_comma(value.range()).is_some(),
                  None => false,
                }
              } else {
                true
              }
            }
            TrailingCommaKind::Jsonc => !is_final_node || context.is_jsonc,
            TrailingCommaKind::Never => !is_final_node,
          };
          let comma_or_nothing = if should_have_comma {
            ",".into()
          } else {
            PrintItems::new()
          };
          gen_comma_separated_value(value, comma_or_nothing, context)
        });
        generated_nodes.push(ir_helpers::GeneratedValue {
          items,
          lines_span,
          allow_inline_multi_line,
          allow_inline_single_line,
        });
      }

      generated_nodes
    },
    ir_helpers::GenSeparatedValuesOptions {
      prefer_hanging: opts.prefer_hanging,
      force_use_new_lines: opts.force_use_new_lines,
      allow_blank_lines: opts.allow_blank_lines,
      single_line_options: SingleLineOptions {
        space_at_start: opts.single_line_space_at_start,
        space_at_end: opts.single_line_space_at_end,
        separator: opts
          .custom_single_line_separator
          .unwrap_or_else(|| Signal::SpaceOrNewLine.into()),
      },
      indent_width,
      multi_line_options: opts.multi_line_options,
      force_possible_newline_at_start: opts.force_possible_newline_at_start,
    },
  )
  .items
}

fn gen_comma_separated_value<'a>(
  value: Option<Node<'a, 'a>>,
  generated_comma: PrintItems,
  context: &mut Context<'a, '_>,
) -> PrintItems {
  let mut items = PrintItems::new();
  let comma_token = get_comma_token(&value, context);

  if let Some(element) = value {
    let generated_comma = generated_comma.into_rc_path();
    items.extend(gen_node_with_inner(element, context, move |mut items, _| {
      // this Rc clone is necessary because we can't move the captured generated_comma out of this closure
      items.push_optional_path(generated_comma);
      items
    }));
  } else {
    items.extend(generated_comma);
  }

  // get the trailing comments after the comma token
  if let Some(comma_token) = comma_token {
    items.extend(gen_trailing_comments(comma_token, context));
  }

  return items;

  fn get_comma_token<'a, 'b>(element: &Option<Node>, context: &mut Context<'a, 'b>) -> Option<&'b TokenAndRange<'a>> {
    if let Some(element) = element {
      context.token_finder.get_next_token_if_comma(element)
    } else {
      None
    }
  }
}

struct GenSurroundedByTokensOptions<'a> {
  open_token: &'static str,
  close_token: &'static str,
  range: Range,
  first_member: Option<&'a Range>,
  prefer_single_line_when_empty: bool,
}

fn gen_surrounded_by_tokens<'a, 'b>(
  gen_inner: impl FnOnce(&mut Context<'a, 'b>) -> PrintItems,
  opts: GenSurroundedByTokensOptions<'a>,
  context: &mut Context<'a, 'b>,
) -> PrintItems {
  let open_token_end = opts.range.start + opts.open_token.len();
  let close_token_start = opts.range.end - opts.close_token.len();

  // assert the tokens are in the place the caller says they are
  #[cfg(debug_assertions)]
  context.assert_text(opts.range.start, open_token_end, opts.open_token);
  #[cfg(debug_assertions)]
  context.assert_text(close_token_start, opts.range.end, opts.close_token);

  // generate
  let mut items = PrintItems::new();
  let open_token_start_line = context.text_info.line_index(opts.range.start);

  items.push_str(opts.open_token);
  if let Some(first_member) = opts.first_member {
    let first_member_start_line = context.text_info.line_index(first_member.start);
    if open_token_start_line < first_member_start_line {
      if let Some(trailing_comments) = context.comments.get(&open_token_end) {
        items.extend(gen_first_line_trailing_comment(
          open_token_start_line,
          trailing_comments.iter(),
          context,
        ));
      }
    }
    items.extend(gen_inner(context));

    let before_trailing_comments_lc = LineAndColumn::new("beforeTrailingComments");
    items.push_line_and_column(before_trailing_comments_lc);
    items.extend(ir_helpers::with_indent(gen_trailing_comments_as_statements(
      &Range::from_byte_index(open_token_end),
      context,
    )));
    if let Some(leading_comments) = context.comments.get(&close_token_start) {
      items.extend(ir_helpers::with_indent(gen_comments_as_statements(
        leading_comments.iter(),
        None,
        context,
      )));
    }
    items.push_condition(conditions::if_true(
      "newLineIfHasCommentsAndNotStartOfNewLine",
      Rc::new(move |context| {
        let had_comments = !condition_helpers::is_at_same_position(context, before_trailing_comments_lc)?;
        Some(had_comments && !context.writer_info.is_start_of_line())
      }),
      Signal::NewLine.into(),
    ));
  } else {
    let range_end_line = context.text_info.line_index(opts.range.end);
    let is_single_line = open_token_start_line == range_end_line;
    if let Some(comments) = context.comments.get(&open_token_end) {
      // generate the trailing comment on the first line only if multi-line and if a comment line
      if !is_single_line {
        items.extend(gen_first_line_trailing_comment(
          open_token_start_line,
          comments.iter(),
          context,
        ));
      }

      // generate the comments
      if has_unhandled_comment(comments.iter(), context) {
        if is_single_line {
          let indent_width = context.config.indent_width;
          items.extend(
            ir_helpers::gen_separated_values(
              |_| {
                let mut generated_comments = Vec::new();
                for c in comments.iter() {
                  let start_line = context.text_info.line_index(c.start());
                  let end_line = context.text_info.line_index(c.end());
                  if let Some(items) = gen_comment(c, context) {
                    generated_comments.push(ir_helpers::GeneratedValue {
                      items,
                      lines_span: Some(ir_helpers::LinesSpan { start_line, end_line }),
                      allow_inline_multi_line: false,
                      allow_inline_single_line: false,
                    });
                  }
                }
                generated_comments
              },
              ir_helpers::GenSeparatedValuesOptions {
                prefer_hanging: false,
                force_use_new_lines: !is_single_line,
                allow_blank_lines: true,
                single_line_options: ir_helpers::SingleLineOptions {
                  space_at_start: false,
                  space_at_end: false,
                  separator: Signal::SpaceOrNewLine.into(),
                },
                indent_width,
                multi_line_options: ir_helpers::MultiLineOptions::surround_newlines_indented(),
                force_possible_newline_at_start: false,
              },
            )
            .items,
          );
        } else {
          items.push_signal(Signal::NewLine);
          items.extend(ir_helpers::with_indent(gen_comments_as_statements(
            comments.iter(),
            None,
            context,
          )));
          items.push_signal(Signal::NewLine);
        }
      }
    } else if !is_single_line && !opts.prefer_single_line_when_empty {
      items.push_signal(Signal::NewLine);
    }
  }

  items.push_str(opts.close_token);

  return items;

  fn gen_first_line_trailing_comment<'a: 'b, 'b>(
    open_token_start_line: usize,
    comments: impl Iterator<Item = &'b Comment<'a>>,
    context: &mut Context,
  ) -> PrintItems {
    let mut items = PrintItems::new();
    let mut comments = comments;
    if let Some(first_comment) = comments.next() {
      if first_comment.kind() == CommentKind::Line
        && context.text_info.line_index(first_comment.start()) == open_token_start_line
      {
        if let Some(generated_comment) = gen_comment(first_comment, context) {
          items.push_signal(Signal::StartForceNoNewLines);
          items.push_str(" ");
          items.extend(generated_comment);
          items.push_signal(Signal::FinishForceNoNewLines);
        }
      }
    }
    items
  }
}

// Comments

fn has_unhandled_comment<'a: 'b, 'b>(
  mut comments: impl Iterator<Item = &'b Comment<'a>>,
  context: &mut Context,
) -> bool {
  comments.any(|c| !context.has_handled_comment(c))
}

fn gen_trailing_comments(node: &dyn Ranged, context: &mut Context) -> PrintItems {
  if let Some(trailing_comments) = context.comments.get(&node.end()) {
    gen_comments_as_trailing(node, trailing_comments.iter(), context)
  } else {
    PrintItems::new()
  }
}

fn gen_trailing_comments_as_statements(node: &dyn Ranged, context: &mut Context) -> PrintItems {
  let unhandled_comments = get_trailing_comments_as_statements(node, context);
  gen_comments_as_statements(unhandled_comments.into_iter(), Some(node), context)
}

fn get_trailing_comments_as_statements<'a, 'b>(
  node: &dyn Ranged,
  context: &mut Context<'a, 'b>,
) -> Vec<&'b Comment<'a>> {
  let mut comments = Vec::new();
  let node_end_line = context.text_info.line_index(node.end());
  if let Some(trailing_comments) = context.comments.get(&node.end()) {
    for comment in trailing_comments.iter() {
      if !context.has_handled_comment(comment) && node_end_line < context.text_info.line_index(comment.end()) {
        comments.push(comment);
      }
    }
  }
  comments
}

fn gen_comments_as_statements<'a: 'b, 'b>(
  comments: impl Iterator<Item = &'b Comment<'a>>,
  last_node: Option<&dyn Ranged>,
  context: &mut Context<'a, 'b>,
) -> PrintItems {
  let mut last_node = last_node;
  let mut items = PrintItems::new();
  for comment in comments {
    if !context.has_handled_comment(comment) {
      items.extend(gen_comment_based_on_last_node(
        comment,
        &last_node,
        GenCommentBasedOnLastNodeOptions {
          separate_with_newlines: true,
        },
        context,
      ));
      last_node = Some(comment);
    }
  }
  items
}

fn gen_comments_as_leading<'a: 'b, 'b>(
  node: &dyn Ranged,
  comments: impl Iterator<Item = &'b Comment<'a>>,
  context: &mut Context,
) -> PrintItems {
  let mut items = PrintItems::new();
  let comments = comments.filter(|c| !context.has_handled_comment(c)).collect::<Vec<_>>();

  if !comments.is_empty() {
    let last_comment = comments.last().unwrap();
    let last_comment_end_line = context.text_info.line_index(last_comment.end());
    let last_comment_kind = last_comment.kind();
    items.extend(gen_comment_collection(comments.into_iter(), None, Some(node), context));

    let node_start_line = context.text_info.line_index(node.start());
    if node_start_line > last_comment_end_line {
      items.push_signal(Signal::NewLine);

      if node_start_line - 1 > last_comment_end_line {
        items.push_signal(Signal::NewLine);
      }
    } else if last_comment_kind == CommentKind::Block && node_start_line == last_comment_end_line {
      items.push_signal(Signal::SpaceIfNotTrailing);
    }
  }

  items
}

fn gen_comments_as_trailing<'a: 'b, 'b>(
  node: &dyn Ranged,
  comments: impl Iterator<Item = &'b Comment<'a>>,
  context: &mut Context,
) -> PrintItems {
  // use the roslyn definition of trailing comments
  let node_end_line = context.text_info.line_index(node.end());
  let trailing_comments_on_same_line = comments
    .filter(|c| context.text_info.line_index(c.start()) <= node_end_line)
    .collect::<Vec<_>>();

  let first_unhandled_comment = trailing_comments_on_same_line
    .iter()
    .find(|c| !context.has_handled_comment(c));
  let mut items = PrintItems::new();

  if let Some(Comment::Block(_)) = first_unhandled_comment {
    items.push_str(" ");
  }

  items.extend(gen_comment_collection(
    trailing_comments_on_same_line.into_iter(),
    Some(node),
    None,
    context,
  ));

  items
}

fn gen_comment_collection<'a: 'b, 'b>(
  comments: impl Iterator<Item = &'b Comment<'a>>,
  last_node: Option<&dyn Ranged>,
  next_node: Option<&dyn Ranged>,
  context: &mut Context,
) -> PrintItems {
  let mut last_node = last_node;
  let mut items = PrintItems::new();
  let next_node_start_line = next_node.map(|n| context.text_info.line_index(n.start()));

  for comment in comments {
    if !context.has_handled_comment(comment) {
      items.extend(gen_comment_based_on_last_node(
        comment,
        &last_node,
        GenCommentBasedOnLastNodeOptions {
          separate_with_newlines: if let Some(next_node_start_line) = next_node_start_line {
            context.text_info.line_index(comment.start()) != next_node_start_line
          } else {
            false
          },
        },
        context,
      ));
      last_node = Some(comment);
    }
  }

  items
}

struct GenCommentBasedOnLastNodeOptions {
  separate_with_newlines: bool,
}

fn gen_comment_based_on_last_node(
  comment: &Comment,
  last_node: &Option<&dyn Ranged>,
  opts: GenCommentBasedOnLastNodeOptions,
  context: &mut Context,
) -> PrintItems {
  let mut items = PrintItems::new();
  let mut pushed_ignore_new_lines = false;

  if let Some(last_node) = last_node {
    let comment_start_line = context.text_info.line_index(comment.start());
    let last_node_end_line = context.text_info.line_index(last_node.end());

    if opts.separate_with_newlines || comment_start_line > last_node_end_line {
      items.push_signal(Signal::NewLine);

      if comment_start_line > last_node_end_line + 1 {
        items.push_signal(Signal::NewLine);
      }
    } else if comment.kind() == CommentKind::Line {
      items.push_signal(Signal::StartForceNoNewLines);
      items.push_str(" ");
      pushed_ignore_new_lines = true;
    } else if last_node.text(context.text).starts_with("/*") {
      items.push_str(" ");
    }
  }

  if let Some(generated_comment) = gen_comment(comment, context) {
    items.extend(generated_comment);
  }

  if pushed_ignore_new_lines {
    items.push_signal(Signal::FinishForceNoNewLines);
  }

  items
}

fn gen_comment(comment: &Comment, context: &mut Context) -> Option<PrintItems> {
  // only generate if handled
  if context.has_handled_comment(comment) {
    return None;
  }

  // mark handled and generate
  context.mark_comment_handled(comment);
  Some(match comment {
    Comment::Block(comment) => ir_helpers::gen_js_like_comment_block(comment.text),
    Comment::Line(comment) => {
      ir_helpers::gen_js_like_comment_line(comment.text, context.config.comment_line_force_space_after_slashes)
    }
  })
}

fn has_ignore_comment(node: &dyn Ranged, context: &Context) -> bool {
  if let Some(last_comment) = context.comments.get(&(node.start())).and_then(|c| c.last()) {
    ir_helpers::text_has_dprint_ignore(last_comment.text(), &context.config.ignore_node_comment_text)
  } else {
    false
  }
}

fn should_break_up_single_line(ranged: &impl Ranged, context: &Context) -> bool {
  // This is a massive performance improvement when formatting huge single line files.
  // Basically, if the node is on a single line and will for sure format as multi-line, then
  // say it's multi-line right away and avoid creating print items to figure that out.
  let range = ranged.range();

  // Obviously this line_width * 2 is not always accurate as it doesn't take into account whitespace,
  // but will provide a good enough and fast way to quickly tell if it's long without having basically
  // any false positives (unless someone is being silly).
  context.text_info.line_index(range.start) == context.text_info.line_index(range.end)
    && range.width() > (context.config.line_width * 2) as usize
}
