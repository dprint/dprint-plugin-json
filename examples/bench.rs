// Benchmark for issue #30. `format_text` now formats off a byte stream with no
// parser at all — this measures its throughput.
//
// Run: cargo run --release --example bench
use std::path::Path;
use std::time::Instant;

use dprint_core::configuration::*;
use dprint_plugin_json::configuration::resolve_config;
use dprint_plugin_json::format_text;

fn gen_input(n: usize, pretty: bool) -> String {
  // Array of n objects, each with a few keys of mixed types.
  let mut s = String::from("[");
  for i in 0..n {
    if i > 0 {
      s.push(',');
    }
    if pretty {
      s.push_str("\n  ");
    }
    s.push_str(&format!(
      r#"{{"id":{i},"name":"item-{i}","active":{},"tags":["a","b","c"],"score":{}.5,"nested":{{"x":1,"y":2,"z":[1,2,3,4,5]}}}}"#,
      i % 2 == 0,
      i
    ));
  }
  if pretty {
    s.push('\n');
  }
  s.push(']');
  s
}

fn time<F: Fn()>(label: &str, iters: u32, f: F) -> f64 {
  // warmup
  f();
  let start = Instant::now();
  for _ in 0..iters {
    f();
  }
  let per = start.elapsed().as_secs_f64() * 1000.0 / iters as f64;
  println!("  {label:<28} {per:>8.3} ms/iter");
  per
}

fn main() {
  let global = GlobalConfiguration::default();
  let config = resolve_config(ConfigKeyMap::new(), &global).config;
  let path = Path::new("file.json");

  for &(n, iters) in &[(1_000usize, 200u32), (20_000, 20)] {
    for &pretty in &[false, true] {
      let input = gen_input(n, pretty);
      let kind = if pretty { "pretty" } else { "minified" };
      println!("\n== n={n} {kind} ({} bytes) ==", input.len());
      time("format_text (stream, no parse)", iters, || {
        std::hint::black_box(format_text(path, &input, &config).unwrap());
      });
    }
  }
}
