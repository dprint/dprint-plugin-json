#![feature(test)]

extern crate test;

use dprint_core::configuration::*;
use dprint_plugin_json::configuration::resolve_config;
use dprint_plugin_json::*;
use std::fs::read_to_string;
use test::Bencher;

#[bench]
fn single_line_800kb_json(b: &mut Bencher) {
  bench_format(b, &get_single_line_800kb_json());
}

fn bench_format(b: &mut Bencher, json_text: &str) {
  b.iter(|| {
    let global_config = resolve_global_config(ConfigKeyMap::new(), &Default::default()).config;
    let config = resolve_config(ConfigKeyMap::new(), &global_config).config;
    format_text(json_text, &config).unwrap()
  });
}

fn get_single_line_800kb_json() -> String {
  read_to_string("benches/data/single_line_800kb.json").unwrap()
}
