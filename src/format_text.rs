use std::path::Path;

use anyhow::Result;
use anyhow::bail;

use super::configuration::Configuration;
use crate::streaming::format_streaming;

pub fn format_text(path: &Path, text: &str, config: &Configuration) -> Result<Option<String>> {
  let result = format_text_inner(path, text, config)?;
  if result == text { Ok(None) } else { Ok(Some(result)) }
}

fn format_text_inner(path: &Path, text: &str, config: &Configuration) -> Result<String> {
  let text = strip_bom(text);
  let is_jsonc = is_jsonc_file(path, config);
  match format_streaming(text.as_bytes(), config, is_jsonc) {
    // Input is valid UTF-8 (`&str`) and the formatter only rearranges/copies its
    // bytes, so the output is always valid UTF-8.
    Ok(bytes) => Ok(String::from_utf8(bytes).expect("formatted output is valid UTF-8")),
    Err(err) => bail!(dprint_core::formatting::utils::string_utils::format_diagnostic(
      Some((err.start, err.end)),
      err.message,
      text,
    )),
  }
}

#[cfg(feature = "tracing")]
pub fn trace_file(text: &str, config: &Configuration) -> dprint_core::formatting::TracingResult {
  use dprint_core::configuration::resolve_new_line_kind;
  use dprint_core::formatting::PrintOptions;
  use jsonc_parser::CollectOptions;
  use jsonc_parser::CommentCollectionStrategy;
  use jsonc_parser::parse_to_ast;

  let parse_result = parse_to_ast(
    text,
    &CollectOptions {
      comments: CommentCollectionStrategy::Separate,
      tokens: true,
    },
    &Default::default(),
  )
  .unwrap();

  dprint_core::formatting::trace_printing(
    || crate::generation::generate(parse_result, text, config, false),
    PrintOptions {
      indent_width: config.indent_width,
      max_width: config.line_width,
      use_tabs: config.use_tabs,
      new_line_text: resolve_new_line_kind(text, config.new_line_kind),
    },
  )
}

fn strip_bom(text: &str) -> &str {
  text.strip_prefix("\u{FEFF}").unwrap_or(text)
}

fn is_jsonc_file(path: &Path, config: &Configuration) -> bool {
  fn has_jsonc_extension(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
      return ext.to_string_lossy().to_ascii_lowercase() == "jsonc";
    }

    false
  }

  fn is_special_json_file(path: &Path, config: &Configuration) -> bool {
    let path = path.to_string_lossy();
    for file_name in &config.json_trailing_comma_files {
      if path.ends_with(file_name) {
        return true;
      }
    }

    false
  }

  has_jsonc_extension(path) || is_special_json_file(path, config)
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use crate::configuration::ConfigurationBuilder;

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
  fn should_strip_bom() {
    for input_text in ["\u{FEFF}{}", "\u{FEFF}{ }"] {
      let global_config = GlobalConfiguration::default();
      let config = resolve_config(ConfigKeyMap::new(), &global_config).config;
      let output_text = format_text(Path::new("."), input_text, &config).unwrap().unwrap();
      assert_eq!(output_text, "{}\n");
    }
  }
}
