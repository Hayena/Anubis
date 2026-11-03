//! HTTP proxy (CONNECT) upstream connector.

use crate::shared::types::Destination;
use crate::upstream::connection::Connection;
use crate::upstream::traits::UpstreamConnector;

use async_trait::async_trait;
use tokio::net::TcpStream;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

/// HTTP proxy connector: establishes a CONNECT tunnel through an HTTP(S) proxy.
///
/// On a successful handshake the returned `Connection` is a TCP stream to the
/// ultimate destination (the CONNECT tunnel), not to the proxy itself.
pub struct HttpProxyConnector {
    pub host: String,
    pub port: u16,
}

impl HttpProxyConnector {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
        }
    }

    fn build_connect_request(destination: &Destination) -> String {
        let authority = format!("{}:{}", destination.host, destination.port);
        format!(
            "CONNECT {} HTTP/1.1\r\nHost: {}\r\nConnection: keep-alive\r\n\r\n",
            authority, authority
        )
    }
}

#[async_trait]
impl UpstreamConnector for HttpProxyConnector {
    async fn connect(&self, destination: &Destination) -> Result<Box<dyn Connection>, String> {
        // 1. Establish TCP connection to the HTTP proxy.
        let addr = format!("{}:{}", self.host, self.port);
        let mut stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| format!("proxy connect failed: {e}"))?;

        // 2. Send the CONNECT request for the ultimate destination.
        let request = Self::build_connect_request(destination);
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| format!("proxy write failed: {e}"))?;

        // 3. Read the proxy response; expect 200 OK.
        let mut response = [0u8; 4096];
        let n = stream
            .read(&mut response)
            .await
            .map_err(|e| format!("proxy read failed: {e}"))?;
        let response = String::from_utf8_lossy(&response[..n]);
        if !response.starts_with("HTTP/1.1 200") && !response.starts_with("HTTP/1.0 200") {
            return Err(format!("proxy CONNECT failed: {response}"));
        }

        // 4. Return the tunneled stream as the connection to the destination.
        Ok(Box::new(stream))
    }
}
