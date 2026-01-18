use extism::{Plugin, Manifest, Wasm, Function};

#[derive(Clone, Debug, PartialEq)]
pub enum ModelType {
    Inference,
    Embedding,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub model_type: ModelType,
    pub name: String,
}

pub struct Behavior {
    pub system_prompt: String,
}

pub struct Agent {
    pub name: String,
    pub models: Vec<Model>,
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
        models: Vec<Model>,
        behavior: Behavior,
    ) -> Result<Agent, LoadError> {
        if !std::path::Path::new(wasm_path).exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "wasm not found",
            )));
        }

        // TODO: validate model paths exist

        Ok(Agent {
            name: name.to_string(),
            models,
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
