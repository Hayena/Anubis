//! HTTP proxy (CONNECT) upstream connector.

use crate::shared::error::ConnectError;
use crate::shared::types::Destination;
use crate::upstream::connection::Connection;
use crate::upstream::traits::{UpstreamConnector, with_connect_timeout};

use async_trait::async_trait;
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
        let authority = format!("{}:{}", destination.host(), destination.port());
        format!(
            "CONNECT {} HTTP/1.1\r\nHost: {}\r\nConnection: keep-alive\r\n\r\n",
            authority, authority
        )
    }
}

/// Upper bound for the proxy's response head.
const MAX_RESPONSE_HEAD: usize = 8 * 1024;

/// Read the response head one byte at a time so no tunnel data that follows
/// the blank line is consumed.
async fn read_response_head(stream: &mut TcpStream) -> Result<String, ConnectError> {
    let mut head = Vec::with_capacity(256);
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_RESPONSE_HEAD {
            return Err(ConnectError::Handshake("proxy response head too large".into()));
        }
        let byte = stream.read_u8().await.map_err(ConnectError::io("proxy read failed"))?;
        head.push(byte);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// Extract the status code from `HTTP/1.x <code> ...`.
fn status_code(head: &str) -> Option<u16> {
    head.lines().next()?.split_whitespace().nth(1)?.parse().ok()
}

#[async_trait]
impl UpstreamConnector for HttpProxyConnector {
    async fn connect(&self, destination: &Destination) -> Result<Box<dyn Connection>, ConnectError> {
        let stream = with_connect_timeout(async {
            let mut stream = TcpStream::connect((self.host.as_str(), self.port))
                .await
                .map_err(ConnectError::io("proxy connect failed"))?;

            stream
                .write_all(Self::build_connect_request(destination).as_bytes())
                .await
                .map_err(ConnectError::io("proxy write failed"))?;

            let head = read_response_head(&mut stream).await?;
            match status_code(&head) {
                Some(200..=299) => Ok(stream),
                _ => Err(ConnectError::Rejected(
                    head.lines().next().unwrap_or_default().to_string(),
                )),
            }
        })
        .await?;
        Ok(Box::new(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_codes() {
        assert_eq!(status_code("HTTP/1.1 200 Connection established\r\n\r\n"), Some(200));
        assert_eq!(status_code("HTTP/1.0 407 Proxy Auth Required\r\n"), Some(407));
        assert_eq!(status_code("garbage"), None);
    }
}
