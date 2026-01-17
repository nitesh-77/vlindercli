use wasmtime::*;
use std::io;

pub struct Model {
    pub path: String,
}

pub struct Behavior {
    pub system_prompt: String,
}

pub trait Executable {
    fn execute(&self, engine: &Engine, input: &str) -> String;
}

pub struct Agent {
    pub name: String,
    pub model: Model,
    pub behavior: Behavior,
    module: Module,
}

#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    Compile(String),
}

impl From<io::Error> for LoadError {
    fn from(e: io::Error) -> Self {
        LoadError::Io(e)
    }
}

impl From<wasmtime::Error> for LoadError {
    fn from(e: wasmtime::Error) -> Self {
        LoadError::Compile(e.to_string())
    }
}

impl Agent {
    pub fn load(
        name: &str,
        wasm_path: &str,
        model: Model,
        behavior: Behavior,
        engine: &Engine,
    ) -> Result<Agent, LoadError> {
        let wasm = std::fs::read(wasm_path)?;
        let module = Module::new(engine, &wasm)?;

        Ok(Agent {
            name: name.to_string(),
            model,
            behavior,
            module,
        })
    }
}

impl Executable for Agent {
    fn execute(&self, engine: &Engine, input: &str) -> String {
        let mut store = Store::new(engine, ());
        let instance = Instance::new(&mut store, &self.module, &[]).unwrap();
        let memory = instance.get_memory(&mut store, "memory").unwrap();

        write_input(&instance, &mut store, &memory, input);
        call_process(&instance, &mut store);
        read_output(&instance, &mut store, &memory)
    }
}

fn write_input(instance: &Instance, store: &mut Store<()>, memory: &Memory, input: &str) {
    let ptr: i32 = instance.get_typed_func(&mut *store, "input_ptr").unwrap().call(&mut *store, ()).unwrap();
    let bytes = input.as_bytes();
    memory.data_mut(&mut *store)[ptr as usize..ptr as usize + bytes.len()].copy_from_slice(bytes);
    instance.get_typed_func::<i32, ()>(&mut *store, "set_input_len").unwrap().call(&mut *store, bytes.len() as i32).unwrap();
}

fn call_process(instance: &Instance, store: &mut Store<()>) {
    instance.get_typed_func::<(), ()>(&mut *store, "process").unwrap().call(&mut *store, ()).unwrap();
}

fn read_output(instance: &Instance, store: &mut Store<()>, memory: &Memory) -> String {
    let ptr: i32 = instance.get_typed_func(&mut *store, "output_ptr").unwrap().call(&mut *store, ()).unwrap();
    let len: i32 = instance.get_typed_func(&mut *store, "output_len").unwrap().call(&mut *store, ()).unwrap();
    std::str::from_utf8(&memory.data(&*store)[ptr as usize..(ptr + len) as usize])
        .unwrap()
        .to_string()
}
