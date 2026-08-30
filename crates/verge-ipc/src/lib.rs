//! 守护进程与 GUI 进程之间的 Unix socket IPC。
//!
//! 协议复用 `verge-ui` 的请求/响应信封，只换传输层；额外消息负责
//! 连接生命周期（握手、批量实时事件、窗口激活、重复实例）。

pub mod client;
pub mod frame;
pub mod peer;
pub mod protocol;
pub mod server;

pub use client::{ClientEvent, ConnectError, IpcClient};
pub use protocol::{ClientMessage, DaemonMessage, InitialSnapshot, PROTOCOL_VERSION};
pub use server::{IpcServer, IpcServerEvent, SendError, ServerError};
