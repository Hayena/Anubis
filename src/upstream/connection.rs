//! Connection trait abstraction.

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

/// A transport connection providing async I/O over the transport.
///
/// Implementations wrap a TCP stream (or similar) and provide async
/// read/write I/O. The async `read`/`write` methods come from the
/// `AsyncRead`/`AsyncWrite` supertraits.
pub trait Connection: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static {
}

impl Connection for TcpStream {}
