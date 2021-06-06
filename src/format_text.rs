use dprint_core::formatting::{PrintOptions};
use dprint_core::configuration::resolve_new_line_kind;
use dprint_core::types::ErrBox;
use jsonc_parser::{parse_to_ast, ParseOptions, ParseResult};
use super::configuration::Configuration;
use super::parser::parse_items;

pub fn format_text(text: &str, config: &Configuration) -> Result<String, ErrBox> {
    let parse_result = get_parse_result(text)?;

    Ok(dprint_core::formatting::format(
        || parse_items(parse_result, text, config),
        config_to_print_options(text, config),
    ))
}

#[cfg(feature = "tracing")]
pub fn trace_file(text: &str, config: &Configuration) -> dprint_core::formatting::TracingResult {
    let parse_result = get_parse_result(text).unwrap();

    dprint_core::formatting::trace_printing(
        || parse_items(parse_result, text, config),
        config_to_print_options(text, config),
    )
}

fn get_parse_result<'a>(text: &'a str) -> Result<ParseResult<'a>, String> {
    let parse_result = parse_to_ast(text, &ParseOptions { comments: true, tokens: true });
    match parse_result {
        Ok(result) => Ok(result),
        Err(err) => Err(dprint_core::formatting::utils::string_utils::format_diagnostic(
            Some((err.range.start, err.range.end)),
            &err.message,
            text
        )),
    }
}

fn config_to_print_options(text: &str, config: &Configuration) -> PrintOptions {
    PrintOptions {
        indent_width: config.indent_width,
        max_width: config.line_width,
        use_tabs: config.use_tabs,
        new_line_text: resolve_new_line_kind(text, config.new_line_kind),
    }
}

#[cfg(test)]
mod tests {
    use dprint_core::configuration::*;
    use std::collections::HashMap;
    use super::super::configuration::resolve_config;
    use super::*;

    #[test]
    fn should_error_on_syntax_diagnostic() {
        let global_config = resolve_global_config(HashMap::new()).config;
        let config = resolve_config(HashMap::new(), &global_config).config;
        let message = format_text("{ &*&* }", &config).err().unwrap().to_string();
        assert_eq!(
            message,
            concat!(
                "Line 1, column 3: Unexpected token\n",
                "\n",
                "  { &*&* }\n",
                "    ~"
            )
        );
    }
}
