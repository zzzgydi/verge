//! Transfer one owned descriptor independently of the JSON stream framing.
use std::{
    io, mem,
    os::{
        fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
        unix::net::UnixStream,
    },
};

pub fn send(stream: &UnixStream, fd: Option<BorrowedFd<'_>>) -> io::Result<()> {
    let mut marker = if fd.is_some() { b'F' } else { b'E' };
    let mut vector = libc::iovec {
        iov_base: (&raw mut marker).cast(),
        iov_len: 1,
    };
    // usize gives the ancillary data the alignment required by cmsghdr.
    let mut control = [0usize; 8];
    // SAFETY: zero is a valid empty msghdr; every buffer is valid until sendmsg returns.
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &raw mut vector;
    message.msg_iovlen = 1;
    if let Some(fd) = fd {
        message.msg_control = control.as_mut_ptr().cast();
        // SAFETY: the control buffer has room for one c_int and its header.
        unsafe {
            message.msg_controllen = libc::CMSG_SPACE(mem::size_of::<libc::c_int>() as _) as _;
            let header = libc::CMSG_FIRSTHDR(&message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(mem::size_of::<libc::c_int>() as _) as _;
            std::ptr::write_unaligned(
                libc::CMSG_DATA(header).cast::<libc::c_int>(),
                fd.as_raw_fd(),
            );
        }
    }
    loop {
        // MSG_NOSIGNAL prevents an abandoned client from terminating the helper.
        // SAFETY: message and its buffers are initialized and the socket is borrowed.
        let sent = unsafe { libc::sendmsg(stream.as_raw_fd(), &message, libc::MSG_NOSIGNAL) };
        if sent == 1 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if sent < 0 && error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(if sent == 0 {
            io::Error::from(io::ErrorKind::WriteZero)
        } else {
            error
        });
    }
}

pub fn receive(stream: &UnixStream) -> io::Result<Option<OwnedFd>> {
    let mut marker = 0u8;
    let mut vector = libc::iovec {
        iov_base: (&raw mut marker).cast(),
        iov_len: 1,
    };
    let mut control = [0usize; 8];
    // SAFETY: all receive buffers are valid, initialized and aligned.
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &raw mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = mem::size_of_val(&control) as _;
    loop {
        message.msg_controllen = mem::size_of_val(&control) as _;
        message.msg_flags = 0;
        // SAFETY: recvmsg owns any received descriptors, immediately wrapped below.
        let count = unsafe { libc::recvmsg(stream.as_raw_fd(), &raw mut message, 0) };
        if count > 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if count < 0 && error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(if count == 0 {
            io::Error::from(io::ErrorKind::UnexpectedEof)
        } else {
            error
        });
    }
    let mut descriptors = Vec::new();
    // SAFETY: the kernel wrote bounded, aligned ancillary data; traverse only its headers.
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&message);
        while !header.is_null() {
            let end = message.msg_control as usize + message.msg_controllen as usize;
            if (*header).cmsg_len < libc::CMSG_LEN(0) as _
                || header as usize + (*header).cmsg_len as usize > end
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid ancillary length",
                ));
            }
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let length =
                    ((*header).cmsg_len as usize).saturating_sub(libc::CMSG_LEN(0) as usize);
                for index in 0..length / mem::size_of::<libc::c_int>() {
                    let fd = std::ptr::read_unaligned(
                        libc::CMSG_DATA(header).cast::<libc::c_int>().add(index),
                    );
                    descriptors.push(OwnedFd::from_raw_fd(fd));
                }
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
    }
    if message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0
        || !matches!((marker, descriptors.len()), (b'F', 1) | (b'E', 0))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid helper descriptor frame",
        ));
    }
    if let Some(fd) = descriptors.pop() {
        // SAFETY: set close-on-exec on the newly received, owned descriptor.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(fd))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::File,
        io::{Read, Write},
        os::fd::AsFd,
    };
    #[test]
    fn descriptor_survives_sender_close_and_json_remains_separate() {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let (source, mut peer) = UnixStream::pair().unwrap();
        send(&a, Some(source.as_fd())).unwrap();
        a.write_all(b"json\n").unwrap();
        drop(source);
        let received = receive(&b).unwrap().unwrap();
        let mut header = [0; 5];
        b.read_exact(&mut header).unwrap();
        assert_eq!(&header, b"json\n");
        let mut file = File::from(received);
        peer.write_all(b"data").unwrap();
        let mut bytes = [0; 4];
        file.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"data");
        drop(file);
        assert_eq!(peer.read(&mut bytes).unwrap(), 0);
    }
    #[test]
    fn error_frame_has_no_descriptor() {
        let (a, b) = UnixStream::pair().unwrap();
        send(&a, None).unwrap();
        assert!(receive(&b).unwrap().is_none());
    }
}
