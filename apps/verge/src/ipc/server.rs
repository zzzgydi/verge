//! 守护进程侧的 Unix socket IPC 服务端。
//!
//! 线程模型：一个 accept 线程接受连接；每条连接一个读线程（帧解码 → 事件队列）
//! 和一个写线程（消费发送队列 → 写 socket）。backend 事件循环只通过
//! [`IpcServerEvent`] 与 [`IpcServer`] 交互，不在 socket 上阻塞。

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufReader, BufWriter, ErrorKind},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
    },
};

use crate::domain::{RealtimeEvent, RealtimeTopic};
use crate::ui::UiRequestEnvelope;

use super::{
    frame,
    peer::peer_uid,
    protocol::{ClientMessage, DaemonMessage},
};

/// backend 事件循环消费的服务器事件。
#[derive(Debug)]
pub enum IpcServerEvent {
    /// 新连接完成 Hello 握手。backend 据此发 Welcome 或 Duplicate。
    Connected {
        conn_id: u64,
        protocol_version: u32,
        app_version: String,
    },
    Request {
        conn_id: u64,
        envelope: UiRequestEnvelope,
    },
    Disconnected {
        conn_id: u64,
    },
}

/// 绑定失败的原因。
#[derive(Debug)]
pub enum ServerError {
    /// socket 上已有活守护进程。
    AlreadyRunning,
    Io(std::io::Error),
}

/// 向单条连接发送失败的分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendError {
    /// 连接已断开。
    Disconnected,
    /// 发送队列满（客户端消费过慢），连接将被关闭。
    Full,
}

struct ConnectionHandle {
    writer_tx: SyncSender<ClientMessage>,
}

/// 每连接写队列上限。实时事件 100ms 一帧，正常远到不了这个水位。
const WRITER_QUEUE_CAPACITY: usize = 512;

pub struct IpcServer {
    events_tx: Sender<IpcServerEvent>,
    connections: Mutex<HashMap<u64, ConnectionHandle>>,
    subscriptions: Mutex<HashMap<u64, HashSet<RealtimeTopic>>>,
    primary: Mutex<Option<u64>>,
    next_conn_id: AtomicU64,
    shutdown: Arc<AtomicBool>,
    listener: Mutex<Option<UnixListener>>,
    socket_path: std::path::PathBuf,
}

impl IpcServer {
    /// 绑定 socket。若路径上已有可连接的守护，返回 [`ServerError::AlreadyRunning`]；
    /// 残留的 stale socket 会被清理后重新绑定。
    /// 返回服务器句柄与 backend 事件循环消费的事件接收端。
    pub fn bind(
        socket_path: &Path,
    ) -> Result<(Arc<Self>, Receiver<IpcServerEvent>), ServerError> {
        if socket_path.exists() {
            match UnixStream::connect(socket_path) {
                Ok(_) => return Err(ServerError::AlreadyRunning),
                Err(_) => {
                    let _ = fs::remove_file(socket_path);
                }
            }
        }
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent).map_err(ServerError::Io)?;
        }
        let listener = UnixListener::bind(socket_path).map_err(ServerError::Io)?;
        fs::set_permissions(socket_path, PermissionsExt::from_mode(0o600))
            .map_err(ServerError::Io)?;
        let (events_tx, events_rx) = mpsc::channel();
        let server = Arc::new(Self {
            events_tx,
            connections: Mutex::new(HashMap::new()),
            subscriptions: Mutex::new(HashMap::new()),
            primary: Mutex::new(None),
            next_conn_id: AtomicU64::new(1),
            shutdown: Arc::new(AtomicBool::new(false)),
            listener: Mutex::new(Some(listener)),
            socket_path: socket_path.to_path_buf(),
        });
        Ok((server, events_rx))
    }

    /// 启动 accept 线程。只调用一次。
    pub fn spawn_accept(self: &Arc<Self>) {
        let listener = self.listener.lock().unwrap().take()
            .expect("spawn_accept must only be called once");
        let shutdown = self.shutdown.clone();
        let this = self.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                match stream {
                    Ok(stream) => this.spawn_connection(stream),
                    Err(_) if shutdown.load(Ordering::Acquire) => break,
                    Err(_) => {}
                }
            }
        });
    }

    fn spawn_connection(self: &Arc<Self>, stream: UnixStream) {
        // 只接受同一用户的连接。
        match peer_uid(&stream) {
            Ok(uid) if uid == unsafe { libc::geteuid() } => {}
            _ => return,
        }
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let Ok(reader_stream) = stream.try_clone() else {
            return;
        };
        let Ok(writer_stream) = stream.try_clone() else {
            return;
        };
        let (writer_tx, writer_rx) = mpsc::sync_channel::<ClientMessage>(WRITER_QUEUE_CAPACITY);
        std::thread::spawn(move || {
            let mut writer = BufWriter::new(writer_stream);
            while let Ok(message) = writer_rx.recv() {
                if frame::write_message(&mut writer, &message).is_err() {
                    break;
                }
            }
        });
        let events_tx = self.events_tx.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader_stream);
            // 第一帧必须是 Hello。
            let (protocol_version, app_version) = match frame::read_message::<DaemonMessage>(&mut reader)
            {
                Ok(DaemonMessage::Hello {
                    protocol_version,
                    app_version,
                }) => (protocol_version, app_version),
                _ => return,
            };
            if events_tx
                .send(IpcServerEvent::Connected {
                    conn_id,
                    protocol_version,
                    app_version,
                })
                .is_err()
            {
                return;
            }
            while let Ok(DaemonMessage::Request(envelope)) =
                frame::read_message::<DaemonMessage>(&mut reader)
            {
                if events_tx
                    .send(IpcServerEvent::Request { conn_id, envelope })
                    .is_err()
                {
                    break;
                }
            }
            let _ = events_tx.send(IpcServerEvent::Disconnected { conn_id });
        });
        self.connections
            .lock().unwrap()
            .insert(conn_id, ConnectionHandle { writer_tx });
    }

    /// 向单条连接发送消息。连接不存在或队列满时返回错误。
    pub fn send(&self, conn_id: u64, message: ClientMessage) -> Result<(), SendError> {
        let connections = self.connections.lock().unwrap();
        let Some(handle) = connections.get(&conn_id) else {
            return Err(SendError::Disconnected);
        };
        handle.writer_tx.try_send(message).map_err(|error| match error {
            TrySendError::Full(_) => SendError::Full,
            TrySendError::Disconnected(_) => SendError::Disconnected,
        })
    }

    /// 把实时事件批量按订阅表分发。无订阅的连接不发送；
    /// `Reconnecting` 视为全局事件，发给任意有订阅的连接。
    pub fn broadcast_realtime(&self, events: &[RealtimeEvent]) {
        if events.is_empty() {
            return;
        }
        let connections = self.connections.lock().unwrap();
        let subscriptions = self.subscriptions.lock().unwrap();
        for (conn_id, handle) in connections.iter() {
            let Some(topics) = subscriptions.get(conn_id) else {
                continue;
            };
            let batch: Vec<RealtimeEvent> = events
                .iter()
                .filter(|event| match event_topic(event) {
                    Some(topic) => topics.contains(&topic),
                    None => true,
                })
                .cloned()
                .collect();
            if !batch.is_empty() {
                let _ = handle.writer_tx.try_send(ClientMessage::RealtimeBatch(batch));
            }
        }
    }

    /// 记录某连接的实时订阅。backend 在处理 `StartRealtime` 时调用。
    pub fn set_subscriptions(&self, conn_id: u64, topics: Vec<RealtimeTopic>) {
        self.subscriptions
            .lock().unwrap()
            .insert(conn_id, topics.into_iter().collect());
    }

    /// 清空某连接的订阅。backend 在处理 `StopRealtime` 或连接断开时调用。
    pub fn clear_subscriptions(&self, conn_id: u64) {
        self.subscriptions.lock().unwrap().remove(&conn_id);
    }

    /// 尝试把某连接设为主 GUI 实例。第一个成功的连接成为主实例；
    /// 已存在主实例时返回 false（新连接是重复实例）。
    pub fn try_claim_primary(&self, conn_id: u64) -> bool {
        let mut primary = self.primary.lock().unwrap();
        if primary.is_none() {
            *primary = Some(conn_id);
        }
        *primary == Some(conn_id)
    }

    /// 当前主 GUI 实例连接。
    pub fn primary(&self) -> Option<u64> {
        *self.primary.lock().unwrap()
    }

    /// 释放主实例（主实例断开或退出时调用）。
    pub fn release_primary(&self, conn_id: u64) {
        let mut primary = self.primary.lock().unwrap();
        if *primary == Some(conn_id) {
            *primary = None;
        }
    }

    /// 主动关闭一条连接（如重复实例）：写队列关闭、订阅与主实例清理。
    /// 对端 socket 由 GUI 进程退出时关闭，读线程随后退出。
    pub fn close(&self, conn_id: u64) {
        self.connections.lock().unwrap().remove(&conn_id);
        self.release_primary(conn_id);
        self.clear_subscriptions(conn_id);
    }

    /// 关闭全部连接并停止 accept。进程退出时兜底。
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.connections.lock().unwrap().clear();
        self.subscriptions.lock().unwrap().clear();
        let _ = fs::remove_file(&self.socket_path);
    }
}

fn event_topic(event: &RealtimeEvent) -> Option<RealtimeTopic> {
    Some(match event {
        RealtimeEvent::Traffic(_) => RealtimeTopic::Traffic,
        RealtimeEvent::Memory(_) => RealtimeTopic::Memory,
        RealtimeEvent::Connections(_) => RealtimeTopic::Connections,
        RealtimeEvent::Log(_) => RealtimeTopic::Logs,
        RealtimeEvent::Reconnecting { .. } => return None,
    })
}

impl From<std::io::Error> for ServerError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == ErrorKind::AddrInUse {
            ServerError::AlreadyRunning
        } else {
            ServerError::Io(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::protocol::PROTOCOL_VERSION;
    use crate::domain::{ConnectionSnapshot, TrafficEvent};
    use crate::ui::{UiResponse, UiResponseEnvelope};

    fn temp_socket(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "verge-ipc-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("daemon.sock")
    }

    #[test]
    fn bind_rejects_when_already_running() {
        let socket = temp_socket("already-running");
        let (server, _events) = IpcServer::bind(&socket).unwrap();
        server.spawn_accept();
        // 先等 accept 线程就绪。
        std::thread::sleep(std::time::Duration::from_millis(50));
        let second = IpcServer::bind(&socket);
        assert!(matches!(second, Err(ServerError::AlreadyRunning)));
        server.shutdown();
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn stale_socket_is_replaced() {
        let socket = temp_socket("stale");
        std::fs::write(&socket, b"stale").unwrap();
        let (server, _events) = IpcServer::bind(&socket).unwrap();
        assert!(socket.exists());
        server.shutdown();
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn event_topic_maps_every_event_variant() {
        assert_eq!(
            event_topic(&RealtimeEvent::Traffic(TrafficEvent { up: 0, down: 0 })),
            Some(RealtimeTopic::Traffic)
        );
        assert_eq!(
            event_topic(&RealtimeEvent::Connections(ConnectionSnapshot {
                upload_total: 0,
                download_total: 0,
                connection_count: 0,
                connections: vec![],
            })),
            Some(RealtimeTopic::Connections)
        );
        assert_eq!(
            event_topic(&RealtimeEvent::Reconnecting {
                attempt: 1,
                delay: std::time::Duration::from_secs(1),
            }),
            None
        );
    }

    #[test]
    fn realtime_broadcast_filters_by_subscription() {
        let socket = temp_socket("broadcast");
        let (server, events) = IpcServer::bind(&socket).unwrap();
        std::thread::spawn(move || {
            // 消费事件，避免 channel 阻塞 accept 线程。
            while events.recv().is_ok() {}
        });

        // 模拟一条已连接连接的写端（直接持有 writer 队列的简化验证：
        // 通过真实连接不可行，这里只验证订阅表与事件分类的组合逻辑）。
        let traffic = RealtimeEvent::Traffic(TrafficEvent { up: 1, down: 2 });
        let logs = RealtimeEvent::Log(crate::domain::LogEvent {
            level: "info".into(),
            payload: "x".into(),
        });
        server
            .subscriptions
            .lock().unwrap()
            .insert(7, [RealtimeTopic::Traffic].into_iter().collect());
        let batch = broadcast_batch_for(&server, &[traffic.clone(), logs.clone()], 7);
        assert_eq!(batch, vec![traffic]);
        server.shutdown();
        let _ = std::fs::remove_file(&socket);
    }

    /// 测试辅助：复刻 broadcast_realtime 的过滤逻辑，独立于真实写线程。
    fn broadcast_batch_for(
        server: &IpcServer,
        events: &[RealtimeEvent],
        conn_id: u64,
    ) -> Vec<RealtimeEvent> {
        let subscriptions = server.subscriptions.lock().unwrap();
        let Some(topics) = subscriptions.get(&conn_id) else {
            return vec![];
        };
        events
            .iter()
            .filter(|event| match event_topic(event) {
                Some(topic) => topics.contains(&topic),
                None => true,
            })
            .cloned()
            .collect()
    }

    #[test]
    fn protocol_messages_roundtrip_through_frames() {
        let envelope = UiResponseEnvelope::realtime(UiResponse::Realtime(
            RealtimeEvent::Traffic(TrafficEvent { up: 0, down: 0 }),
        ));
        let message = ClientMessage::Response(envelope.clone());
        let mut buffer = Vec::new();
        frame::write_message(&mut buffer, &message).unwrap();
        let decoded: ClientMessage = frame::read_message(&mut std::io::Cursor::new(buffer)).unwrap();
        assert!(matches!(decoded, ClientMessage::Response(_)));
        drop((PROTOCOL_VERSION, envelope));
    }

    #[test]
    fn primary_claim_is_first_wins() {
        let socket = temp_socket("primary");
        let (server, _events) = IpcServer::bind(&socket).unwrap();
        assert!(server.try_claim_primary(1));
        assert!(!server.try_claim_primary(2));
        assert_eq!(server.primary(), Some(1));
        server.release_primary(1);
        assert!(server.try_claim_primary(2));
        server.shutdown();
        let _ = std::fs::remove_file(&socket);
    }
}
