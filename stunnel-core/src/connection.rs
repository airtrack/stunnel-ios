use crate::config::AppConfig;
use std::io;
use std::net::SocketAddr;
use tokio::sync::{Mutex as TokioMutex, OnceCell};
use tracing::info;

pub trait ConnectionState {
    fn in_good_condition(&mut self) -> io::Result<()>;
}

impl ConnectionState for s2n_quic::connection::Handle {
    fn in_good_condition(&mut self) -> io::Result<()> {
        self.keep_alive(true)
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionReset, e))
    }
}

impl ConnectionState for stunnel::tlstcp::client::Connector {
    fn in_good_condition(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Manages the lifecycle of outbound connections.
pub struct ConnectionManager {
    s2n_client: OnceCell<s2n_quic::Client>,
    s2n_handle: TokioMutex<Option<s2n_quic::connection::Handle>>,
    tlstcp_connector: OnceCell<stunnel::tlstcp::client::Connector>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            s2n_client: OnceCell::new(),
            s2n_handle: TokioMutex::new(None),
            tlstcp_connector: OnceCell::new(),
        }
    }

    pub async fn get_s2n_handle(
        &self,
        config: &AppConfig,
    ) -> io::Result<s2n_quic::connection::Handle> {
        let mut lock = self.s2n_handle.lock().await;

        if let Some(mut handle) = lock.clone() {
            if handle.in_good_condition().is_ok() {
                return Ok(handle);
            }
            info!("QUIC connection broken, reconnecting...");
        }

        let client = self
            .s2n_client
            .get_or_try_init(|| async {
                let quic_config = stunnel::quic::Config {
                    addr: "0.0.0.0:0".to_string(),
                    cert: config.cert.clone(),
                    priv_key: config.priv_key.clone(),
                    loss_threshold: 10,
                };
                stunnel::quic::s2n_quic::client::new(&quic_config)
            })
            .await?;

        let addr: SocketAddr = config.server_addr.parse().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid addr: {:?}", e),
            )
        })?;
        let connect =
            s2n_quic::client::Connect::new(addr).with_server_name(config.server_name.as_str());

        let mut conn = client.connect(connect).await.map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("QUIC connect error: {:?}", e))
        })?;
        conn.keep_alive(true).map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("QUIC keep-alive error: {:?}", e),
            )
        })?;

        let handle = conn.handle();
        *lock = Some(handle.clone());
        Ok(handle)
    }

    pub async fn get_tlstcp_connector(
        &self,
        config: &AppConfig,
    ) -> io::Result<stunnel::tlstcp::client::Connector> {
        self.tlstcp_connector
            .get_or_try_init(|| async {
                let tls_config = stunnel::tlstcp::client::Config {
                    server_addr: config.server_addr.clone(),
                    server_name: config.server_name.clone(),
                    cert: config.cert.clone(),
                    priv_key: config.priv_key.clone(),
                };
                Ok(stunnel::tlstcp::client::new(&tls_config))
            })
            .await
            .cloned()
    }
}
