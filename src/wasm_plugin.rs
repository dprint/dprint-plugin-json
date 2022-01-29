use anyhow::Result;
use dprint_core::configuration::{ConfigKeyMap, GlobalConfiguration, ResolveConfigurationResult};
use dprint_core::generate_plugin_code;
use dprint_core::plugins::{PluginHandler, PluginInfo};
use std::path::Path;

use super::configuration::{resolve_config, Configuration};

struct JsonPluginHandler {}

impl JsonPluginHandler {
  pub const fn new() -> Self {
    JsonPluginHandler {}
  }
}

impl PluginHandler<Configuration> for JsonPluginHandler {
  fn resolve_config(&mut self, config: ConfigKeyMap, global_config: &GlobalConfiguration) -> ResolveConfigurationResult<Configuration> {
    resolve_config(config, global_config)
  }

  fn get_plugin_info(&mut self) -> PluginInfo {
    let version = env!("CARGO_PKG_VERSION").to_string();
    PluginInfo {
      name: env!("CARGO_PKG_NAME").to_string(),
      version: version.clone(),
      config_key: "json".to_string(),
      file_extensions: vec!["json".to_string(), "jsonc".to_string()],
      file_names: vec![],
      help_url: "https://dprint.dev/plugins/json".to_string(),
      config_schema_url: format!("https://plugins.dprint.dev/dprint/dprint-plugin-json/{}/schema.json", version),
      update_url: Some("https://plugins.dprint.dev/dprint/dprint-plugin-json/latest.json".to_string()),
    }
  }

  fn get_license_text(&mut self) -> String {
    std::str::from_utf8(include_bytes!("../LICENSE")).unwrap().into()
  }

  fn format_text(
    &mut self,
    _file_path: &Path,
    file_text: &str,
    config: &Configuration,
    _format_with_host: impl FnMut(&Path, String, &ConfigKeyMap) -> Result<String>,
  ) -> Result<String> {
    super::format_text(file_text, config)
  }
}

// for clearing the configuration in the playground
#[no_mangle]
pub fn reset_config() {
  unsafe {
    RESOLVE_CONFIGURATION_RESULT.get().take();
  }
}

generate_plugin_code!(JsonPluginHandler, JsonPluginHandler::new());
