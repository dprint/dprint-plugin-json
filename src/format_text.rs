use std::path::Path;

use anyhow::bail;
use anyhow::Result;
use dprint_core::configuration::resolve_new_line_kind;
use dprint_core::formatting::PrintOptions;
use dprint_core::plugins::FormatResult;
use jsonc_parser::parse_to_ast;
use jsonc_parser::CollectOptions;
use jsonc_parser::ParseResult;

use super::configuration::Configuration;
use super::generation::generate;

pub fn format_text(path: &Path, text: &str, config: &Configuration) -> FormatResult {
  let parse_result = parse(text)?;
  let is_jsonc = is_jsonc_file(path);
  let result = dprint_core::formatting::format(
    || generate(parse_result, text, config, is_jsonc),
    config_to_print_options(text, config),
  );
  if result == text {
    Ok(None)
  } else {
    Ok(Some(result))
  }
}

#[cfg(feature = "tracing")]
pub fn trace_file(text: &str, config: &Configuration) -> dprint_core::formatting::TracingResult {
  let parse_result = parse(text).unwrap();

  dprint_core::formatting::trace_printing(
    || generate(parse_result, text, config),
    config_to_print_options(text, config),
  )
}

fn parse(text: &str) -> Result<ParseResult<'_>> {
  let parse_result = parse_to_ast(
    text,
    &CollectOptions {
      comments: true,
      tokens: true,
    },
    &Default::default(),
  );
  match parse_result {
    Ok(result) => Ok(result),
    Err(err) => bail!(dprint_core::formatting::utils::string_utils::format_diagnostic(
      Some((err.range.start, err.range.end)),
      &err.message,
      text,
    )),
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

fn is_jsonc_file(path: &Path) -> bool {
  fn has_jsonc_extension(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
      return ext.to_string_lossy().to_ascii_lowercase() == "jsonc";
    }

    false
  }

  static SPECIAL_JSON_FILES: [&str; 1] = ["tsconfig.json"];
  static SPECIAL_JSON_DIRECTORIES: [&str; 1] = [".vscode"];

  fn is_special_json_file(path: &Path) -> bool {
    if let Some(file_name) = path.file_name() {
      if SPECIAL_JSON_FILES.contains(&file_name.to_string_lossy().as_ref()) {
        return true;
      }
    }

    if let Some(parent_dir_name) = path.parent().and_then(|p| p.file_name()) {
      if SPECIAL_JSON_DIRECTORIES.contains(&parent_dir_name.to_string_lossy().as_ref()) {
        return true;
      }
    }

    false
  }

  has_jsonc_extension(path) || is_special_json_file(path)
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

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
      "Line 1, column 3: Text cannot contain more than one JSON value\n\n  {},"
    );
  }

  #[test]
  fn test_is_jsonc_file() {
    assert!(!is_jsonc_file(&PathBuf::from("asdf.json")));
    assert!(is_jsonc_file(&PathBuf::from("asdf.jsonc")));
    assert!(is_jsonc_file(&PathBuf::from("ASDF.JSONC")));
    assert!(is_jsonc_file(&PathBuf::from("tsconfig.json")));
    assert!(is_jsonc_file(&PathBuf::from("test/.vscode/settings.json")));
    assert!(!is_jsonc_file(&PathBuf::from("test/vscode/settings.json")));
  }
}
