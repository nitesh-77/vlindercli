use extism::{Plugin, Manifest, Wasm, Function};

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
    wasm_path: String,
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
        // TODO: validate wasm with runtime-provided functions
        // For now, just check file exists
        if !std::path::Path::new(wasm_path).exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "wasm not found",
            )));
        }

        Ok(Agent {
            name: name.to_string(),
            model,
            behavior,
            wasm_path: wasm_path.to_string(),
        })
    }

    pub fn execute(&self, input: &str) -> String {
        self.execute_with_functions(input, [])
    }

    pub fn execute_with_functions(
        &self,
        input: &str,
        functions: impl IntoIterator<Item = Function>,
    ) -> String {
        let wasm = Wasm::file(&self.wasm_path);
        let manifest = Manifest::new([wasm]).with_allowed_host("*");
        let mut plugin = Plugin::new(&manifest, functions, true).unwrap();
        plugin.call("process", input).unwrap()
    }
}
