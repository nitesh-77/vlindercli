use extism_pdk::*;

#[plugin_fn]
pub fn process(input: String) -> FnResult<String> {
    Ok(input.to_uppercase())
}
