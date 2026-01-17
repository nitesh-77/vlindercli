pub struct Model {
    pub path: String,
    pub model_type: String,
}

pub struct Behavior {
    pub system_prompt: String,
}

pub struct Agent {
    pub name: String,
    pub model: Model,
    pub behavior: Behavior,
}

pub struct Operation {
    pub agent: String,
    pub input: String,
}

pub type ExecutionPlan = Box<dyn FnMut(Vec<AgentOutput>) -> Vec<Operation>>;

pub struct AgentOutput {
    pub response: Option<String>,
    pub plan: Option<ExecutionPlan>,
}
