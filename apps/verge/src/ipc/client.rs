//! GUI 进程侧的 IPC 客户端。
//!
//! `connect` 同步完成 Hello 握手并取回初始快照，之后：
//! - 读线程把服务端消息转成 [`ClientEvent`] 推到 futures channel，GUI 主协程
//!   用 `into_events().next().await` 事件驱动消费（无轮询）；
//! - 写线程从 std mpsc 消费 `UiRequestEnvelope` 写 socket，UI 线程永不阻塞在 IO。
//!   `request_sender()` 直接返回该通道，视图层可原样持有。

use std::{
    fmt,
    io::{BufReader, BufWriter},
    os::unix::net::UnixStream,
    path::Path,
    sync::mpsc::{self, Sender},
    time::Duration,
};

use crate::domain::RealtimeEvent;
use crate::ui::UiRequestEnvelope;
use futures::{SinkExt as _, channel::mpsc::Receiver};

use super::{
    frame,
    protocol::{ClientMessage, DaemonMessage, InitialSnapshot, PROTOCOL_VERSION},
};

/// 服务端推送给 GUI 的事件。
#[derive(Clone, Debug)]
pub enum ClientEvent {
    /// 握手成功（connect 已同步返回，此事件保证在事件流最前）。
    Welcome {
        protocol_version: u32,
        initial: InitialSnapshot,
    },
    Response(crate::ui::UiResponseEnvelope),
    RealtimeBatch(Vec<RealtimeEvent>),
    ActivateWindow,
    HideWindow,
    /// 已有主 GUI 实例，本实例应立即退出。
    Duplicate,
    Closed {
        reason: String,
    },
}

/// 连接失败的分类，GUI 据此决定是否拉起守护进程重试。
#[derive(Debug)]
pub enum ConnectError {
    /// socket 不存在或无法连接（无守护进程）。
    Io(std::io::Error),
    /// 协议版本不匹配。
    VersionMismatch { server: u32, client: u32 },
    /// 服务端主动关闭（如守护已有关闭原因）。
    Closed { reason: String },
    /// 已有主 GUI 实例在运行，本实例应直接退出（守护已通知旧实例激活窗口）。
    Duplicate,
    /// 握手阶段收到非法消息。
    Protocol(String),
}

pub struct IpcClient {
    events_rx: Receiver<ClientEvent>,
    requests_tx: Sender<UiRequestEnvelope>,
}

impl fmt::Debug for IpcClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("IpcClient").finish_non_exhaustive()
    }
}

/// 握手超时：守护进程正在启动时客户端重试窗口由调用方控制，这里只防死等。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

impl IpcClient {
    /// 连接并完成握手。成功后返回客户端；调用方应在连接失败时决定
    /// 是否清理 stale socket 并拉起守护进程后重试。
    pub fn connect(socket_path: &Path, app_version: &str) -> Result<Self, ConnectError> {
        let stream = UnixStream::connect(socket_path).map_err(ConnectError::Io)?;
        stream
            .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
            .map_err(ConnectError::Io)?;
        let reader_stream = stream.try_clone().map_err(ConnectError::Io)?;
        let writer_stream = stream.try_clone().map_err(ConnectError::Io)?;
        let mut reader = BufReader::new(reader_stream);
        let mut writer = BufWriter::new(writer_stream);

        frame::write_message(
            &mut writer,
            &DaemonMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                app_version: app_version.to_string(),
            },
        )
        .map_err(ConnectError::Io)?;

        let initial =
            match frame::read_message::<ClientMessage>(&mut reader).map_err(ConnectError::Io)? {
                ClientMessage::Welcome {
                    protocol_version,
                    initial,
                } if protocol_version == PROTOCOL_VERSION => initial,
                ClientMessage::Welcome {
                    protocol_version, ..
                } => {
                    return Err(ConnectError::VersionMismatch {
                        server: protocol_version,
                        client: PROTOCOL_VERSION,
                    });
                }
                ClientMessage::Closed { reason } => {
                    return Err(ConnectError::Closed { reason });
                }
                ClientMessage::Duplicate => {
                    return Err(ConnectError::Duplicate);
                }
                other => {
                    return Err(ConnectError::Protocol(format!(
                        "unexpected handshake message: {other:?}"
                    )));
                }
            };

        // 握手完成后读线程应无限阻塞等待后续消息。
        reader
            .get_mut()
            .set_read_timeout(None)
            .map_err(ConnectError::Io)?;

        // Backpressure stays on the socket reader thread, never the GPUI thread.
        let (mut events_tx, events_rx) = futures::channel::mpsc::channel::<ClientEvent>(8);
        events_tx
            .try_send(ClientEvent::Welcome {
                protocol_version: PROTOCOL_VERSION,
                initial,
            })
            .expect("new event queue has room for Welcome");

        std::thread::spawn(move || {
            let mut reader = reader;
            while let Ok(message) = frame::read_message::<ClientMessage>(&mut reader) {
                let event = match message {
                    ClientMessage::Welcome { .. } => continue,
                    ClientMessage::Response(envelope) => ClientEvent::Response(envelope),
                    ClientMessage::RealtimeBatch(events) => ClientEvent::RealtimeBatch(events),
                    ClientMessage::ActivateWindow => ClientEvent::ActivateWindow,
                    ClientMessage::HideWindow => ClientEvent::HideWindow,
                    ClientMessage::Duplicate => ClientEvent::Duplicate,
                    ClientMessage::Closed { reason } => ClientEvent::Closed { reason },
                };
                let closed = matches!(event, ClientEvent::Closed { .. });
                if futures::executor::block_on(events_tx.send(event)).is_err() || closed {
                    break;
                }
            }
        });

        let (requests_tx, requests_rx) = mpsc::channel::<UiRequestEnvelope>();
        std::thread::spawn(move || {
            let mut writer = writer;
            while let Ok(envelope) = requests_rx.recv() {
                if frame::write_message(&mut writer, &DaemonMessage::Request(envelope)).is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            events_rx,
            requests_tx,
        })
    }

    /// 事件接收端（futures channel，单消费者）。GUI 主协程
    /// `while let Some(event) = events.next().await` 消费；无事件时空闲挂起，
    /// CPU 为零。消费前先取走 `request_sender()`。
    pub fn into_events(self) -> Receiver<ClientEvent> {
        self.events_rx
    }

    /// 请求发送端（std mpsc）。视图层 `MainView::new` 直接持有。
    pub fn request_sender(&self) -> Sender<UiRequestEnvelope> {
        self.requests_tx.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::super::server::{IpcServer, IpcServerEvent};
    use super::*;
    use futures::StreamExt;

    fn temp_socket(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "verge-ipc-client-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("daemon.sock")
    }

    /// 最小假守护：accept 后应答 Welcome。
    fn spawn_fake_daemon(socket: &std::path::Path, response: ClientMessage) {
        let socket = socket.to_path_buf();
        std::thread::spawn(move || {
            use std::os::unix::net::UnixListener;
            let listener = UnixListener::bind(&socket).unwrap();
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = BufWriter::new(stream);
            let _hello: DaemonMessage = frame::read_message(&mut reader).unwrap();
            frame::write_message(&mut writer, &response).unwrap();
            // 保持连接直到测试结束。
            std::thread::sleep(Duration::from_secs(10));
        });
    }

    #[test]
    fn slow_consumer_backpressures_reader_and_preserves_event_order() {
        use futures::{FutureExt as _, StreamExt as _};
        let (mut tx, mut rx) = futures::channel::mpsc::channel(1);
        tx.try_send(ClientEvent::ActivateWindow).unwrap();
        tx.try_send(ClientEvent::HideWindow).unwrap(); // futures reserves one sender slot
        let pending = tx.send(ClientEvent::Duplicate);
        futures::pin_mut!(pending);
        assert!(pending.as_mut().now_or_never().is_none());
        assert!(matches!(
            futures::executor::block_on(rx.next()),
            Some(ClientEvent::ActivateWindow)
        ));
        drop(rx);
        assert!(futures::executor::block_on(pending).is_err());
    }

    #[test]
    fn connect_receives_welcome_and_initial_snapshot() {
        let socket = temp_socket("welcome");
        spawn_fake_daemon(
            &socket,
            ClientMessage::Welcome {
                protocol_version: PROTOCOL_VERSION,
                initial: InitialSnapshot::default(),
            },
        );
        std::thread::sleep(Duration::from_millis(100));
        let client = IpcClient::connect(&socket, "test").unwrap();
        let mut events = client.into_events();
        match futures::executor::block_on(events.next()) {
            Some(ClientEvent::Welcome { .. }) => {}
            other => panic!("expected Welcome, got {other:?}"),
        }

        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn version_mismatch_is_reported() {
        let socket = temp_socket("mismatch");
        spawn_fake_daemon(
            &socket,
            ClientMessage::Welcome {
                protocol_version: PROTOCOL_VERSION + 1,
                initial: InitialSnapshot::default(),
            },
        );
        std::thread::sleep(Duration::from_millis(100));
        let error = IpcClient::connect(&socket, "test").unwrap_err();
        assert!(matches!(error, ConnectError::VersionMismatch { .. }));
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn closed_reason_propagates() {
        let socket = temp_socket("closed");
        spawn_fake_daemon(
            &socket,
            ClientMessage::Closed {
                reason: "shutting down".into(),
            },
        );
        std::thread::sleep(Duration::from_millis(100));
        let error = IpcClient::connect(&socket, "test").unwrap_err();
        assert!(matches!(error, ConnectError::Closed { .. }));
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn refused_connection_is_io_error() {
        let socket = temp_socket("refused");
        let error = IpcClient::connect(&socket, "test").unwrap_err();
        assert!(matches!(error, ConnectError::Io(_)));
    }

    #[test]
    fn requests_flow_to_daemon_side() {
        let socket = temp_socket("requests");
        let socket_server = socket.clone();
        let daemon = std::thread::spawn(move || {
            let (server, events) = IpcServer::bind(&socket_server).unwrap();
            server.spawn_accept();
            // 消费事件直到收到一个 Request。
            loop {
                match events.recv_timeout(Duration::from_secs(5)) {
                    Ok(IpcServerEvent::Connected { conn_id, .. }) => {
                        let _ = server.send(
                            conn_id,
                            ClientMessage::Welcome {
                                protocol_version: PROTOCOL_VERSION,
                                initial: InitialSnapshot::default(),
                            },
                        );
                    }
                    Ok(IpcServerEvent::Request { .. }) => break,
                    _ => {}
                }
            }
            server
        });
        std::thread::sleep(Duration::from_millis(100));
        let client = IpcClient::connect(&socket, "test").unwrap();
        let sender = client.request_sender();
        let request = UiRequestEnvelope {
            request_id: 1,
            operation_id: 1,
            operation_index: 0,
            operation_len: 1,
            context: crate::domain::CommandContext {
                actor: crate::domain::CommandActor::UserInterface,
                approval: None,
            },
            request: crate::ui::UiRequest::Runtime(crate::domain::RuntimeCommand::GetMode),
        };
        sender.send(request).unwrap();
        let server = daemon.join().unwrap();
        server.shutdown();
        let _ = std::fs::remove_file(&socket);
    }
}
