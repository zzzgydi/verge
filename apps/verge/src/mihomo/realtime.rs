use std::{
    collections::VecDeque,
    io,
    net::{SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Deserialize;
use tungstenite::{
    Error as WebSocketError, Message, client,
    client::IntoClientRequest,
    http::{HeaderValue, header::AUTHORIZATION},
};
use crate::domain::{
    AppError, Connection, ConnectionSnapshot, ErrorCode, RealtimeEvent, RealtimeTopic,
};

fn topic_path(topic: RealtimeTopic) -> &'static str {
    match topic {
        RealtimeTopic::Traffic => "/traffic",
        RealtimeTopic::Memory => "/memory",
        RealtimeTopic::Connections => "/connections?interval=1000",
        RealtimeTopic::Logs => "/logs?level=debug",
    }
}

#[derive(Clone, Debug)]
pub struct RealtimeOptions {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub buffer_capacity: usize,
}

impl Default for RealtimeOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_millis(200),
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            buffer_capacity: 128,
        }
    }
}

impl RealtimeOptions {
    fn validate(&self) -> Result<(), AppError> {
        if self.buffer_capacity == 0
            || self.initial_backoff.is_zero()
            || self.max_backoff < self.initial_backoff
            || self.read_timeout.is_zero()
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "invalid Mihomo realtime options",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct EventBuffer {
    capacity: usize,
    events: Arc<Mutex<VecDeque<RealtimeEvent>>>,
}

impl EventBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            events: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
        }
    }

    fn push(&self, event: RealtimeEvent) {
        let mut events = self.events.lock().expect("realtime event mutex poisoned");
        if events.len() == self.capacity {
            events.pop_front();
        }
        events.push_back(event);
    }

    fn drain(&self) -> Vec<RealtimeEvent> {
        self.events
            .lock()
            .expect("realtime event mutex poisoned")
            .drain(..)
            .collect()
    }
}

pub struct RealtimeSubscription {
    cancel: Arc<AtomicBool>,
    events: EventBuffer,
    worker: Option<JoinHandle<()>>,
}

impl RealtimeSubscription {
    pub fn spawn(
        controller: SocketAddr,
        secret: impl Into<String>,
        topic: RealtimeTopic,
        options: RealtimeOptions,
    ) -> Result<Self, AppError> {
        options.validate()?;
        if !controller.ip().is_loopback() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Mihomo realtime controller must use a loopback address",
            ));
        }
        let secret = secret.into();
        if secret.chars().any(char::is_control) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Mihomo controller secret contains control characters",
            ));
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let events = EventBuffer::new(options.buffer_capacity);
        let worker_cancel = Arc::clone(&cancel);
        let worker_events = events.clone();
        let worker = thread::spawn(move || {
            run_subscription(
                controller,
                &secret,
                topic,
                &options,
                &worker_cancel,
                &worker_events,
            );
        });
        Ok(Self {
            cancel,
            events,
            worker: Some(worker),
        })
    }

    pub fn drain(&self) -> Vec<RealtimeEvent> {
        self.events.drain()
    }

    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for RealtimeSubscription {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_subscription(
    controller: SocketAddr,
    secret: &str,
    topic: RealtimeTopic,
    options: &RealtimeOptions,
    cancel: &AtomicBool,
    events: &EventBuffer,
) {
    let mut attempt = 0_u32;
    while !cancel.load(Ordering::Acquire) {
        if let Ok(mut socket) = connect_stream(controller, secret, topic, options) {
            let mut received_event = false;
            loop {
                if cancel.load(Ordering::Acquire) {
                    let _ = socket.close(None);
                    return;
                }
                match socket.read() {
                    Ok(Message::Text(text)) => {
                        if let Ok(event) = parse_event(topic, text.as_str()) {
                            events.push(event);
                            received_event = true;
                        }
                    }
                    Ok(Message::Binary(bytes)) => {
                        if let Ok(text) = std::str::from_utf8(&bytes)
                            && let Ok(event) = parse_event(topic, text)
                        {
                            events.push(event);
                            received_event = true;
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(WebSocketError::Io(error))
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                }
            }
            if received_event {
                attempt = 0;
            }
        }
        if cancel.load(Ordering::Acquire) {
            break;
        }
        attempt = attempt.saturating_add(1);
        let multiplier = 1_u32 << attempt.saturating_sub(1).min(8);
        let delay = options
            .initial_backoff
            .saturating_mul(multiplier)
            .min(options.max_backoff);
        events.push(RealtimeEvent::Reconnecting { attempt, delay });
        cancellable_sleep(delay, cancel);
    }
}

fn connect_stream(
    controller: SocketAddr,
    secret: &str,
    topic: RealtimeTopic,
    options: &RealtimeOptions,
) -> Result<tungstenite::WebSocket<TcpStream>, AppError> {
    let stream =
        TcpStream::connect_timeout(&controller, options.connect_timeout).map_err(realtime_error)?;
    stream
        .set_read_timeout(Some(options.read_timeout))
        .map_err(realtime_error)?;
    stream
        .set_write_timeout(Some(options.read_timeout))
        .map_err(realtime_error)?;
    let mut request = format!("ws://{controller}{}", topic_path(topic))
        .into_client_request()
        .map_err(realtime_error)?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {secret}")).map_err(realtime_error)?,
    );
    client(request, stream)
        .map(|(socket, _)| socket)
        .map_err(realtime_error)
}

fn parse_event(topic: RealtimeTopic, text: &str) -> Result<RealtimeEvent, AppError> {
    match topic {
        RealtimeTopic::Traffic => serde_json::from_str(text)
            .map(RealtimeEvent::Traffic)
            .map_err(realtime_error),
        RealtimeTopic::Memory => serde_json::from_str(text)
            .map(RealtimeEvent::Memory)
            .map_err(realtime_error),
        RealtimeTopic::Logs => serde_json::from_str(text)
            .map(RealtimeEvent::Log)
            .map_err(realtime_error),
        RealtimeTopic::Connections => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct ConnectionsMessage {
                upload_total: u64,
                download_total: u64,
                connections: Option<Vec<RawConnection>>,
            }
            #[derive(Default, Deserialize)]
            #[serde(rename_all = "camelCase", default)]
            struct RawConnection {
                id: String,
                metadata: Metadata,
                upload: u64,
                download: u64,
                start: String,
                chains: Vec<String>,
                rule: String,
                rule_payload: String,
            }
            #[derive(Default, Deserialize)]
            #[serde(rename_all = "camelCase", default)]
            struct Metadata {
                network: String,
                #[serde(rename = "sourceIP")]
                source_ip: String,
                source_port: u16,
                #[serde(rename = "destinationIP")]
                destination_ip: String,
                destination_port: u16,
                host: String,
                process: String,
                process_path: String,
            }
            let message: ConnectionsMessage = serde_json::from_str(text).map_err(realtime_error)?;
            let connections = message
                .connections
                .unwrap_or_default()
                .into_iter()
                .map(|connection| Connection {
                    id: connection.id,
                    network: connection.metadata.network,
                    source: format_endpoint(
                        &connection.metadata.source_ip,
                        connection.metadata.source_port,
                    ),
                    destination: format_endpoint(
                        &connection.metadata.destination_ip,
                        connection.metadata.destination_port,
                    ),
                    host: connection.metadata.host,
                    process: if connection.metadata.process.is_empty() {
                        connection.metadata.process_path
                    } else {
                        connection.metadata.process
                    },
                    upload: connection.upload,
                    download: connection.download,
                    start: connection.start,
                    rule: connection.rule,
                    rule_payload: connection.rule_payload,
                    chains: connection.chains,
                })
                .collect::<Vec<_>>();
            Ok(RealtimeEvent::Connections(ConnectionSnapshot {
                upload_total: message.upload_total,
                download_total: message.download_total,
                connection_count: connections.len(),
                connections,
            }))
        }
    }
}

fn format_endpoint(host: &str, port: u16) -> String {
    if host.is_empty() {
        String::new()
    } else if port == 0 {
        host.to_owned()
    } else if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn cancellable_sleep(delay: Duration, cancel: &AtomicBool) {
    let deadline = Instant::now() + delay;
    while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(Duration::from_millis(10).min(remaining));
    }
}

fn realtime_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::CoreUnavailable, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{LogEvent, MemoryEvent, TrafficEvent};

    #[test]
    fn parses_all_realtime_payloads() {
        assert_eq!(
            parse_event(RealtimeTopic::Traffic, r#"{"up":12,"down":34}"#).unwrap(),
            RealtimeEvent::Traffic(TrafficEvent { up: 12, down: 34 })
        );
        assert_eq!(
            parse_event(RealtimeTopic::Memory, r#"{"inuse":99,"oslimit":1000}"#).unwrap(),
            RealtimeEvent::Memory(MemoryEvent {
                inuse: 99,
                oslimit: 1000,
            })
        );
        assert_eq!(
            parse_event(
                RealtimeTopic::Connections,
                r#"{"uploadTotal":1,"downloadTotal":2,"connections":[{},{}]}"#,
            )
            .unwrap(),
            RealtimeEvent::Connections(ConnectionSnapshot {
                upload_total: 1,
                download_total: 2,
                connection_count: 2,
                connections: vec![Connection::default(), Connection::default()],
            })
        );
        assert_eq!(
            parse_event(RealtimeTopic::Logs, r#"{"type":"info","payload":"ready"}"#).unwrap(),
            RealtimeEvent::Log(LogEvent {
                level: "info".into(),
                payload: "ready".into(),
            })
        );
    }

    #[test]
    fn parses_connection_details_for_ui_and_close_actions() {
        let event = parse_event(
            RealtimeTopic::Connections,
            r#"{"uploadTotal":11,"downloadTotal":22,"connections":[{"id":"abc","metadata":{"network":"tcp","sourceIP":"127.0.0.1","sourcePort":50123,"destinationIP":"1.1.1.1","destinationPort":443,"host":"example.com","process":"Browser"},"upload":3,"download":4,"start":"2026-08-29T12:00:00Z","chains":["Proxy","Node"],"rule":"DomainSuffix","rulePayload":"example.com"}]}"#,
        )
        .unwrap();
        let RealtimeEvent::Connections(snapshot) = event else {
            panic!("expected connections event");
        };
        assert_eq!(snapshot.connection_count, 1);
        assert_eq!(snapshot.connections[0].id, "abc");
        assert_eq!(snapshot.connections[0].source, "127.0.0.1:50123");
        assert_eq!(snapshot.connections[0].destination, "1.1.1.1:443");
        assert_eq!(snapshot.connections[0].host, "example.com");
        assert_eq!(snapshot.connections[0].chains, ["Proxy", "Node"]);
    }

    #[test]
    fn bounded_buffer_drops_oldest_event() {
        let buffer = EventBuffer::new(2);
        for up in 1..=3 {
            buffer.push(RealtimeEvent::Traffic(TrafficEvent { up, down: 0 }));
        }
        assert_eq!(
            buffer.drain(),
            [
                RealtimeEvent::Traffic(TrafficEvent { up: 2, down: 0 }),
                RealtimeEvent::Traffic(TrafficEvent { up: 3, down: 0 }),
            ]
        );
    }

    #[test]
    fn unavailable_controller_retries_and_can_be_cancelled() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut subscription = RealtimeSubscription::spawn(
            address,
            "secret",
            RealtimeTopic::Traffic,
            RealtimeOptions {
                connect_timeout: Duration::from_millis(20),
                read_timeout: Duration::from_millis(20),
                initial_backoff: Duration::from_millis(10),
                max_backoff: Duration::from_millis(20),
                buffer_capacity: 4,
            },
        )
        .unwrap();
        thread::sleep(Duration::from_millis(35));
        subscription.stop();
        assert!(
            subscription
                .drain()
                .iter()
                .any(|event| matches!(event, RealtimeEvent::Reconnecting { .. }))
        );
    }
}
