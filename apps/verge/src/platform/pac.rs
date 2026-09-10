//! Loopback-only PAC endpoint owned by the daemon; no application command routes.
use crate::domain::{AppError, ErrorCode};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub struct PacServer {
    port: u16,
    script: Arc<RwLock<String>>,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl PacServer {
    pub fn start(script: String) -> Result<Self, AppError> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(error)?;
        let port = listener.local_addr().map_err(error)?.port();
        let script = Arc::new(RwLock::new(script));
        let stopped = Arc::new(AtomicBool::new(false));
        let content = script.clone();
        let stop = stopped.clone();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(mut stream) = stream {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                    let mut request = [0u8; 4096];
                    let mut size = 0;
                    // A total deadline prevents a slow peer from holding the single worker.
                    let deadline = std::time::Instant::now() + Duration::from_millis(500);
                    while size < request.len() && std::time::Instant::now() < deadline {
                        let _ = stream.set_read_timeout(Some(
                            deadline
                                .saturating_duration_since(std::time::Instant::now())
                                .max(Duration::from_millis(1)),
                        ));
                        match stream.read(&mut request[size..]) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                size += n;
                                if request[..size].windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                        }
                    }
                    let valid = request[..size].starts_with(b"GET /proxy.pac HTTP/1.")
                        && request[..size].windows(4).any(|w| w == b"\r\n\r\n");
                    let body = if valid {
                        content.read().unwrap().clone()
                    } else {
                        "Not found".into()
                    };
                    let status = if valid { "200 OK" } else { "404 Not Found" };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    );
                }
            }
        });
        Ok(Self {
            port,
            script,
            stopped,
            thread: Some(thread),
        })
    }
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/proxy.pac", self.port)
    }
    pub fn set_script(&self, script: String) {
        *self.script.write().unwrap() = script;
    }
}
impl Drop for PacServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn error(error: std::io::Error) -> AppError {
    AppError::new(ErrorCode::PlatformFailed, format!("PAC server: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serves_latest_pac_and_rejects_other_routes() {
        let server = PacServer::start("first".into()).unwrap();
        let get = |path| {
            let mut client =
                TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, server.port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(client, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut text = String::new();
            client.read_to_string(&mut text).unwrap();
            text
        };
        assert!(get("/proxy.pac").ends_with("first"));
        server.set_script("second".into());
        let response = get("/proxy.pac");
        assert!(response.contains("application/x-ns-proxy-autoconfig"));
        assert!(response.ends_with("second"));
        assert!(get("/commands/quit").contains("404"));
    }
}
