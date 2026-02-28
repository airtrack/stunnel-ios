use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub mode: String, // "tlstcp", "quinn-quic", "s2n-quic"
    pub server_addr: String,
    pub server_name: String,
    pub cert: String,
    pub priv_key: String,
}
