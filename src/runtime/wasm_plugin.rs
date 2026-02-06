//! Extism plugin plumbing for WASM agent execution.
//!
//! All extism-specific code lives here:
//! - Host function creation (send, get_prompts)
//! - Plugin loading, manifest configuration, execution
//!
//! The agent wiring logic (validation, routing, request lifecycle)
//! lives in `SendFunctionData::handle_send`, not here.

use extism::{CurrentPlugin, Function, Manifest, Plugin, UserData, Val, Wasm};

use super::wasm::SendFunctionData;

/// Load and execute a WASM plugin.
///
/// Creates the extism plugin from the WASM file, registers host functions,
/// and calls the "process" entry point.
pub(super) fn run_plugin(wasm_path: &str, data: SendFunctionData, payload: &[u8]) -> Vec<u8> {
    let functions = [
        make_send_function(data),
        make_get_prompts_function(),
    ];

    let wasm = Wasm::file(wasm_path);
    let manifest = Manifest::new([wasm]).with_allowed_hosts(["*".to_string()].into_iter());
    let mut plugin = Plugin::new(&manifest, functions, true).unwrap();

    plugin.call::<_, Vec<u8>>("process", payload).unwrap()
}

fn write_output(plugin: &mut CurrentPlugin, outputs: &mut [Val], response: &[u8]) -> Result<(), extism::Error> {
    let handle = plugin.memory_new(response)?;
    outputs[0] = Val::I64(handle.offset() as i64);
    Ok(())
}

/// Host function: send(payload) -> response
///
/// Extism plumbing that delegates to `SendFunctionData::handle_send`.
fn make_send_function(data: SendFunctionData) -> Function {
    Function::new(
        "send",
        [extism::PTR], // payload (SdkMessage JSON)
        [extism::PTR], // response payload
        UserData::new(data),
        |plugin, inputs, outputs, user_data| {
            let payload: Vec<u8> = plugin.memory_get_val(&inputs[0])?;

            let data = user_data.get().unwrap();
            let data = data.lock().unwrap();

            let response = data.handle_send(payload)
                .map_err(|e| extism::Error::msg(e))?;

            write_output(plugin, outputs, &response)
        },
    )
}

/// Host function: get_prompts() -> JSON string
///
/// Returns prompt overrides from the agent's manifest.
/// Returns empty JSON object if no overrides defined.
fn make_get_prompts_function() -> Function {
    Function::new(
        "get_prompts",
        [],           // no inputs
        [extism::PTR], // JSON string
        UserData::new(()),
        |plugin, _inputs, outputs, _user_data| {
            // Return empty object - prompts default to compiled-in values
            write_output(plugin, outputs, b"{}")
        },
    )
}
