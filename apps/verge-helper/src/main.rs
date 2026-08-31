use std::{
    env,
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
};

use verge_helper::{MacTun, prepare_socket_path, restrict_socket, serve_connection, shared_tun_backend};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    if arguments.next().as_deref() != Some("--socket") {
        return Err("usage: verge-helper --socket /var/run/verge-helper.sock --uid UID".into());
    }
    let socket = PathBuf::from(arguments.next().ok_or("missing socket path")?);
    if arguments.next().as_deref() != Some("--uid") {
        return Err("missing --uid".into());
    }
    let allowed_uid = arguments.next().ok_or("missing uid")?.parse::<u32>()?;
    if arguments.next().is_some() {
        return Err("unexpected helper arguments".into());
    }
    if socket.as_path() != Path::new("/var/run/verge-helper.sock") {
        return Err("helper socket must be /var/run/verge-helper.sock".into());
    }
    prepare_socket_path(&socket)?;
    let listener = UnixListener::bind(&socket)?;
    restrict_socket(&socket)?;
    // TUN 设备状态跨连接共享（fd 由 helper 进程持有，连接关闭不销毁设备）。
    let tun = shared_tun_backend(MacTun::default());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let tun = tun.clone();
                std::thread::spawn(move || {
                    let _ = serve_connection(stream, allowed_uid, &tun);
                });
            }
            Err(error) => eprintln!("helper accept failed: {error}"),
        }
    }
    Ok(())
}
