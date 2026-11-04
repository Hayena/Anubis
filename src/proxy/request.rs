//! HTTP request parsing for CONNECT requests.

use thiserror::Error;

use crate::shared::error::DestinationError;
use crate::shared::types::Destination;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ParseError {
    #[error("empty request")]
    Empty,

    #[error("malformed request line")]
    MalformedLine,

    #[error("unsupported method: {0}")]
    UnsupportedMethod(String),

    #[error("missing port")]
    MissingPort,

    #[error("invalid port")]
    InvalidPort,

    #[error(transparent)]
    Destination(#[from] DestinationError),
}

/// Parse a CONNECT request line into the destination it targets.
///
/// Expected form (authority-form or with a leading scheme):
///   CONNECT dev.company.com:443 HTTP/1.1
///   CONNECT [::1]:443 HTTP/1.1
///   CONNECT https://dev.company.com:443 HTTP/1.1
pub fn parse_connect(req: &str) -> Result<Destination, ParseError> {
    let line = req.lines().next().ok_or(ParseError::Empty)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or(ParseError::MalformedLine)?;
    if !method.eq_ignore_ascii_case("CONNECT") {
        return Err(ParseError::UnsupportedMethod(method.to_string()));
    }
    let target = parts.next().ok_or(ParseError::MalformedLine)?;
    let target = ["https://", "http://"]
        .iter()
        .find_map(|scheme| target.strip_prefix(scheme))
        .unwrap_or(target);
    parse_host_port(target)
}

/// Split `host:port`, accepting bracketed IPv6 literals.
fn parse_host_port(target: &str) -> Result<Destination, ParseError> {
    let (host, port) = target.rsplit_once(':').ok_or(ParseError::MissingPort)?;
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let port = port.parse().map_err(|_| ParseError::InvalidPort)?;
    Ok(Destination::new(host, port)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dest(host: &str, port: u16) -> Destination {
        Destination::new(host, port).unwrap()
    }

    #[test]
    fn parses_standard_connect_line() {
        let req = "CONNECT dev.company.com:443 HTTP/1.1\r\nHost: dev.company.com:443\r\n\r\n";
        assert_eq!(parse_connect(req), Ok(dest("dev.company.com", 443)));
    }

    #[test]
    fn parses_scheme_prefixed_and_ipv6_targets() {
        assert_eq!(parse_connect("CONNECT https://a.com:8443 HTTP/1.1"), Ok(dest("a.com", 8443)));
        assert_eq!(parse_connect("CONNECT [::1]:443 HTTP/1.1"), Ok(dest("::1", 443)));
    }

    #[test]
    fn rejects_non_connect_and_bad_targets() {
        assert_eq!(
            parse_connect("GET a.com:80 HTTP/1.1"),
            Err(ParseError::UnsupportedMethod("GET".into()))
        );
        assert_eq!(parse_connect("CONNECT a.com HTTP/1.1"), Err(ParseError::MissingPort));
        assert_eq!(parse_connect("CONNECT a.com:x HTTP/1.1"), Err(ParseError::InvalidPort));
        assert_eq!(
            parse_connect("CONNECT a.com:0 HTTP/1.1"),
            Err(DestinationError::InvalidPort.into())
        );
        assert_eq!(
            parse_connect("CONNECT :443 HTTP/1.1"),
            Err(DestinationError::EmptyHost.into())
        );
        assert_eq!(parse_connect(""), Err(ParseError::Empty));
    }
}
