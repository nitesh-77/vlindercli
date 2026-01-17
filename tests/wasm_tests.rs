use wasmtime::*;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn wasm_yield_to_with_text_protocol() {
    // Track what agent was called with what input
    let calls = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let calls_clone = calls.clone();

    let mut config = Config::new();
    config.async_support(true);
    let engine = Engine::new(&config).unwrap();

    let mut linker = Linker::<()>::new(&engine);

    // yield_to(agent_ptr, agent_len, input_ptr, input_len, response_buf_ptr, response_buf_len) -> response_len
    // Host reads strings from wasm memory, simulates agent call, writes response to provided buffer
    linker.func_wrap_async(
        "host",
        "yield_to",
        move |mut caller: Caller<'_, ()>,
              (agent_ptr, agent_len, input_ptr, input_len, resp_buf_ptr, resp_buf_len): (i32, i32, i32, i32, i32, i32)| {
            let calls = calls_clone.clone();
            Box::new(async move {
                // Read agent name from wasm memory
                let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
                let data = memory.data(&caller);

                let agent = std::str::from_utf8(
                    &data[agent_ptr as usize..(agent_ptr + agent_len) as usize]
                ).unwrap().to_string();

                let input = std::str::from_utf8(
                    &data[input_ptr as usize..(input_ptr + input_len) as usize]
                ).unwrap().to_string();

                // Record the call
                calls.lock().unwrap().push((agent.clone(), input.clone()));

                // Simulate agent response
                let response = format!("response from {} for: {}", agent, input);
                let response_bytes = response.as_bytes();
                let write_len = std::cmp::min(response_bytes.len(), resp_buf_len as usize);

                // Write response to provided buffer
                let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
                memory.data_mut(&mut caller)[resp_buf_ptr as usize..resp_buf_ptr as usize + write_len]
                    .copy_from_slice(&response_bytes[..write_len]);

                // Return actual response length
                write_len as i32
            })
        },
    ).unwrap();

    // WAT module that calls yield_to and captures the response
    let module = Module::new(&engine, r#"
        (module
            (import "host" "yield_to" (func $yield_to (param i32 i32 i32 i32 i32 i32) (result i32)))

            (memory (export "memory") 1)

            ;; Response buffer at offset 256, size 256
            ;; Store actual response length
            (global $last_response_len (mut i32) (i32.const 0))
            (func (export "get_last_response") (result i32 i32)
                (i32.const 256)  ;; response buffer ptr
                (global.get $last_response_len)
            )

            ;; Data section: agent name and input at known offsets
            (data (i32.const 0) "note-taker")      ;; 10 bytes at offset 0
            (data (i32.const 16) "get all notes")  ;; 13 bytes at offset 16

            (func (export "run")
                ;; Call yield_to("note-taker", "get all notes", response_buf, buf_size)
                (global.set $last_response_len
                    (call $yield_to
                        (i32.const 0)    ;; agent_ptr
                        (i32.const 10)   ;; agent_len
                        (i32.const 16)   ;; input_ptr
                        (i32.const 13)   ;; input_len
                        (i32.const 256)  ;; response_buf_ptr
                        (i32.const 256)  ;; response_buf_len
                    )
                )
            )
        )
    "#).unwrap();

    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate_async(&mut store, &module).await.unwrap();
    let run = instance.get_typed_func::<(), ()>(&mut store, "run").unwrap();

    run.call_async(&mut store, ()).await.unwrap();

    // Verify the call was made with correct strings
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "note-taker");
    assert_eq!(calls[0].1, "get all notes");

    // Verify response was written back
    let get_last_response = instance.get_typed_func::<(), (i32, i32)>(&mut store, "get_last_response").unwrap();
    let (resp_ptr, resp_len) = get_last_response.call_async(&mut store, ()).await.unwrap();

    let memory = instance.get_memory(&mut store, "memory").unwrap();
    let response = std::str::from_utf8(
        &memory.data(&store)[resp_ptr as usize..(resp_ptr + resp_len) as usize]
    ).unwrap();

    assert_eq!(response, "response from note-taker for: get all notes");
}
