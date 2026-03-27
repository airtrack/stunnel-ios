use std::io;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub mode: String,
    pub server_addr: String,
    pub server_name: String,
    pub cert: String,
    pub priv_key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelMode {
    S2nQuic,
    TlsTcp,
}

impl TunnelMode {
    pub fn parse(value: &str) -> io::Result<Self> {
        match value {
            "s2n-quic" => Ok(Self::S2nQuic),
            "tlstcp" => Ok(Self::TlsTcp),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported tunnel mode: {}", value),
            )),
        }
    }
}

impl AppConfig {
    pub fn tunnel_mode(&self) -> io::Result<TunnelMode> {
        TunnelMode::parse(&self.mode)
    }
}
