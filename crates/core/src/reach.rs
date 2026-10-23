//! A login-free reachability probe, cheap enough to run on a timer for every host.
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

/// Whether an SSH server answers at `address:port`. Reads the server's identification line, answers with
/// its own and leaves before key exchange, so no credential is read and nothing logs in.
pub fn probe(address: &str, port: u16, timeout: Duration) -> bool {
    let Ok(addresses) = (address, port).to_socket_addrs() else {
        return false;
    };
    for target in addresses {
        let Ok(mut stream) = TcpStream::connect_timeout(&target, timeout) else {
            continue;
        };
        if stream.set_read_timeout(Some(timeout)).is_err() {
            return false;
        }
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        // The identification line is at most 255 bytes (RFC 4253 section 4.2).
        while line.len() < 255 && !line.ends_with(b"\n") {
            match stream.read(&mut byte) {
                Ok(1) => line.push(byte[0]),
                _ => break,
            }
        }
        let answered = line.starts_with(b"SSH-");
        if answered {
            // Without our own line OpenSSH logs a client that never identified itself.
            let _ = stream.write_all(b"SSH-2.0-ssh-manager-probe\r\n");
        }
        return answered;
    }
    false
}
