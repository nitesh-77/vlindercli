use wasmtime::*;
use std::sync::{Arc, Mutex};

#[test]
fn wasm_can_load_and_run() {
    // Simple wasm that adds two numbers
    let engine = Engine::default();
    let module = Module::new(&engine, r#"
        (module
            (func (export "add") (param i32 i32) (result i32)
                local.get 0
                local.get 1
                i32.add
            )
        )
    "#).unwrap();

    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).unwrap();
    let add = instance.get_typed_func::<(i32, i32), i32>(&mut store, "add").unwrap();

    let result = add.call(&mut store, (2, 3)).unwrap();
    assert_eq!(result, 5);
}

#[test]
fn wasm_can_call_host_function() {
    let yielded = Arc::new(Mutex::new(Vec::new()));
    let yielded_clone = yielded.clone();

    let engine = Engine::default();
    let mut linker = Linker::new(&engine);

    // Host function that wasm can call to "yield"
    linker.func_wrap("host", "yield_op", move |agent: i32| {
        yielded_clone.lock().unwrap().push(agent);
    }).unwrap();

    let module = Module::new(&engine, r#"
        (module
            (import "host" "yield_op" (func $yield_op (param i32)))
            (func (export "run")
                i32.const 42
                call $yield_op
                i32.const 43
                call $yield_op
            )
        )
    "#).unwrap();

    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let run = instance.get_typed_func::<(), ()>(&mut store, "run").unwrap();

    run.call(&mut store, ()).unwrap();

    assert_eq!(*yielded.lock().unwrap(), vec![42, 43]);
}

#[tokio::test]
async fn wasm_can_suspend_and_resume() {
    // Track the sequence of events
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let mut config = Config::new();
    config.async_support(true);
    let engine = Engine::new(&config).unwrap();

    let mut linker = Linker::new(&engine);

    // Host function that suspends wasm and lets runtime do work
    linker.func_wrap_async("host", "yield_op", move |_caller: Caller<'_, ()>, (op,): (i32,)| {
        let events = events_clone.clone();
        Box::new(async move {
            events.lock().unwrap().push(format!("wasm yielded: {}", op));
            // Simulate runtime doing work while wasm is suspended
            events.lock().unwrap().push("runtime working".to_string());
            events.lock().unwrap().push(format!("resuming wasm after: {}", op));
        })
    }).unwrap();

    let module = Module::new(&engine, r#"
        (module
            (import "host" "yield_op" (func $yield_op (param i32)))
            (func (export "run")
                i32.const 1
                call $yield_op
                i32.const 2
                call $yield_op
            )
        )
    "#).unwrap();

    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate_async(&mut store, &module).await.unwrap();
    let run = instance.get_typed_func::<(), ()>(&mut store, "run").unwrap();

    events.lock().unwrap().push("starting wasm".to_string());
    run.call_async(&mut store, ()).await.unwrap();
    events.lock().unwrap().push("wasm completed".to_string());

    let events = events.lock().unwrap();
    assert_eq!(events[0], "starting wasm");
    assert_eq!(events[1], "wasm yielded: 1");
    assert_eq!(events[2], "runtime working");
    assert_eq!(events[3], "resuming wasm after: 1");
    assert_eq!(events[4], "wasm yielded: 2");
    assert_eq!(events[5], "runtime working");
    assert_eq!(events[6], "resuming wasm after: 2");
    assert_eq!(events[7], "wasm completed");
}
