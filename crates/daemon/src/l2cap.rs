//! A minimal async L2CAP socket (Linux, BlueZ). The AirPods control channel is
//! plain L2CAP, which std and tokio do not cover, so we call the kernel directly.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use tokio::io::unix::AsyncFd;

/// Protocol number for L2CAP sockets (BTPROTO_L2CAP in <bluetooth/bluetooth.h>).
const BTPROTO_L2CAP: libc::c_int = 0;

/// `struct sockaddr_l2` from <bluetooth/l2cap.h>. `repr(C)` fixes the field
/// order and padding to match what the kernel expects.
#[repr(C)]
struct SockAddrL2 {
    family: libc::sa_family_t,
    psm: u16,
    bdaddr: [u8; 6],
    cid: u16,
    bdaddr_type: u8,
}

/// "AA:BB:CC:DD:EE:FF" -> the six bytes in the order BlueZ stores them (reversed).
pub fn parse_address(text: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let mut parts = text.split(':');
    for slot in out.iter_mut().rev() {
        let part = parts.next()?;
        if part.len() != 2 {
            return None;
        }
        *slot = u8::from_str_radix(part, 16).ok()?;
    }
    if parts.next().is_some() { None } else { Some(out) }
}

pub struct L2capStream {
    fd: AsyncFd<OwnedFd>,
}

impl L2capStream {
    pub async fn connect(address: [u8; 6], psm: u16) -> io::Result<Self> {
        // SAFETY: `socket` has no memory-safety preconditions. We check the result
        // and wrap the descriptor in `OwnedFd` right away so it is always closed.
        let raw = unsafe {
            libc::socket(
                libc::AF_BLUETOOTH,
                libc::SOCK_SEQPACKET | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                BTPROTO_L2CAP,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a valid descriptor that nothing else owns.
        let owned = unsafe { OwnedFd::from_raw_fd(raw) };

        let addr = SockAddrL2 {
            family: libc::AF_BLUETOOTH as libc::sa_family_t,
            psm: psm.to_le(),
            bdaddr: address,
            cid: 0,
            bdaddr_type: 0, // BDADDR_BREDR
        };
        // SAFETY: `addr` is a valid sockaddr_l2 and we pass its exact size.
        let rc = unsafe {
            libc::connect(
                owned.as_raw_fd(),
                &addr as *const SockAddrL2 as *const libc::sockaddr,
                std::mem::size_of::<SockAddrL2>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EINPROGRESS) {
                return Err(err);
            }
        }

        let fd = AsyncFd::new(owned)?;
        // A non-blocking connect finishes when the socket becomes writable;
        // SO_ERROR then says whether it worked.
        let _ = fd.writable().await?;
        let mut code: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: `code` and `len` are valid for writes of the sizes given.
        let rc = unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                &mut code as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code));
        }
        Ok(Self { fd })
    }

    /// Reads one packet. `Ok(0)` means the pods closed the channel.
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.fd.readable().await?;
            // `try_io` runs the call and clears readiness if it would block.
            let result = guard.try_io(|inner| {
                // SAFETY: `buf` is valid for writes of `buf.len()` bytes.
                let n = unsafe { libc::recv(inner.as_raw_fd(), buf.as_mut_ptr() as *mut _, buf.len(), 0) };
                if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
            });
            match result {
                Ok(r) => return r,
                Err(_would_block) => continue,
            }
        }
    }

    pub async fn write(&self, data: &[u8]) -> io::Result<()> {
        loop {
            let mut guard = self.fd.writable().await?;
            let result = guard.try_io(|inner| {
                // SAFETY: `data` is valid for reads of `data.len()` bytes.
                let n = unsafe { libc::send(inner.as_raw_fd(), data.as_ptr() as *const _, data.len(), libc::MSG_NOSIGNAL) };
                if n < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
            });
            match result {
                Ok(r) => return r,
                Err(_would_block) => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_is_reversed() {
        assert_eq!(parse_address("AA:BB:CC:DD:EE:01"), Some([0x01, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA]));
    }

    #[test]
    fn rejects_bad_addresses() {
        for bad in ["", "AA:BB", "AA:BB:CC:DD:EE:FF:00", "AA:BB:CC:DD:EE:GG", "A:BB:CC:DD:EE:FF"] {
            assert_eq!(parse_address(bad), None, "{bad}");
        }
    }

    #[test]
    fn sockaddr_matches_kernel_size() {
        assert_eq!(std::mem::size_of::<SockAddrL2>(), 14);
    }
}
