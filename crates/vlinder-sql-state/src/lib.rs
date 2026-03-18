//! vlinder-sql-state — SQLite-backed state vertical.
//!
//! Contains the SqliteDagStore (Merkle DAG persistence) and the
//! SessionServer (read-only HTTP viewer for conversation sessions).

#[cfg(feature = "server")]
pub mod dag_store;
pub mod state_service;
#[cfg(feature = "server")]
pub use dag_store::SqliteDagStore;

// =============================================================================
// SessionServer — server-only (tiny_http + DagStore)
// =============================================================================

#[cfg(feature = "server")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "server")]
use std::sync::Arc;
#[cfg(feature = "server")]
use std::thread::JoinHandle;

#[cfg(feature = "server")]
use std::fmt::Write as _;
#[cfg(feature = "server")]
use vlinder_core::domain::{DagStore, MessageType, SessionId};

/// A running session viewer server.
///
/// Created by `start()`, runs in a background thread.
/// Shuts down when dropped or `stop()` is called.
#[cfg(feature = "server")]
pub struct SessionServer {
    port: u16,
    stop_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

#[cfg(feature = "server")]
impl SessionServer {
    /// Start the session viewer in a background thread.
    ///
    /// Binds to `127.0.0.1:{port}` (localhost only — this is a local dev tool).
    pub fn start(store: Arc<dyn DagStore>, port: u16) -> std::io::Result<Self> {
        let server = tiny_http::Server::http(format!("127.0.0.1:{port}"))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::AddrInUse, e.to_string()))?;

        let port = server.server_addr().to_ip().map_or(port, |a| a.port());

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stop_flag);

        let handle = std::thread::spawn(move || {
            run_server(&server, &*store, &stop);
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

#[cfg(feature = "server")]
impl Drop for SessionServer {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

// =============================================================================
// Server loop
// =============================================================================

#[cfg(feature = "server")]
fn run_server(server: &tiny_http::Server, store: &dyn DagStore, stop: &AtomicBool) {
    let timeout = std::time::Duration::from_millis(100);
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match server.recv_timeout(timeout) {
            Ok(Some(request)) => handle_request(request, store),
            Ok(None) => {} // timeout — check stop flag
            Err(_) => break,
        }
    }
}

#[cfg(feature = "server")]
fn handle_request(request: tiny_http::Request, store: &dyn DagStore) {
    let url = request.url().to_string();

    if url == "/" {
        let body = render_index(store);
        let _ = request.respond(html_response(200, &body));
    } else if let Some(raw_id) = url.strip_prefix("/session/") {
        if raw_id.contains("..") || raw_id.contains('/') || raw_id.contains('\\') {
            let body = html_page("Not Found", "<h1>Invalid session id</h1>");
            let _ = request.respond(html_response(404, &body));
            return;
        }
        let session = SessionId::try_from(raw_id.to_string())
            .ok()
            .and_then(|sid| store.get_session(&sid).ok().flatten());
        if let Some(session) = session {
            if let Ok(body) = render_session(store, &session) {
                let _ = request.respond(html_response(200, &body));
            } else {
                let body = html_page(
                    "Not Found",
                    "<h1>Session not found</h1><p><a href=\"/\">&larr; Back</a></p>",
                );
                let _ = request.respond(html_response(404, &body));
            }
        } else {
            let body = html_page(
                "Not Found",
                "<h1>Session not found</h1><p><a href=\"/\">&larr; Back</a></p>",
            );
            let _ = request.respond(html_response(404, &body));
        }
    } else {
        let body = html_page("Not Found", "<h1>404</h1>");
        let _ = request.respond(html_response(404, &body));
    }
}

// =============================================================================
// Rendering
// =============================================================================

#[cfg(feature = "server")]
fn render_index(store: &dyn DagStore) -> String {
    let Ok(sessions) = store.list_sessions() else {
        return html_page(
            "Vlinder Sessions",
            "<h1>Sessions</h1><p>Error loading sessions.</p>",
        );
    };

    if sessions.is_empty() {
        return html_page(
            "Vlinder Sessions",
            "<h1>Sessions</h1><p>No conversations yet.</p>",
        );
    }

    let mut items = String::new();
    for s in &sessions {
        let datetime = s.started_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let status = if s.is_open {
            "<span class=\"badge\">pending</span>"
        } else {
            ""
        };

        let _ = writeln!(
            items,
            "<li><a href=\"/session/{session_id}\">\
             <strong>{agent}</strong>\
             <span class=\"meta\">{datetime} &middot; {turns} messages {status}</span>\
             </a></li>",
            session_id = html_escape(s.session_id.as_str()),
            agent = html_escape(&s.agent_name),
            datetime = html_escape(&datetime),
            turns = s.message_count,
            status = status,
        );
    }

    html_page(
        "Vlinder Sessions",
        &format!("<h1>Sessions</h1>\n<ul class=\"session-list\">\n{items}</ul>",),
    )
}

#[cfg(feature = "server")]
fn render_session(
    store: &dyn DagStore,
    session: &vlinder_core::domain::Session,
) -> Result<String, String> {
    let nodes = store.get_session_nodes(&session.id)?;
    if nodes.is_empty() {
        return Err("session not found".to_string());
    }

    let agent_name = &session.agent;

    let is_open = nodes
        .last()
        .is_some_and(|n| n.message_type() != MessageType::Complete);

    let mut messages = String::new();

    // Show pending indicator if the last message is not a Complete
    if is_open {
        // Show the last invoke payload as the pending question
        if let Some(last_invoke) = nodes
            .iter()
            .rev()
            .find(|n| n.message_type() == MessageType::Invoke)
        {
            let payload = String::from_utf8_lossy(last_invoke.payload());
            let _ = writeln!(
                messages,
                "<div class=\"open-indicator\">Pending: {}</div>",
                html_escape(&payload)
            );
        }
    }

    for node in &nodes {
        match node.message_type() {
            MessageType::Invoke => {
                let payload = String::from_utf8_lossy(node.payload());
                let ts = node.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
                let _ = writeln!(
                    messages,
                    "<div class=\"msg user\">\
                     <div class=\"role\">User <span class=\"ts\">{}</span></div>\
                     <pre>{}</pre>\
                     </div>",
                    html_escape(&ts),
                    html_escape(&payload),
                );
            }
            MessageType::Complete => {
                let payload = String::from_utf8_lossy(node.payload());
                let ts = node.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
                let _ = writeln!(
                    messages,
                    "<div class=\"msg agent\">\
                     <div class=\"role\">Agent <span class=\"ts\">{}</span></div>\
                     <pre>{}</pre>\
                     </div>",
                    html_escape(&ts),
                    html_escape(&payload),
                );
            }
            _ => {} // Skip Request/Response/Delegate — internal protocol messages
        }
    }

    let title = format!("{} / {}", agent_name, session.name);

    Ok(html_page(
        &title,
        &format!(
            "<p><a href=\"/\">&larr; All sessions</a></p>\n\
         <h1>{}</h1>\n\
         {}",
            html_escape(&title),
            messages,
        ),
    ))
}

// =============================================================================
// HTML helpers
// =============================================================================

#[cfg(feature = "server")]
const CSS: &str = r"
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
";

#[cfg(feature = "server")]
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

#[cfg(feature = "server")]
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(feature = "server")]
fn html_response(status: u16, body: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let status_code = tiny_http::StatusCode(status);
    let content_type = tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8")
        .expect("valid header");
    let connection = tiny_http::Header::from_bytes("Connection", "close").expect("valid header");
    tiny_http::Response::from_data(body.as_bytes().to_vec())
        .with_status_code(status_code)
        .with_header(content_type)
        .with_header(connection)
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use vlinder_core::domain::session::Session;
    use vlinder_core::domain::workers::dag::build_dag_node;
    use vlinder_core::domain::{
        AgentId, BranchId, CompleteMessage, DagNodeId, HarnessType, InMemoryDagStore,
        InvokeDiagnostics, InvokeMessage, ObservableMessage, RuntimeDiagnostics, RuntimeType,
        SubmissionId,
    };

    fn sess_id() -> SessionId {
        SessionId::try_from("d4761d76-dee4-4ebf-9df4-43b52efa4f78".to_string()).unwrap()
    }

    fn make_invoke_msg(payload: &[u8], agent: &str, session: SessionId) -> ObservableMessage {
        InvokeMessage::new(
            BranchId::from(1),
            SubmissionId::from("sub-1".to_string()),
            session,
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new(agent),
            payload.to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            DagNodeId::root(),
        )
        .into()
    }

    fn make_complete_msg(payload: &[u8], agent: &str, session: SessionId) -> ObservableMessage {
        CompleteMessage::new(
            BranchId::from(1),
            SubmissionId::from("sub-1".to_string()),
            session,
            AgentId::new(agent),
            HarnessType::Cli,
            payload.to_vec(),
            None,
            RuntimeDiagnostics::placeholder(0),
        )
        .into()
    }

    fn test_store_with_session() -> Arc<InMemoryDagStore> {
        let store = Arc::new(InMemoryDagStore::new());
        let sid = sess_id();

        let invoke_msg = make_invoke_msg(b"summarize this article", "pensieve", sid.clone());
        let mut invoke = build_dag_node(&invoke_msg, &DagNodeId::root());
        invoke.created_at = Utc.with_ymd_and_hms(2026, 2, 8, 14, 30, 5).unwrap();

        let complete_msg =
            make_complete_msg(b"This article discusses several topics.", "pensieve", sid);
        let mut complete = build_dag_node(&complete_msg, &invoke.id);
        complete.created_at = Utc.with_ymd_and_hms(2026, 2, 8, 14, 30, 10).unwrap();

        store.insert_node(&invoke).unwrap();
        store.insert_node(&complete).unwrap();

        // Create a session so the viewer can look it up
        let session = Session::new(sess_id(), "pensieve");
        store.create_session(&session).unwrap();

        store
    }

    fn get_body(port: u16, path: &str) -> (u16, String) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let mut resp = agent
            .get(&format!("http://127.0.0.1:{port}{path}"))
            .call()
            .unwrap();
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().unwrap();
        (status, body)
    }

    #[test]
    fn server_starts_and_stops() {
        let store = Arc::new(InMemoryDagStore::new());
        let server = SessionServer::start(store, 0).unwrap();
        assert!(server.port() > 0);
        server.stop();
    }

    #[test]
    fn index_returns_html() {
        let store = Arc::new(InMemoryDagStore::new());
        let server = SessionServer::start(store, 0).unwrap();
        let port = server.port();

        let (status, body) = get_body(port, "/");
        assert_eq!(status, 200);
        assert!(body.contains("Vlinder Sessions"));

        server.stop();
    }

    #[test]
    fn index_lists_sessions() {
        let store = test_store_with_session();
        let server = SessionServer::start(store, 0).unwrap();
        let port = server.port();

        let (_, body) = get_body(port, "/");
        assert!(body.contains("pensieve"));
        assert!(body.contains("2 messages"));

        server.stop();
    }

    #[test]
    fn session_page_renders_history() {
        let store = test_store_with_session();
        let server = SessionServer::start(store, 0).unwrap();
        let port = server.port();

        let (status, body) = get_body(port, "/session/d4761d76-dee4-4ebf-9df4-43b52efa4f78");
        assert_eq!(status, 200);
        assert!(body.contains("summarize this article"));
        assert!(body.contains("This article discusses"));

        server.stop();
    }

    #[test]
    fn nonexistent_session_returns_404() {
        let store = Arc::new(InMemoryDagStore::new());
        let server = SessionServer::start(store, 0).unwrap();
        let port = server.port();

        let (status, _) = get_body(port, "/session/ses-nonexistent");
        assert_eq!(status, 404);

        server.stop();
    }

    #[test]
    fn path_traversal_rejected() {
        let store = Arc::new(InMemoryDagStore::new());
        let server = SessionServer::start(store, 0).unwrap();
        let port = server.port();

        let (status, _) = get_body(port, "/session/../../etc/passwd");
        assert_eq!(status, 404);

        server.stop();
    }

    #[test]
    fn html_escape_prevents_xss() {
        let escaped = html_escape("<script>alert('xss')</script>");
        assert!(!escaped.contains("<script>"));
        assert!(escaped.contains("&lt;script&gt;"));
    }

    #[test]
    fn render_index_empty_store() {
        let store = InMemoryDagStore::new();
        let html = render_index(&store);
        assert!(html.contains("No conversations yet"));
    }

    #[test]
    fn render_session_with_open_question() {
        let store = InMemoryDagStore::new();
        let sid = SessionId::try_from("e2660cff-33d6-4428-acca-2d297dcc1cad".to_string()).unwrap();
        let msg = make_invoke_msg(b"what next?", "todoapp", sid.clone());
        let mut invoke = build_dag_node(&msg, &DagNodeId::root());
        invoke.created_at = Utc.with_ymd_and_hms(2026, 2, 8, 14, 30, 5).unwrap();
        store.insert_node(&invoke).unwrap();

        let session = Session::new(sid, "todoapp");
        store.create_session(&session).unwrap();

        let html = render_session(&store, &session).unwrap();
        assert!(html.contains("Pending"));
        assert!(html.contains("what next?"));
    }
}
