//! Pensieve Container Agent — HTTP server version
//!
//! Like Dumbledore's pensieve: store memories (articles) for later review.
//! Container version that uses the HTTP bridge for service calls.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

mod bridge;
mod config;
mod handlers;
mod html;
mod intent;
mod persistence;
mod summarize;
mod text;
mod util;

use handlers::{
    handle_get_memory, handle_list_memories, handle_process_url, handle_question, handle_search,
    handle_unknown,
};
use intent::{determine_intent, Intent};

fn process(input: &str) -> String {
    let intent = determine_intent(input);

    let result = match intent {
        Intent::ProcessUrl { url } => handle_process_url(&url),
        Intent::ListMemories => handle_list_memories(),
        Intent::GetMemory { url } => handle_get_memory(&url),
        Intent::Search { query } => handle_search(&query),
        Intent::Question { query } => handle_question(&query),
        Intent::Unknown => handle_unknown(),
    };

    match result {
        Ok(s) => s,
        Err(e) => format!("Error: {}", e),
    }
}

fn main() {
    let listener = TcpListener::bind("0.0.0.0:8080").expect("Failed to bind port 8080");

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        handle_connection(stream);
    }
}

fn handle_connection(mut stream: std::net::TcpStream) {
    let mut reader = BufReader::new(&stream);

    // Parse request line
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }

    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }
    let method = parts[0];

    // Read headers
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }

    if method == "GET" {
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
        let _ = stream.write_all(response.as_bytes());
        return;
    }

    if method != "POST" {
        let body = b"Method not allowed";
        let header = format!(
            "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body);
        return;
    }

    // Read body
    let mut body = vec![0u8; content_length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }

    let input = String::from_utf8_lossy(&body);
    let result = process(&input);

    let response_body = result.as_bytes();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(response_body);
    let _ = stream.flush();
}
