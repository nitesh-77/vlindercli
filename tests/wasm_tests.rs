use wasmtime::*;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

#[tokio::test]
async fn wasm_async_yield_with_parallel_operations() {
    // Track operations and their order
    let operations = Arc::new(Mutex::new(Vec::<String>::new()));
    let operations_clone = operations.clone();
    let operations_clone2 = operations.clone();

    // Pending results by handle
    let pending: Arc<Mutex<HashMap<i32, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let pending_for_yield = pending.clone();
    let pending_for_wait = pending.clone();

    let next_handle = Arc::new(Mutex::new(1i32));
    let next_handle_clone = next_handle.clone();

    let mut config = Config::new();
    config.async_support(true);
    let engine = Engine::new(&config).unwrap();

    let mut linker = Linker::<()>::new(&engine);

    // yield_to_async(agent_ptr, agent_len, input_ptr, input_len) -> handle
    // Returns immediately with handle, schedules work
    linker.func_wrap(
        "host",
        "yield_to_async",
        move |mut caller: Caller<'_, ()>, agent_ptr: i32, agent_len: i32, input_ptr: i32, input_len: i32| -> i32 {
            let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
            let data = memory.data(&caller);

            let agent = std::str::from_utf8(
                &data[agent_ptr as usize..(agent_ptr + agent_len) as usize]
            ).unwrap().to_string();

            let input = std::str::from_utf8(
                &data[input_ptr as usize..(input_ptr + input_len) as usize]
            ).unwrap().to_string();

            // Get handle
            let mut handle_guard = next_handle_clone.lock().unwrap();
            let handle = *handle_guard;
            *handle_guard += 1;

            // Record that async operation was issued
            operations_clone.lock().unwrap().push(format!("async_yield: {} {}", agent, input));

            // Simulate: result is computed immediately but stored for later retrieval
            let response = format!("response from {} for: {}", agent, input);
            pending_for_yield.lock().unwrap().insert(handle, response);

            handle
        },
    ).unwrap();

    // wait(handle, response_buf_ptr, response_buf_len) -> response_len
    // Blocks until result ready, writes to buffer
    linker.func_wrap_async(
        "host",
        "wait",
        move |mut caller: Caller<'_, ()>, (handle, resp_buf_ptr, resp_buf_len): (i32, i32, i32)| {
            let pending = pending_for_wait.clone();
            let operations = operations_clone2.clone();
            Box::new(async move {
                operations.lock().unwrap().push(format!("wait: handle {}", handle));

                // Get result for handle
                let response = pending.lock().unwrap().remove(&handle)
                    .expect("No pending result for handle");

                let response_bytes = response.as_bytes();
                let write_len = std::cmp::min(response_bytes.len(), resp_buf_len as usize);

                let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
                memory.data_mut(&mut caller)[resp_buf_ptr as usize..resp_buf_ptr as usize + write_len]
                    .copy_from_slice(&response_bytes[..write_len]);

                write_len as i32
            })
        },
    ).unwrap();

    // WAT module that issues two parallel yields then waits for both
    let module = Module::new(&engine, r#"
        (module
            (import "host" "yield_to_async" (func $yield_to_async (param i32 i32 i32 i32) (result i32)))
            (import "host" "wait" (func $wait (param i32 i32 i32) (result i32)))

            (memory (export "memory") 1)

            ;; Data section
            (data (i32.const 0) "agent-a")       ;; 7 bytes
            (data (i32.const 16) "task a")       ;; 6 bytes
            (data (i32.const 32) "agent-b")      ;; 7 bytes
            (data (i32.const 48) "task b")       ;; 6 bytes

            ;; Response buffers and lengths
            (global $handle_a (mut i32) (i32.const 0))
            (global $handle_b (mut i32) (i32.const 0))
            (global $resp_a_len (mut i32) (i32.const 0))
            (global $resp_b_len (mut i32) (i32.const 0))

            (func (export "get_responses") (result i32 i32 i32 i32)
                (i32.const 256)  ;; resp_a_ptr
                (global.get $resp_a_len)
                (i32.const 512)  ;; resp_b_ptr
                (global.get $resp_b_len)
            )

            (func (export "run")
                ;; Issue both yields first (non-blocking)
                (global.set $handle_a
                    (call $yield_to_async
                        (i32.const 0)    ;; agent_ptr "agent-a"
                        (i32.const 7)    ;; agent_len
                        (i32.const 16)   ;; input_ptr "task a"
                        (i32.const 6)    ;; input_len
                    )
                )
                (global.set $handle_b
                    (call $yield_to_async
                        (i32.const 32)   ;; agent_ptr "agent-b"
                        (i32.const 7)    ;; agent_len
                        (i32.const 48)   ;; input_ptr "task b"
                        (i32.const 6)    ;; input_len
                    )
                )

                ;; Now wait for results
                (global.set $resp_a_len
                    (call $wait
                        (global.get $handle_a)
                        (i32.const 256)  ;; response buffer a
                        (i32.const 256)  ;; buffer size
                    )
                )
                (global.set $resp_b_len
                    (call $wait
                        (global.get $handle_b)
                        (i32.const 512)  ;; response buffer b
                        (i32.const 256)  ;; buffer size
                    )
                )
            )
        )
    "#).unwrap();

    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate_async(&mut store, &module).await.unwrap();
    let run = instance.get_typed_func::<(), ()>(&mut store, "run").unwrap();

    run.call_async(&mut store, ()).await.unwrap();

    // Verify operation order: both yields issued before waits
    let ops = operations.lock().unwrap();
    assert_eq!(ops[0], "async_yield: agent-a task a");
    assert_eq!(ops[1], "async_yield: agent-b task b");
    assert_eq!(ops[2], "wait: handle 1");
    assert_eq!(ops[3], "wait: handle 2");

    // Verify responses
    let get_responses = instance.get_typed_func::<(), (i32, i32, i32, i32)>(&mut store, "get_responses").unwrap();
    let (resp_a_ptr, resp_a_len, resp_b_ptr, resp_b_len) = get_responses.call_async(&mut store, ()).await.unwrap();

    let memory = instance.get_memory(&mut store, "memory").unwrap();
    let resp_a = std::str::from_utf8(
        &memory.data(&store)[resp_a_ptr as usize..(resp_a_ptr + resp_a_len) as usize]
    ).unwrap();
    let resp_b = std::str::from_utf8(
        &memory.data(&store)[resp_b_ptr as usize..(resp_b_ptr + resp_b_len) as usize]
    ).unwrap();

    assert_eq!(resp_a, "response from agent-a for: task a");
    assert_eq!(resp_b, "response from agent-b for: task b");
}
