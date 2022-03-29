use dprint_core::configuration::{resolve_global_config, ConfigKeyMap, ConfigKeyValue, GlobalConfiguration, NewLineKind};

use super::*;

/// Formatting configuration builder.
///
/// # Example
///
/// ```
/// use dprint_plugin_json::configuration::*;
///
/// let config = ConfigurationBuilder::new()
///     .line_width(80)
///     .build();
/// ```
#[derive(Default)]
pub struct ConfigurationBuilder {
  pub(super) config: ConfigKeyMap,
  global_config: Option<GlobalConfiguration>,
}

impl ConfigurationBuilder {
  /// Constructs a new configuration builder.
  pub fn new() -> ConfigurationBuilder {
    Default::default()
  }

  /// Gets the final configuration that can be used to format a file.
  pub fn build(&self) -> Configuration {
    if let Some(global_config) = &self.global_config {
      resolve_config(self.config.clone(), global_config).config
    } else {
      let global_config = resolve_global_config(ConfigKeyMap::new(), &Default::default()).config;
      resolve_config(self.config.clone(), &global_config).config
    }
  }

  /// Set the global configuration.
  pub fn global_config(&mut self, global_config: GlobalConfiguration) -> &mut Self {
    self.global_config = Some(global_config);
    self
  }

  /// The width of a line the printer will try to stay under. Note that the printer may exceed this width in certain cases.
  /// Default: 120
  pub fn line_width(&mut self, value: u32) -> &mut Self {
    self.insert("lineWidth", (value as i32).into())
  }

  /// Whether to use tabs (true) or spaces (false).
  ///
  /// Default: `false`
  pub fn use_tabs(&mut self, value: bool) -> &mut Self {
    self.insert("useTabs", value.into())
  }

  /// The number of columns for an indent.
  ///
  /// Default: `2`
  pub fn indent_width(&mut self, value: u8) -> &mut Self {
    self.insert("indentWidth", (value as i32).into())
  }

  /// The kind of newline to use.
  /// Default: `NewLineKind::LineFeed`
  pub fn new_line_kind(&mut self, value: NewLineKind) -> &mut Self {
    self.insert("newLineKind", value.to_string().into())
  }

  /// The kind of newline to use.
  /// Default: true
  pub fn comment_line_force_space_after_slashes(&mut self, value: bool) -> &mut Self {
    self.insert("commentLine.forceSpaceAfterSlashes", value.into())
  }

  /// The text to use for an ignore comment (ex. `// dprint-ignore`).
  ///
  /// Default: `"dprint-ignore"`
  pub fn ignore_node_comment_text(&mut self, value: &str) -> &mut Self {
    self.insert("ignoreNodeCommentText", value.into())
  }

  /// Whether to make objects and arrays collapse to a single line when below the line width.
  /// Default: false
  pub fn prefer_single_line(&mut self, value: bool) -> &mut Self {
    self.insert("preferSingleLine", value.into())
  }

  /// Whether to make arrays collapse to a single line when below the line width.
  /// Default: false
  pub fn array_prefer_single_line(&mut self, value: bool) -> &mut Self {
    self.insert("array.preferSingleLine", value.into())
  }

  /// Whether to make ojects collapse to a single line when below the line width.
  /// Default: false
  pub fn object_prefer_single_line(&mut self, value: bool) -> &mut Self {
    self.insert("object.preferSingleLine", value.into())
  }

  /// Sets the configuration to what is used in Deno.
  pub fn deno(&mut self) -> &mut Self {
    self
      .line_width(80)
      .ignore_node_comment_text("deno-fmt-ignore")
      .comment_line_force_space_after_slashes(false)
  }

  #[cfg(test)]
  pub(super) fn get_inner_config(&self) -> ConfigKeyMap {
    self.config.clone()
  }

  fn insert(&mut self, name: &str, value: ConfigKeyValue) -> &mut Self {
    self.config.insert(String::from(name), value);
    self
  }
}

#[cfg(test)]
mod tests {
  use dprint_core::configuration::resolve_global_config;
  use dprint_core::configuration::NewLineKind;

  use super::*;

  #[test]
  fn check_all_values_set() {
    let mut config = ConfigurationBuilder::new();
    config
      .new_line_kind(NewLineKind::CarriageReturnLineFeed)
      .line_width(90)
      .use_tabs(true)
      .indent_width(4)
      .new_line_kind(NewLineKind::CarriageReturnLineFeed)
      .comment_line_force_space_after_slashes(false)
      .prefer_single_line(true)
      .array_prefer_single_line(true)
      .object_prefer_single_line(false)
      .ignore_node_comment_text("deno-fmt-ignore");

    let inner_config = config.get_inner_config();
    assert_eq!(inner_config.len(), 9);
    let diagnostics = resolve_config(inner_config, &resolve_global_config(ConfigKeyMap::new(), &Default::default()).config).diagnostics;
    assert_eq!(diagnostics.len(), 0);
  }

  #[test]
  fn handle_global_config() {
    let mut global_config = ConfigKeyMap::new();
    global_config.insert(String::from("lineWidth"), 90.into());
    global_config.insert(String::from("newLineKind"), "crlf".into());
    global_config.insert(String::from("useTabs"), true.into());
    let global_config = resolve_global_config(global_config, &Default::default()).config;
    let mut config_builder = ConfigurationBuilder::new();
    let config = config_builder.global_config(global_config).build();
    assert_eq!(config.line_width, 90);
    assert_eq!(config.new_line_kind == NewLineKind::CarriageReturnLineFeed, true);
  }

  #[test]
  fn use_json_defaults_when_global_not_set() {
    let global_config = GlobalConfiguration::default();
    let mut config_builder = ConfigurationBuilder::new();
    let config = config_builder.global_config(global_config).build();
    assert_eq!(config.indent_width, 2); // this is different
    assert_eq!(config.new_line_kind == NewLineKind::LineFeed, true);
  }

  #[test]
  fn support_deno_config() {
    let mut config_builder = ConfigurationBuilder::new();
    let config = config_builder.deno().build();
    assert_eq!(config.indent_width, 2);
    assert_eq!(config.line_width, 80);
    assert_eq!(config.new_line_kind == NewLineKind::LineFeed, true);
    assert_eq!(config.use_tabs, false);
    assert_eq!(config.comment_line_force_space_after_slashes, false);
    assert_eq!(config.ignore_node_comment_text, "deno-fmt-ignore");
    assert_eq!(config.array_prefer_single_line, false);
    assert_eq!(config.object_prefer_single_line, false);
  }

  #[test]
  fn support_prefer_single_line_config() {
    let mut config_builder = ConfigurationBuilder::new();
    let config = config_builder.prefer_single_line(true).build();
    assert_eq!(config.array_prefer_single_line, true);
    assert_eq!(config.object_prefer_single_line, true);
  }
}
