//! Pensieve Agent - Extract and recall web content
//!
//! Like Dumbledore's pensieve: store memories (articles) for later review.
//! An intent-driven agent that understands what you want to do with your memories.
//!
//! ## Supported Intents
//!
//! - **PROCESS_URL**: Commit a new web page to memory
//! - **LIST_MEMORIES**: Review the index of all stored memories
//! - **GET_MEMORY**: Recall a specific memory in its entirety
//! - **SEARCH**: Probe all memories for connections on a topic
//! - **UNKNOWN**: Request clarification when intent is unclear

use extism_pdk::*;
use minijinja::{context, Environment};

mod config;
mod handlers;
mod host;
mod html;
mod intent;
mod persistence;
mod summarize;
mod text;
mod util;

use config::{
    DEFAULT_ANSWER_GENERATION, DEFAULT_DIRECT_SUMMARIZE, DEFAULT_INTENT_RECOGNITION,
    DEFAULT_MAP_SUMMARIZE, DEFAULT_QUERY_EXPANSION, DEFAULT_REDUCE_SUMMARIES,
};
use handlers::{
    handle_get_memory, handle_list_memories, handle_process_url, handle_question, handle_search,
    handle_unknown,
};
use intent::{determine_intent, Intent};

/// Template for generating default configuration
const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("templates/default_config.toml.j2");

/// Process user input by determining intent and dispatching to the appropriate handler
#[plugin_fn]
pub fn process(input: String) -> FnResult<String> {
    // Step 1: Determine what the user wants to do
    let intent = determine_intent(&input)?;

    // Step 2: Dispatch to the appropriate handler
    match intent {
        Intent::ProcessUrl { url } => handle_process_url(&url),
        Intent::ListMemories => handle_list_memories(),
        Intent::GetMemory { url } => handle_get_memory(&url),
        Intent::Search { query } => handle_search(&query),
        Intent::Question { query } => handle_question(&query),
        Intent::Unknown => handle_unknown(),
    }
}

/// Return the default Vlinderfile configuration with all compiled-in prompts.
///
/// Safety net for users - if they mess up their Vlinderfile, this returns
/// a working configuration with all defaults exposed.
#[plugin_fn]
pub fn get_default_config(_input: String) -> FnResult<String> {
    let mut env = Environment::new();
    env.add_template("config", DEFAULT_CONFIG_TEMPLATE)
        .map_err(|e| extism_pdk::Error::msg(format!("Template error: {}", e)))?;

    let tmpl = env
        .get_template("config")
        .map_err(|e| extism_pdk::Error::msg(format!("Template not found: {}", e)))?;

    let result = tmpl
        .render(context! {
            intent_recognition => DEFAULT_INTENT_RECOGNITION,
            query_expansion => DEFAULT_QUERY_EXPANSION,
            answer_generation => DEFAULT_ANSWER_GENERATION,
            map_summarize => DEFAULT_MAP_SUMMARIZE,
            reduce_summaries => DEFAULT_REDUCE_SUMMARIES,
            direct_summarize => DEFAULT_DIRECT_SUMMARIZE,
        })
        .map_err(|e| extism_pdk::Error::msg(format!("Render error: {}", e)))?;

    Ok(result)
}
