pub mod configuration;
mod format_text;
// Only the tracing path still uses the AST/IR generator; the normal formatter
// is the streaming one.
#[cfg(feature = "tracing")]
mod generation;
pub mod streaming;

pub use format_text::format_text;
pub use streaming::format_streaming;

#[cfg(feature = "tracing")]
pub use format_text::trace_file;

#[cfg(feature = "wasm")]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod wasm_plugin;

#[cfg(feature = "wasm")]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use wasm_plugin::*;
