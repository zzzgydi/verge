use std::{
    env,
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
};

use verge_helper::{prepare_socket_path, restrict_socket, serve_connection};

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
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                std::thread::spawn(move || {
                    let _ = serve_connection(stream, allowed_uid);
                });
            }
            Err(error) => eprintln!("helper accept failed: {error}"),
        }
    }
    Ok(())
}
