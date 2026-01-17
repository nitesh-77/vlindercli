use extism::{Plugin, Manifest, Wasm};
use std::sync::Mutex;

pub struct Model {
    pub path: String,
}

pub struct Behavior {
    pub system_prompt: String,
}

pub struct Agent {
    pub name: String,
    pub model: Model,
    pub behavior: Behavior,
    plugin: Mutex<Plugin>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Plugin(String),
}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

impl From<extism::Error> for LoadError {
    fn from(e: extism::Error) -> Self {
        LoadError::Plugin(e.to_string())
    }
}

impl Agent {
    pub fn load(
        name: &str,
        wasm_path: &str,
        model: Model,
        behavior: Behavior,
    ) -> Result<Agent, LoadError> {
        let wasm = Wasm::file(wasm_path);
        let manifest = Manifest::new([wasm]);
        let plugin = Plugin::new(&manifest, [], true)?;

        Ok(Agent {
            name: name.to_string(),
            model,
            behavior,
            plugin: Mutex::new(plugin),
        })
    }

    pub fn execute(&self, input: &str) -> String {
        let mut plugin = self.plugin.lock().unwrap();
        plugin.call("process", input).unwrap()
    }
}
