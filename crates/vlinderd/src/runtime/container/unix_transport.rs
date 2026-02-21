//! Unix socket transport for ureq v3.
//!
//! Implements the ureq `Connector` + `Transport` traits over
//! `std::os::unix::net::UnixStream`, allowing ureq to speak HTTP/1.1
//! to the Podman REST API socket.

use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport,
};

// ── Connector ────────────────────────────────────────────────────────

/// Connector that opens a `UnixStream` to a fixed socket path.
///
/// Ignores DNS-resolved addresses — the path is known at construction time.
pub(crate) struct UnixConnector {
    path: PathBuf,
}

impl UnixConnector {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl fmt::Debug for UnixConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnixConnector")
            .field("path", &self.path)
            .finish()
    }
}

impl Connector for UnixConnector {
    type Out = UnixTransport;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let stream = UnixStream::connect(&self.path)
            .map_err(ureq::Error::Io)?;

        let buffers = LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        );

        Ok(Some(UnixTransport {
            stream,
            buffers,
            timeout_write: None,
            timeout_read: None,
        }))
    }
}

// ── Transport ────────────────────────────────────────────────────────

/// HTTP/1.1 transport over a Unix domain socket.
pub(crate) struct UnixTransport {
    stream: UnixStream,
    buffers: LazyBuffers,
    timeout_write: Option<std::time::Duration>,
    timeout_read: Option<std::time::Duration>,
}

impl fmt::Debug for UnixTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnixTransport").finish()
    }
}

impl Transport for UnixTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let new_timeout = timeout.not_zero().map(|d| *d);
        if new_timeout != self.timeout_write {
            self.stream.set_write_timeout(new_timeout)?;
            self.timeout_write = new_timeout;
        }

        let output = &self.buffers.output()[..amount];
        match self.stream.write_all(output) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock
                || e.kind() == io::ErrorKind::TimedOut =>
            {
                Err(ureq::Error::Timeout(timeout.reason))
            }
            Err(e) => Err(ureq::Error::Io(e)),
        }
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let new_timeout = timeout.not_zero().map(|d| *d);
        if new_timeout != self.timeout_read {
            self.stream.set_read_timeout(new_timeout)?;
            self.timeout_read = new_timeout;
        }

        let input = self.buffers.input_append_buf();
        let amount = match self.stream.read(input) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock
                || e.kind() == io::ErrorKind::TimedOut =>
            {
                return Err(ureq::Error::Timeout(timeout.reason));
            }
            Err(e) => return Err(ureq::Error::Io(e)),
        };
        self.buffers.input_appended(amount);

        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        // Probe with a non-blocking read — if the socket has closed,
        // we'll get an error or 0 bytes.
        self.stream.set_nonblocking(true).ok();
        let mut buf = [0u8; 1];
        let open = match self.stream.read(&mut buf) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => true,
            Ok(0) => false,
            _ => false,
        };
        self.stream.set_nonblocking(false).ok();
        open
    }
}

// ── Agent factory ────────────────────────────────────────────────────

/// Build a ureq `Agent` that routes all requests through the given Unix socket.
///
/// The agent uses `http://localhost` as the base URL (host doesn't matter —
/// all connections go to the socket path).
pub(crate) fn unix_agent(socket_path: &Path) -> ureq::Agent {
    let connector = UnixConnector::new(socket_path);
    let resolver = ureq::unversioned::resolver::DefaultResolver::default();
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build();
    ureq::Agent::with_parts(config, connector, resolver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;

    #[test]
    fn round_trip_through_unix_socket() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sock_path = tmp.path().join("test.sock");

        let listener = UnixListener::bind(&sock_path).unwrap();

        // Spawn a minimal HTTP server on the socket
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&stream);

            // Read the HTTP request (headers end with blank line)
            let mut request = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }

            // Send a minimal HTTP response
            let body = r#"{"version":"5.0.0"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                body.len(),
                body,
            );
            (&stream).write_all(response.as_bytes()).unwrap();

            request
        });

        // Use the agent to make a request
        let agent = unix_agent(&sock_path);
        let mut resp = agent.get("http://localhost/version").call().unwrap();
        assert_eq!(resp.status().as_u16(), 200);

        let body = resp.body_mut().read_to_string().unwrap();
        assert!(body.contains("5.0.0"));

        // Verify the server saw a valid HTTP request
        let request = handle.join().unwrap();
        assert!(request.contains("GET /version"));
    }
}
