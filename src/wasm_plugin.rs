use dprint_core::configuration::ConfigKeyMap;
use dprint_core::configuration::GlobalConfiguration;
use dprint_core::configuration::ResolveConfigurationResult;
use dprint_core::generate_plugin_code;
use dprint_core::plugins::FileMatchingInfo;
use dprint_core::plugins::FormatResult;
use dprint_core::plugins::PluginInfo;
use dprint_core::plugins::SyncPluginHandler;
use dprint_core::plugins::SyncPluginInfo;
use std::path::Path;

use super::configuration::resolve_config;
use super::configuration::Configuration;

struct JsonPluginHandler;

impl SyncPluginHandler<Configuration> for JsonPluginHandler {
  fn resolve_config(
    &mut self,
    config: ConfigKeyMap,
    global_config: &GlobalConfiguration,
  ) -> ResolveConfigurationResult<Configuration> {
    resolve_config(config, global_config)
  }

  fn plugin_info(&mut self) -> SyncPluginInfo {
    let version = env!("CARGO_PKG_VERSION").to_string();
    SyncPluginInfo {
      info: PluginInfo {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: version.clone(),
        config_key: "json".to_string(),
        help_url: "https://dprint.dev/plugins/json".to_string(),
        config_schema_url: format!(
          "https://plugins.dprint.dev/dprint/dprint-plugin-json/{}/schema.json",
          version
        ),
        update_url: Some("https://plugins.dprint.dev/dprint/dprint-plugin-json/latest.json".to_string()),
      },
      file_matching: FileMatchingInfo {
        file_extensions: vec!["json".to_string(), "jsonc".to_string()],
        file_names: vec![],
      },
    }
  }

  fn license_text(&mut self) -> String {
    std::str::from_utf8(include_bytes!("../LICENSE")).unwrap().into()
  }

  fn format(
    &mut self,
    file_path: &Path,
    file_bytes: Vec<u8>,
    config: &Configuration,
    _format_with_host: impl FnMut(&Path, Vec<u8>, &ConfigKeyMap) -> FormatResult,
  ) -> FormatResult {
    let file_text = String::from_utf8(file_bytes)?;
    super::format_text(file_path, &file_text, config).map(|maybe_text| maybe_text.map(|t| t.into_bytes()))
  }
}

generate_plugin_code!(JsonPluginHandler, JsonPluginHandler);
