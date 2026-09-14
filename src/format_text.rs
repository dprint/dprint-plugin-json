use std::borrow::Cow;
use std::path::Path;

use dprint_core::configuration::resolve_new_line_kind;
use dprint_core::formatting::PrintOptions;
use jsonc_parser::CollectOptions;
use jsonc_parser::CommentCollectionStrategy;
use jsonc_parser::ParseResult;
use jsonc_parser::errors::ParseError;
use jsonc_parser::parse_to_ast;

use super::configuration::Configuration;
use super::configuration::TrailingCommaKind;
use super::generation::generate;
use super::glob;
use super::package_json;

/// Error that occurs while formatting.
///
/// The [`Display`](std::fmt::Display) output is a formatted diagnostic, while
/// the underlying [`ParseError`] can be recovered via [`Error::source`](std::error::Error::source).
#[derive(Debug, thiserror::Error)]
#[error("{diagnostic}")]
pub struct FormatError {
  diagnostic: String,
  #[source]
  source: ParseError,
}

impl FormatError {
  /// The error message without position or source highlight (ex. `Unexpected token`).
  pub fn message(&self) -> String {
    self.source.kind().to_string()
  }

  /// The parser error that caused this formatting error.
  pub fn parse_error(&self) -> &ParseError {
    &self.source
  }
}

pub fn format_text(path: &Path, text: &str, config: &Configuration) -> Result<Option<String>, FormatError> {
  let result = format_text_inner(path, text, config)?;
  if result == text { Ok(None) } else { Ok(Some(result)) }
}

fn format_text_inner(path: &Path, text: &str, config: &Configuration) -> Result<String, FormatError> {
  let text = strip_bom(text);
  let text = if config.package_json_apply_conventions && package_json::is_package_json_file(path) {
    package_json::apply_conventions(text, config)
  } else {
    Cow::Borrowed(text)
  };
  let parse_result = parse(&text)?;
  // only used for jsonc trailing commas, so avoid matching the path otherwise
  let is_jsonc = config.trailing_commas == TrailingCommaKind::Jsonc && is_jsonc_file(path, config);
  Ok(dprint_core::formatting::format(
    || generate(parse_result, &text, config, is_jsonc),
    config_to_print_options(&text, config),
  ))
}

#[cfg(feature = "tracing")]
pub fn trace_file(text: &str, config: &Configuration) -> dprint_core::formatting::TracingResult {
  let parse_result = parse(text).unwrap();

  dprint_core::formatting::trace_printing(
    || generate(parse_result, text, config, false),
    config_to_print_options(text, config),
  )
}

fn strip_bom(text: &str) -> &str {
  text.strip_prefix("\u{FEFF}").unwrap_or(text)
}

fn parse(text: &str) -> Result<ParseResult<'_>, FormatError> {
  let parse_result = parse_to_ast(
    text,
    &CollectOptions {
      comments: CommentCollectionStrategy::Separate,
      tokens: true,
    },
    &Default::default(),
  );
  match parse_result {
    Ok(result) => Ok(result),
    Err(err) => {
      let diagnostic = dprint_core::formatting::utils::string_utils::format_diagnostic(
        Some((err.range().start, err.range().end)),
        &err.kind().to_string(),
        text,
      );
      Err(FormatError {
        diagnostic,
        source: err,
      })
    }
  }
}

fn config_to_print_options(text: &str, config: &Configuration) -> PrintOptions {
  PrintOptions {
    indent_width: config.indent_width,
    max_width: config.line_width,
    use_tabs: config.use_tabs,
    new_line_text: resolve_new_line_kind(text, config.new_line_kind),
  }
}

fn is_jsonc_file(path: &Path, config: &Configuration) -> bool {
  fn has_jsonc_extension(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("jsonc"))
  }

  fn is_special_json_file(path: &Path, config: &Configuration) -> bool {
    if config.json_trailing_comma_files.is_empty() {
      return false;
    }

    let path = path.to_string_lossy();
    config
      .json_trailing_comma_files
      .iter()
      .any(|pattern| glob::matches_path_end(pattern, &path))
  }

  has_jsonc_extension(path) || is_special_json_file(path, config)
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use crate::configuration::ConfigurationBuilder;
  use crate::configuration::EofNewLineKind;
  use crate::configuration::TrailingCommaKind;

  use super::super::configuration::resolve_config;
  use super::*;
  use dprint_core::configuration::*;

  #[test]
  fn should_error_on_syntax_diagnostic() {
    let global_config = GlobalConfiguration::default();
    let config = resolve_config(ConfigKeyMap::new(), &global_config).config;
    let message = format_text(Path::new("."), "{ &*&* }", &config)
      .err()
      .unwrap()
      .to_string();
    assert_eq!(
      message,
      concat!("Line 1, column 3: Unexpected token\n", "\n", "  { &*&* }\n", "    ~")
    );
  }

  #[test]
  fn no_panic_diagnostic_at_multibyte_char() {
    let global_config = GlobalConfiguration::default();
    let config = resolve_config(ConfigKeyMap::new(), &global_config).config;
    let message = format_text(Path::new("."), "{ \"a\":\u{200b}5 }", &config)
      .err()
      .unwrap()
      .to_string();
    assert_eq!(
      message,
      "Line 1, column 7: Unexpected token\n\n  { \"a\":\u{200b}5 }\n        ~"
    );
  }

  #[test]
  fn no_panic_diagnostic_multiple_values() {
    let global_config = GlobalConfiguration::default();
    let config = resolve_config(ConfigKeyMap::new(), &global_config).config;
    let message = format_text(Path::new("."), "{},\n", &config).err().unwrap().to_string();
    assert_eq!(
      message,
      "Line 1, column 3: Text cannot contain more than one JSON value\n\n  {},\n    ~"
    );
  }

  #[test]
  fn test_is_jsonc_file() {
    let config = ConfigurationBuilder::new()
      .json_trailing_comma_files(vec!["tsconfig.json".to_string(), ".vscode/settings.json".to_string()])
      .build();
    assert!(!is_jsonc_file(&PathBuf::from("/asdf.json"), &config));
    assert!(is_jsonc_file(&PathBuf::from("/asdf.jsonc"), &config));
    assert!(is_jsonc_file(&PathBuf::from("/ASDF.JSONC"), &config));
    assert!(is_jsonc_file(&PathBuf::from("/tsconfig.json"), &config));
    assert!(is_jsonc_file(&PathBuf::from("/test/.vscode/settings.json"), &config));
    assert!(!is_jsonc_file(&PathBuf::from("/test/vscode/settings.json"), &config));
    if cfg!(windows) {
      assert!(is_jsonc_file(&PathBuf::from("test\\.vscode\\settings.json"), &config));
    }
  }

  #[test]
  fn test_is_jsonc_file_globs() {
    let config = ConfigurationBuilder::new()
      .json_trailing_comma_files(vec!["{j,t}sconfig*.json".to_string(), ".vscode/*.json".to_string()])
      .build();
    assert!(is_jsonc_file(&PathBuf::from("/tsconfig.json"), &config));
    assert!(is_jsonc_file(&PathBuf::from("/a/jsconfig.json"), &config));
    assert!(is_jsonc_file(&PathBuf::from("/a/tsconfig.lib.prod.json"), &config));
    assert!(!is_jsonc_file(&PathBuf::from("/a/psconfig.json"), &config));
    assert!(is_jsonc_file(&PathBuf::from("/a/.vscode/tasks.json"), &config));
    assert!(!is_jsonc_file(&PathBuf::from("/a/.vscode/sub/tasks.json"), &config));
    assert!(!is_jsonc_file(&PathBuf::from("/a/vscode/tasks.json"), &config));
    assert!(is_jsonc_file(&PathBuf::from("C:\\a\\.vscode\\launch.json"), &config));
  }

  #[test]
  fn package_json_listed_as_a_trailing_comma_file() {
    // opting package.json into jsonc is the caller's business; the conventions still apply, and
    // the two compose into a reordered file with the trailing comma that was asked for
    let config = ConfigurationBuilder::new()
      .json_trailing_comma_files(vec!["package.json".to_string()])
      .trailing_commas(TrailingCommaKind::Jsonc)
      .build();
    let text = "{\n  \"version\": \"1.0.0\",\n  \"name\": \"a\"\n}\n";
    let output = format_text(Path::new("/package.json"), text, &config).unwrap().unwrap();
    assert_eq!(output, "{\n  \"name\": \"a\",\n  \"version\": \"1.0.0\",\n}\n");
  }

  #[test]
  fn package_json_that_fails_to_parse_reports_the_usual_diagnostic() {
    // text that doesn't parse is handed straight back by the conventions, so the positions in the
    // message are the ones the author wrote
    let global_config = GlobalConfiguration::default();
    let config = resolve_config(ConfigKeyMap::new(), &global_config).config;
    let message = format_text(Path::new("/package.json"), "{ &*&* }", &config)
      .err()
      .unwrap()
      .to_string();
    assert_eq!(
      message,
      concat!("Line 1, column 3: Unexpected token\n", "\n", "  { &*&* }\n", "    ~")
    );
  }

  #[test]
  fn eof_new_line_maintain_whitespace() {
    // the spec files can't express these since they normalize newlines and editors trim trailing spaces
    let config = ConfigurationBuilder::new()
      .eof_new_line(EofNewLineKind::Maintain)
      .new_line_kind(NewLineKind::Auto)
      .build();
    assert_eq!(format(&config, "{\"a\":1}\r\n"), "{ \"a\": 1 }\r\n");
    assert_eq!(format(&config, "{\"a\":1}\r\n\r\n"), "{ \"a\": 1 }\r\n");
    assert_eq!(format(&config, "{\"a\":1}  "), "{ \"a\": 1 }");
    assert_eq!(format(&config, "{\"a\":1}\n  "), "{ \"a\": 1 }\n");
    assert_eq!(format(&config, "\u{FEFF}{\"a\":1}"), "{ \"a\": 1 }");

    let config = ConfigurationBuilder::new()
      .eof_new_line(EofNewLineKind::Maintain)
      .build();
    assert_eq!(format(&config, "{\"a\":1}\r"), "{ \"a\": 1 }\n");
  }

  #[test]
  fn eof_new_line_never_crlf() {
    let config = ConfigurationBuilder::new()
      .eof_new_line(EofNewLineKind::Never)
      .new_line_kind(NewLineKind::Auto)
      .build();
    assert_eq!(format(&config, "{\r\n  \"a\": 1\r\n}\r\n"), "{\r\n  \"a\": 1\r\n}");
  }

  #[test]
  fn should_strip_bom() {
    for input_text in ["\u{FEFF}{}", "\u{FEFF}{ }"] {
      let global_config = GlobalConfiguration::default();
      let config = resolve_config(ConfigKeyMap::new(), &global_config).config;
      let output_text = format_text(Path::new("."), input_text, &config).unwrap().unwrap();
      assert_eq!(output_text, "{}\n");
    }
  }

  #[test]
  fn escapes_raw_control_chars_in_strings() {
    // json doesn't allow these unescaped in strings (https://github.com/dprint/dprint-plugin-json/issues/63)
    let config = ConfigurationBuilder::new().build();
    assert_eq!(format(&config, "\"a\nb\""), "\"a\\nb\"\n");
    assert_eq!(format(&config, "\"a\r\nb\""), "\"a\\r\\nb\"\n");
    assert_eq!(format(&config, "\"a\tb\u{08}\u{0C}\""), "\"a\\tb\\b\\f\"\n");
    assert_eq!(
      format(&config, "\"a\u{00}\u{1B}\u{1F}\""),
      "\"a\\u0000\\u001b\\u001f\"\n"
    );
    assert_eq!(format(&config, "'a\"\n\\'b'"), "\"a\\\"\\n'b\"\n");
    // escape sequences and non-control characters are left alone
    assert_eq!(format(&config, "\"a\\n\\\\\u{7F}\""), "\"a\\n\\\\\u{7F}\"\n");
  }

  fn format(config: &Configuration, text: &str) -> String {
    let output = format_text(Path::new("/file.json"), text, config)
      .unwrap()
      .unwrap_or_else(|| text.to_string());
    // ensure formatting is stable
    assert_eq!(format_text(Path::new("/file.json"), &output, config).unwrap(), None);
    output
  }
}
