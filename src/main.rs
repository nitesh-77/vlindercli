struct Model {
    path: String,
    model_type: String,
}

struct Behavior {
    system_prompt: String,
}

struct Agent {
    name: String,
    model: Model,
    behavior: Behavior,
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
            behavior: Behavior {
                system_prompt: "You are helpful.".to_string(),
            },
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
            behavior: Behavior {
                system_prompt: "You are helpful.".to_string(),
            },
        };

        assert_eq!(agent.name, "test-agent");
    }

    #[test]
    fn agent_has_behavior() {
        let model = Model {
            path: "models/llama-2-7b.gguf".to_string(),
            model_type: "llm".to_string(),
        };

        let behavior = Behavior {
            system_prompt: "You are a note-taking assistant.".to_string(),
        };

        let agent = Agent {
            name: "note-taker".to_string(),
            model,
            behavior,
        };

        assert_eq!(agent.behavior.system_prompt, "You are a note-taking assistant.");
    }

    #[test]
    fn same_model_different_behavior_different_agent() {
        let model1 = Model {
            path: "models/llama-2-7b.gguf".to_string(),
            model_type: "llm".to_string(),
        };

        let model2 = Model {
            path: "models/llama-2-7b.gguf".to_string(),
            model_type: "llm".to_string(),
        };

        let note_taker = Agent {
            name: "note-taker".to_string(),
            model: model1,
            behavior: Behavior {
                system_prompt: "You are a note-taking assistant.".to_string(),
            },
        };

        let podcast_agent = Agent {
            name: "podcast-agent".to_string(),
            model: model2,
            behavior: Behavior {
                system_prompt: "You help manage podcast subscriptions.".to_string(),
            },
        };

        // Same model path
        assert_eq!(note_taker.model.path, podcast_agent.model.path);
        // Different behavior
        assert_ne!(note_taker.behavior.system_prompt, podcast_agent.behavior.system_prompt);
    }
}
