use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixListener, UnixStream},
    },
    thread,
    time::{Duration, Instant},
};

use rustix::{net::sockopt::socket_peercred, process::geteuid};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const LIMIT: u64 = 128 * 1024;
const IO_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    IssueKey,
    ServiceStatus,
    Submit {
        key: String,
        task_id: String,
        #[serde(default)]
        inputs: BTreeMap<String, toml::Value>,
    },
    Status {
        run_id: String,
    },
    Cancel {
        run_id: String,
    },
    Pause,
    Resume,
    Reviews {
        after: Option<String>,
    },
    Acknowledge {
        acknowledgement_id: String,
        review: Box<crate::review::Review>,
        note: String,
    },
    Shutdown,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Response {
    Ok(serde_json::Value),
    Error(String),
}

fn address(name: &str) -> io::Result<SocketAddr> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid service name",
        ));
    }
    SocketAddr::from_abstract_name(format!("rustward-{}-{name}", geteuid().as_raw()))
}

pub(super) fn listen(name: &str) -> io::Result<UnixListener> {
    let listener = UnixListener::bind_addr(&address(name)?)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn check_peer(stream: &UnixStream) -> io::Result<()> {
    if socket_peercred(stream)?.uid != geteuid() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

pub(super) fn receive(stream: &mut UnixStream) -> io::Result<Request> {
    check_peer(stream)?;
    let mut bytes = Vec::new();
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut buffer = [0; 4096];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        stream.set_read_timeout(Some(remaining))?;
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return decode(&bytes);
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() as u64 > LIMIT {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
}

pub(super) fn respond(stream: &mut UnixStream, response: &Response) -> io::Result<()> {
    let bytes = serde_json::to_vec(response).map_err(io::Error::other)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut remaining = bytes.as_slice();
    while !remaining.is_empty() {
        let time = deadline.saturating_duration_since(Instant::now());
        if time.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        stream.set_write_timeout(Some(time))?;
        let written = stream.write(remaining)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        remaining = &remaining[written..];
    }
    Ok(())
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> io::Result<T> {
    serde_json::from_slice(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid request or response"))
}

pub fn call(name: &str, input: impl Read, mut output: impl Write) -> io::Result<bool> {
    let mut bytes = Vec::new();
    input.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request is too large",
        ));
    }
    let _: Request = decode(&bytes)?;
    let mut stream = UnixStream::connect_addr(&address(name)?)?;
    check_peer(&stream)?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.write_all(&bytes)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    bytes.clear();
    stream.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let response: Response = decode(&bytes)?;
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    Ok(matches!(response, Response::Ok(_)))
}

pub(super) fn idle() {
    thread::sleep(Duration::from_millis(20));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_messages_in_both_directions() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        let worker = thread::spawn(move || {
            let _ = peer.write_all(&vec![b' '; LIMIT as usize + 1]);
        });
        assert_eq!(
            receive(&mut stream).err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
        worker.join().unwrap();
        assert_eq!(
            respond(&mut stream, &Response::Error("x".repeat(LIMIT as usize)))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let name = format!("oversize-test-{}", std::process::id());
        let listener = UnixListener::bind_addr(&address(&name).unwrap()).unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            receive(&mut stream).unwrap();
            stream.write_all(&vec![b' '; LIMIT as usize + 1]).unwrap();
        });
        assert_eq!(
            call(&name, br#"{"command":"issue_key"}"#.as_slice(), io::sink())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        worker.join().unwrap();
    }
}
