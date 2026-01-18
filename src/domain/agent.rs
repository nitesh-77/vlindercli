use extism::{Plugin, Manifest, Wasm, Function};

#[derive(Clone, Debug)]
pub struct Model {
    pub name: String,
}

#[derive(Clone)]
pub struct Agent {
    pub name: String,
    pub models: Vec<Model>,
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
    pub fn load(name: &str, models: Vec<Model>) -> Result<Agent, LoadError> {
        // Convention: agent_wasm/{name_with_underscores}.wasm
        let wasm_name = name.replace('-', "_");
        let wasm_path = format!("agent_wasm/{}.wasm", wasm_name);

        if !std::path::Path::new(&wasm_path).exists() {
            return Err(LoadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("wasm not found: {}", wasm_path),
            )));
        }

        Ok(Agent {
            name: name.to_string(),
            models,
            wasm_path,
        })
    }

    pub fn has_model(&self, name: &str) -> bool {
        self.models.iter().any(|m| m.name == name)
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
