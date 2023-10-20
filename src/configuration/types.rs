use dprint_core::configuration::*;
use dprint_core::generate_str_to_from;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrailingCommaKind {
  /// Always format with trailing commas. Beware: trailing commas can cause many JSON parsers to fail.
  Always,
  /// Use trailing commas in JSONC files and do not use trailing commas in JSON files. (Default)
  Jsonc,
  /// Keep the trailing comma if it exists.
  Maintain,
  /// Never format with trailing commas.
  Never,
}

generate_str_to_from![
  TrailingCommaKind,
  [Always, "always"],
  [Maintain, "maintain"],
  [Never, "never"],
  [Jsonc, "jsonc"]
];
