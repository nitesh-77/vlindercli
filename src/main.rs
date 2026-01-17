struct Model {
    path: String,
    model_type: String,
}

struct Agent {
    name: String,
    model: Model,
}

fn main() {
    println!("Hello, world!");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_has_model_dependency() {
        let model = Model {
            path: "models/llama-2-7b.gguf".to_string(),
            model_type: "llm".to_string(),
        };

        let agent = Agent {
            name: "hello-agent".to_string(),
            model,
        };

        assert_eq!(agent.name, "hello-agent");
        assert_eq!(agent.model.path, "models/llama-2-7b.gguf");
        assert_eq!(agent.model.model_type, "llm");
    }

    #[test]
    fn model_is_separate_from_agent() {
        let model = Model {
            path: "models/llama-2-7b.gguf".to_string(),
            model_type: "llm".to_string(),
        };

        // Model exists independently
        assert_eq!(model.path, "models/llama-2-7b.gguf");

        // Agent uses model as dependency
        let agent = Agent {
            name: "test-agent".to_string(),
            model,
        };

        assert_eq!(agent.name, "test-agent");
    }
}
