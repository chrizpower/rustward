use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

use hmac::digest::CtOutput;
use hyper::HeaderMap;
use rustix::{fs::OFlags, process::geteuid};
use sha2::{Digest, Sha256};

const HEADER: &str = "x-rustward-proxy-token";
const TOKEN_LENGTH: usize = 64;

pub(super) struct ProxyToken(CtOutput<Sha256>);

impl ProxyToken {
    pub(super) fn read(path: &Path) -> Result<Self, String> {
        let read_error = |error: std::io::Error| format!("Web proxy token file: {error}");
        let file = OpenOptions::new()
            .read(true)
            .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32)
            .open(path)
            .map_err(read_error)?;
        let metadata = file.metadata().map_err(read_error)?;
        if !metadata.is_file()
            || metadata.uid() != geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err("The web proxy token must be a private file owned by the service".into());
        }
        let mut bytes = Vec::new();
        file.take((TOKEN_LENGTH + 2) as u64)
            .read_to_end(&mut bytes)
            .map_err(read_error)?;
        let token = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
        if token.len() != TOKEN_LENGTH
            || !token
                .iter()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(
                "The web proxy token must contain 64 lowercase hexadecimal characters".into(),
            );
        }
        Ok(Self(CtOutput::new(Sha256::digest(token))))
    }

    pub(super) fn accepts(&self, headers: &HeaderMap) -> bool {
        let all = headers.get_all(HEADER);
        let mut values = all.iter();
        let Some(value) = values.next() else {
            return false;
        };
        let bytes = value.as_bytes();
        values.next().is_none()
            && bytes.len() == TOKEN_LENGTH
            && CtOutput::<Sha256>::new(Sha256::digest(bytes)) == self.0
    }
}
