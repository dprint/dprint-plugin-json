use super::builder::ConfigurationBuilder;
use super::Configuration;
use super::types::TrailingCommaKind;
use dprint_core::configuration::*;

/// Resolves configuration from a collection of key value strings.
///
/// # Example
///
/// ```
/// use dprint_core::configuration::ConfigKeyMap;
/// use dprint_core::configuration::resolve_global_config;
/// use dprint_plugin_json::configuration::resolve_config;
///
/// let config_map = ConfigKeyMap::new(); // get a collection of key value pairs from somewhere
/// let global_config_result = resolve_global_config(config_map, &Default::default());
///
/// // check global_config_result.diagnostics here...
///
/// let jsonc_config_map = ConfigKeyMap::new(); // get a collection of k/v pairs from somewhere
/// let config_result = resolve_config(
///     jsonc_config_map,
///     &global_config_result.config
/// );
///
/// // check config_result.diagnostics here and use config_result.config
/// ```
pub fn resolve_config(
  config: ConfigKeyMap,
  global_config: &GlobalConfiguration,
) -> ResolveConfigurationResult<Configuration> {
  let mut diagnostics = Vec::new();
  let mut config = config;

  if get_value(&mut config, "deno", false, &mut diagnostics) {
    fill_deno_config(&mut config);
  }

  let prefer_single_line = get_value(&mut config, "preferSingleLine", false, &mut diagnostics);

  let resolved_config = Configuration {
    line_width: get_value(
      &mut config,
      "lineWidth",
      global_config
        .line_width
        .unwrap_or(RECOMMENDED_GLOBAL_CONFIGURATION.line_width),
      &mut diagnostics,
    ),
    use_tabs: get_value(
      &mut config,
      "useTabs",
      global_config
        .use_tabs
        .unwrap_or(RECOMMENDED_GLOBAL_CONFIGURATION.use_tabs),
      &mut diagnostics,
    ),
    indent_width: get_value(
      &mut config,
      "indentWidth",
      global_config.indent_width.unwrap_or(2),
      &mut diagnostics,
    ),
    new_line_kind: get_value(
      &mut config,
      "newLineKind",
      global_config
        .new_line_kind
        .unwrap_or(RECOMMENDED_GLOBAL_CONFIGURATION.new_line_kind),
      &mut diagnostics,
    ),
    comment_line_force_space_after_slashes: get_value(
      &mut config,
      "commentLine.forceSpaceAfterSlashes",
      true,
      &mut diagnostics,
    ),
    ignore_node_comment_text: get_value(
      &mut config,
      "ignoreNodeCommentText",
      String::from("dprint-ignore"),
      &mut diagnostics,
    ),
    array_prefer_single_line: get_value(
      &mut config,
      "array.preferSingleLine",
      prefer_single_line,
      &mut diagnostics,
    ),
    object_prefer_single_line: get_value(
      &mut config,
      "object.preferSingleLine",
      prefer_single_line,
      &mut diagnostics,
    ),
    trailing_commas: get_value(
      &mut config,
      "trailingCommas",
      TrailingCommaKind::OnlyInJSONC,
      &mut diagnostics,
    ),
  };

  diagnostics.extend(get_unknown_property_diagnostics(config));

  ResolveConfigurationResult {
    config: resolved_config,
    diagnostics,
  }
}

fn fill_deno_config(config: &mut ConfigKeyMap) {
  for (key, value) in ConfigurationBuilder::new().deno().config.iter() {
    if !config.contains_key(key) {
      config.insert(key.clone(), value.clone());
    }
  }
}
