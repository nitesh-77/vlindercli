use vlindercli::domain::*;

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

#[test]
fn agent_output_has_response() {
    let output = AgentOutput {
        response: Some("Here are your notes.".to_string()),
        plan: None,
    };

    assert_eq!(output.response, Some("Here are your notes.".to_string()));
}

#[test]
fn operation_specifies_agent_and_input() {
    let op = Operation {
        agent: "note-taker".to_string(),
        input: "get all notes".to_string(),
    };

    assert_eq!(op.agent, "note-taker");
    assert_eq!(op.input, "get all notes");
}

#[test]
fn execution_plan_yields_operations() {
    let mut done = false;
    let mut plan: ExecutionPlan = Box::new(move |_results| {
        if done {
            return vec![];
        }
        done = true;
        vec![Operation {
            agent: "note-taker".to_string(),
            input: "get notes".to_string(),
        }]
    });

    let ops = plan(vec![]);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].agent, "note-taker");

    // After yielding, plan is done
    let ops = plan(vec![]);
    assert_eq!(ops.len(), 0);
}

#[test]
fn execution_plan_can_yield_parallel_operations() {
    let mut done = false;
    let mut plan: ExecutionPlan = Box::new(move |_results| {
        if done {
            return vec![];
        }
        done = true;
        vec![
            Operation {
                agent: "agent-a".to_string(),
                input: "task a".to_string(),
            },
            Operation {
                agent: "agent-b".to_string(),
                input: "task b".to_string(),
            },
        ]
    });

    let ops = plan(vec![]);
    assert_eq!(ops.len(), 2); // Both at once = parallel
}

#[test]
fn agent_output_can_include_execution_plan() {
    let plan: ExecutionPlan = Box::new(|_results| {
        vec![Operation {
            agent: "note-taker".to_string(),
            input: "get notes".to_string(),
        }]
    });

    let output = AgentOutput {
        response: Some("Let me check your notes...".to_string()),
        plan: Some(plan),
    };

    assert!(output.plan.is_some());
}
