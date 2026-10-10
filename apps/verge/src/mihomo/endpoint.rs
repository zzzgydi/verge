use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    time::Duration,
};

/// Both REST and WebSocket use the same local transport.
#[derive(Clone, Debug)]
pub enum ControllerEndpoint {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}
impl From<SocketAddr> for ControllerEndpoint {
    fn from(address: SocketAddr) -> Self {
        Self::Tcp(address)
    }
}
impl ControllerEndpoint {
    pub fn connect(&self, timeout: Duration) -> io::Result<ControllerStream> {
        match self {
            Self::Tcp(address) => {
                let stream = TcpStream::connect_timeout(address, timeout)?;
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                Ok(ControllerStream::Tcp(stream))
            }
            #[cfg(unix)]
            Self::Unix(path) => {
                let stream = std::os::unix::net::UnixStream::connect(path)?;
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                Ok(ControllerStream::Unix(stream))
            }
        }
    }
    pub fn host(&self) -> String {
        match self {
            Self::Tcp(address) => address.to_string(),
            #[cfg(unix)]
            Self::Unix(_) => "localhost".into(),
        }
    }
    pub fn is_local(&self) -> bool {
        match self {
            Self::Tcp(address) => address.ip().is_loopback(),
            #[cfg(unix)]
            Self::Unix(path) => path.is_absolute(),
        }
    }
}
#[derive(Debug)]
pub enum ControllerStream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}
impl ControllerStream {
    pub fn set_nonblocking(&self, enabled: bool) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.set_nonblocking(enabled),
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_nonblocking(enabled),
        }
    }

    pub fn set_timeout(&self, timeout: Duration) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))
            }
            #[cfg(unix)]
            Self::Unix(stream) => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))
            }
        }
    }

    pub fn wait_readable(&self, timeout: Duration) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd as _;

            // macOS rejects setsockopt(SO_RCVTIMEO) after a peer closes, even if
            // its response is still buffered. Polling also keeps the total deadline.
            let fd = match self {
                Self::Tcp(stream) => stream.as_raw_fd(),
                Self::Unix(stream) => stream.as_raw_fd(),
            };
            let mut ready = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let millis = timeout
                .as_nanos()
                .div_ceil(1_000_000)
                .min(libc::c_int::MAX as u128) as libc::c_int;
            let count = unsafe { libc::poll(&mut ready, 1, millis) };
            if count < 0 {
                Err(io::Error::last_os_error())
            } else if count == 0 {
                Err(io::ErrorKind::TimedOut.into())
            } else if ready.revents & libc::POLLNVAL != 0 {
                Err(io::Error::from_raw_os_error(libc::EBADF))
            } else {
                Ok(())
            }
        }
        #[cfg(not(unix))]
        self.set_timeout(timeout)
    }
}
impl Read for ControllerStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(bytes),
            #[cfg(unix)]
            Self::Unix(s) => s.read(bytes),
        }
    }
}
impl Write for ControllerStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(bytes),
            #[cfg(unix)]
            Self::Unix(s) => s.write(bytes),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Self::Unix(s) => s.flush(),
        }
    }
}
