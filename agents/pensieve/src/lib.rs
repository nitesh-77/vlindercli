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

mod config;
mod handlers;
mod host;
mod html;
mod intent;
mod persistence;
mod summarize;
mod text;
mod util;

use handlers::{
    handle_get_memory, handle_list_memories, handle_process_url, handle_search, handle_unknown,
};
use intent::{determine_intent, Intent};

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
        Intent::Unknown => handle_unknown(),
    }
}
