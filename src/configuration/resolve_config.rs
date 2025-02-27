use super::builder::ConfigurationBuilder;
use super::types::TrailingCommaKind;
use super::Configuration;
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
/// let mut config_map = ConfigKeyMap::new(); // get a collection of key value pairs from somewhere
/// let global_config_result = resolve_global_config(&mut config_map);
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
      TrailingCommaKind::Jsonc,
      &mut diagnostics,
    ),
    json_trailing_comma_files: get_trailing_comma_files(&mut config, "jsonTrailingCommaFiles", &mut diagnostics),
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

fn get_trailing_comma_files(
  config: &mut ConfigKeyMap,
  key: &str,
  diagnostics: &mut Vec<ConfigurationDiagnostic>,
) -> Vec<String> {
  let mut entries = Vec::with_capacity(0);
  if let Some(values) = config.shift_remove(key) {
    if let ConfigKeyValue::Array(values) = values {
      entries = Vec::with_capacity(values.len() * 2);
      for (i, value) in values.into_iter().enumerate() {
        if let ConfigKeyValue::String(value) = value {
          if value.starts_with("./") {
            diagnostics.push(ConfigurationDiagnostic {
              property_name: key.to_string(),
              message: format!(
                "Element at index {} starting with dot slash (./) is not supported. Remove the leading dot slash.",
                i
              ),
            });
          } else if value.chars().any(|c| matches!(c, '\\' | '/')) {
            let value = if value.starts_with('/') || value.starts_with('\\') {
              value
            } else {
              format!("/{}", value)
            };
            entries.push(value.replace('/', "\\"));
            entries.push(value.replace('\\', "/"));
          } else {
            entries.push(format!("/{}", value));
            entries.push(format!("\\{}", value));
          }
        } else {
          diagnostics.push(ConfigurationDiagnostic {
            property_name: key.to_string(),
            message: format!("Expected element at index {} to be a string.", i),
          });
        }
      }
    } else {
      diagnostics.push(ConfigurationDiagnostic {
        property_name: key.to_string(),
        message: "Expected an array.".to_string(),
      });
    }
  }
  entries
}

#[cfg(test)]
mod test {
  use dprint_core::configuration::ConfigKeyMap;
  use dprint_core::configuration::ConfigKeyValue;
  use dprint_core::configuration::GlobalConfiguration;

  use super::resolve_config;

  #[test]
  fn json_trailing_comma_files() {
    let global_config = GlobalConfiguration::default();
    {
      let result = resolve_config(
        ConfigKeyMap::from([(
          "jsonTrailingCommaFiles".to_string(),
          ConfigKeyValue::Array(vec![ConfigKeyValue::String("test.json".to_string())]),
        )]),
        &global_config,
      );
      assert!(result.diagnostics.is_empty());
      assert_eq!(
        result.config.json_trailing_comma_files,
        vec!["/test.json".to_string(), "\\test.json".to_string(),]
      );
    }
    {
      let result = resolve_config(
        ConfigKeyMap::from([(
          "jsonTrailingCommaFiles".to_string(),
          ConfigKeyValue::Array(vec![ConfigKeyValue::String("./test.json".to_string())]),
        )]),
        &global_config,
      );
      assert_eq!(
        result.diagnostics[0].message,
        "Element at index 0 starting with dot slash (./) is not supported. Remove the leading dot slash."
      );
    }
    {
      let result = resolve_config(
        ConfigKeyMap::from([(
          "jsonTrailingCommaFiles".to_string(),
          ConfigKeyValue::Array(vec![ConfigKeyValue::Number(5)]),
        )]),
        &global_config,
      );
      assert_eq!(
        result.diagnostics[0].message,
        "Expected element at index 0 to be a string."
      );
    }
  }
}
