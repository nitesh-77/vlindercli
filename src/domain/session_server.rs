//! SessionServer — read-only HTTP server for browsing conversation sessions.
//!
//! Serves `~/.vlinder/conversations/` as a chat-style HTML viewer:
//!   GET /                     → index page listing all sessions
//!   GET /session/{file}.json  → rendered conversation view
//!
//! Starts by default alongside `vlinder agent run`. Binds to 127.0.0.1
//! (localhost only). Zero external dependencies — uses `std::net::TcpListener`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use super::session::{HistoryEntry, Session};

/// A running session viewer server.
///
/// Created by `start()`, runs in a background thread.
/// Shuts down when dropped or `stop()` is called.
pub struct SessionServer {
    port: u16,
    stop_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SessionServer {
    /// Start the session viewer in a background thread.
    ///
    /// Binds to `127.0.0.1:{port}` (localhost only — this is a local dev tool).
    pub fn start(conversations_dir: PathBuf, port: u16) -> std::io::Result<Self> {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stop_flag);

        let handle = std::thread::spawn(move || {
            run_server(listener, conversations_dir, stop);
        });

        Ok(Self {
            port,
            stop_flag,
            handle: Some(handle),
        })
    }

    /// The port the server is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Signal the server to stop and wait for it to finish.
    pub fn stop(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for SessionServer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

// =============================================================================
// Server loop
// =============================================================================

fn run_server(listener: TcpListener, dir: PathBuf, stop: Arc<AtomicBool>) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match listener.accept() {
            Ok((stream, _addr)) => {
                let _ = stream.set_nonblocking(false);
                handle_request(stream, &dir);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            Err(_) => break,
        }
    }
}

// =============================================================================
// Request handling
// =============================================================================

fn handle_request(mut stream: TcpStream, dir: &Path) {
    let path = match parse_request_path(&stream) {
        Ok(p) => p,
        Err(_) => {
            write_html_response(&mut stream, 400, "Bad Request");
            return;
        }
    };

    if path == "/" {
        let body = render_index(dir);
        write_html_response(&mut stream, 200, &body);
    } else if let Some(filename) = path.strip_prefix("/session/") {
        match render_session(dir, filename) {
            Ok(body) => write_html_response(&mut stream, 200, &body),
            Err(_) => write_html_response(&mut stream, 404, &html_page("Not Found", "<h1>Session not found</h1><p><a href=\"/\">&larr; Back</a></p>")),
        }
    } else {
        write_html_response(&mut stream, 404, &html_page("Not Found", "<h1>404</h1>"));
    }
}

/// Parse the request path from the first HTTP line.
fn parse_request_path(stream: &TcpStream) -> Result<String, String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)
        .map_err(|e| format!("read error: {}", e))?;

    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return Err("malformed request".to_string());
    }

    // Only accept GET
    if parts[0] != "GET" {
        return Err("method not allowed".to_string());
    }

    // Drain remaining headers
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)
            .map_err(|e| format!("header read error: {}", e))?;
        if line.trim().is_empty() {
            break;
        }
    }

    Ok(parts[1].to_string())
}

// =============================================================================
// Rendering
// =============================================================================

fn render_index(dir: &Path) -> String {
    let mut sessions: Vec<(String, Option<Session>)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }
            let session = std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|json| serde_json::from_str::<Session>(&json).ok());
            sessions.push((name, session));
        }
    }

    // Sort newest first (filenames start with datetime)
    sessions.sort_by(|a, b| b.0.cmp(&a.0));

    if sessions.is_empty() {
        return html_page("Vlinder Sessions", "<h1>Sessions</h1><p>No conversations yet.</p>");
    }

    let mut items = String::new();
    for (filename, session) in &sessions {
        let (agent, turns, status) = match session {
            Some(s) => {
                let turns = s.history.len();
                let status = if s.open.is_some() { "<span class=\"badge\">pending</span>" } else { "" };
                (s.agent.as_str().to_string(), turns, status.to_string())
            }
            None => ("?".to_string(), 0, String::new()),
        };

        // Parse datetime from filename: 2026-02-08T14-30-05Z_agent_id.json
        let datetime = filename.split('_').next().unwrap_or("")
            .replace('T', " ")
            .trim_end_matches('Z')
            .to_string();

        items.push_str(&format!(
            "<li><a href=\"/session/{filename}\">\
             <strong>{agent}</strong>\
             <span class=\"meta\">{datetime} &middot; {turns} messages {status}</span>\
             </a></li>\n",
            filename = html_escape(filename),
            agent = html_escape(&agent),
            datetime = html_escape(&datetime),
            turns = turns,
            status = status,
        ));
    }

    html_page("Vlinder Sessions", &format!(
        "<h1>Sessions</h1>\n<ul class=\"session-list\">\n{}</ul>", items
    ))
}

fn render_session(dir: &Path, filename: &str) -> Result<String, String> {
    // Security: reject path traversal
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Err("invalid filename".to_string());
    }
    if !filename.ends_with(".json") {
        return Err("invalid filename".to_string());
    }

    let filepath = dir.join(filename);
    let json = std::fs::read_to_string(&filepath)
        .map_err(|_| "session not found".to_string())?;
    let session: Session = serde_json::from_str(&json)
        .map_err(|e| format!("invalid session: {}", e))?;

    let mut messages = String::new();

    // Show pending indicator
    if let Some(ref question) = session.open {
        messages.push_str(&format!(
            "<div class=\"open-indicator\">Pending: {}</div>\n",
            html_escape(question)
        ));
    }

    for entry in &session.history {
        match entry {
            HistoryEntry::User { user, at, .. } => {
                messages.push_str(&format!(
                    "<div class=\"msg user\">\
                     <div class=\"role\">User <span class=\"ts\">{}</span></div>\
                     <pre>{}</pre>\
                     </div>\n",
                    html_escape(at),
                    html_escape(user),
                ));
            }
            HistoryEntry::Agent { agent, at } => {
                messages.push_str(&format!(
                    "<div class=\"msg agent\">\
                     <div class=\"role\">Agent <span class=\"ts\">{}</span></div>\
                     <pre>{}</pre>\
                     </div>\n",
                    html_escape(at),
                    html_escape(agent),
                ));
            }
        }
    }

    let title = format!("{} / {}", session.agent, &session.session.as_str()[4..12.min(session.session.as_str().len())]);

    Ok(html_page(&title, &format!(
        "<p><a href=\"/\">&larr; All sessions</a></p>\n\
         <h1>{}</h1>\n\
         {}",
        html_escape(&title),
        messages,
    )))
}

// =============================================================================
// HTML helpers
// =============================================================================

const CSS: &str = r#"
* { box-sizing: border-box; margin: 0; padding: 0; }
body {
    font-family: system-ui, -apple-system, sans-serif;
    max-width: 800px; margin: 2em auto; padding: 0 1em;
    background: #1a1a1a; color: #e0e0e0;
    line-height: 1.5;
}
h1 { margin-bottom: 0.5em; color: #fff; }
a { color: #6cb4ee; text-decoration: none; }
a:hover { text-decoration: underline; }
.session-list { list-style: none; }
.session-list li { margin: 0.5em 0; }
.session-list a {
    display: block; padding: 0.8em 1em;
    background: #252525; border-radius: 6px;
    transition: background 0.15s;
}
.session-list a:hover { background: #303030; text-decoration: none; }
.session-list strong { display: block; color: #fff; }
.meta { color: #888; font-size: 0.85em; }
.badge {
    display: inline-block; background: #e6a817; color: #000;
    padding: 1px 6px; border-radius: 3px; font-size: 0.75em;
    font-weight: bold; vertical-align: middle;
}
.msg { border-radius: 8px; padding: 0.8em 1em; margin: 0.6em 0; }
.msg.user { background: #1a3a5c; }
.msg.agent { background: #252525; }
.role { font-weight: bold; font-size: 0.85em; color: #aaa; margin-bottom: 0.3em; }
.ts { font-weight: normal; color: #666; font-size: 0.85em; }
pre {
    white-space: pre-wrap; word-wrap: break-word;
    font-family: inherit; font-size: 0.95em; color: #e0e0e0;
}
.open-indicator {
    background: #3d2e00; border: 1px solid #e6a817;
    border-radius: 6px; padding: 0.6em 1em; margin-bottom: 1em;
    color: #e6a817; font-size: 0.9em;
}
"#;

fn html_page(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n\
         <html>\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n\
         <style>{css}</style>\n\
         </head>\n\
         <body>\n\
         {body}\n\
         </body>\n\
         </html>",
        title = html_escape(title),
        css = CSS,
        body = body,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

fn write_html_response(stream: &mut TcpStream, status: u16, body: &str) {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };

    let body_bytes = body.as_bytes();
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status, status_text, body_bytes.len()
    );

    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body_bytes);
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{SessionId, SubmissionId};
    use std::io::Read;

    fn test_session() -> Session {
        let mut session = Session::new(
            SessionId::from("ses-abc12345".to_string()),
            "pensieve",
        );
        session.record_user_input("summarize this article", SubmissionId::from("a1b2c3d".to_string()));
        session.record_agent_response("This article discusses several topics.");
        session
    }

    #[test]
    fn server_starts_and_stops() {
        let tmp = tempfile::TempDir::new().unwrap();
        let server = SessionServer::start(tmp.path().to_path_buf(), 0).unwrap();
        assert!(server.port() > 0);
        server.stop();
    }

    #[test]
    fn index_returns_html() {
        let tmp = tempfile::TempDir::new().unwrap();
        let server = SessionServer::start(tmp.path().to_path_buf(), 0).unwrap();
        let port = server.port();

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
        assert!(response.contains("text/html"));
        assert!(response.contains("Vlinder Sessions"));

        server.stop();
    }

    #[test]
    fn index_lists_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session = test_session();
        let json = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(tmp.path().join("2026-02-08T14-30-05Z_pensieve_abc12345.json"), &json).unwrap();

        let server = SessionServer::start(tmp.path().to_path_buf(), 0).unwrap();
        let port = server.port();

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("pensieve"));
        assert!(response.contains("2 messages"));

        server.stop();
    }

    #[test]
    fn session_page_renders_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session = test_session();
        let filename = "2026-02-08T14-30-05Z_pensieve_abc12345.json";
        let json = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(tmp.path().join(filename), &json).unwrap();

        let server = SessionServer::start(tmp.path().to_path_buf(), 0).unwrap();
        let port = server.port();

        let request = format!("GET /session/{} HTTP/1.1\r\nHost: localhost\r\n\r\n", filename);
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
        assert!(response.contains("summarize this article"));
        assert!(response.contains("This article discusses"));

        server.stop();
    }

    #[test]
    fn nonexistent_session_returns_404() {
        let tmp = tempfile::TempDir::new().unwrap();
        let server = SessionServer::start(tmp.path().to_path_buf(), 0).unwrap();
        let port = server.port();

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(b"GET /session/nosuch.json HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("404"));

        server.stop();
    }

    #[test]
    fn path_traversal_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let server = SessionServer::start(tmp.path().to_path_buf(), 0).unwrap();
        let port = server.port();

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(b"GET /session/../../etc/passwd HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("404"));

        server.stop();
    }

    #[test]
    fn html_escape_prevents_xss() {
        let escaped = html_escape("<script>alert('xss')</script>");
        assert!(!escaped.contains("<script>"));
        assert!(escaped.contains("&lt;script&gt;"));
    }

    #[test]
    fn render_index_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let html = render_index(tmp.path());
        assert!(html.contains("No conversations yet"));
    }

    #[test]
    fn render_session_with_open_question() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut session = Session::new(
            SessionId::from("ses-def67890".to_string()),
            "todoapp",
        );
        session.record_user_input("what next?", SubmissionId::from("b2c3d4e".to_string()));
        // Don't record response — question is pending

        let filename = "test_session.json";
        let json = serde_json::to_string_pretty(&session).unwrap();
        std::fs::write(tmp.path().join(filename), &json).unwrap();

        let html = render_session(tmp.path(), filename).unwrap();
        assert!(html.contains("Pending"));
        assert!(html.contains("what next?"));
    }
}
