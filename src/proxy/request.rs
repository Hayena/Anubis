//! Parsing of proxy requests: `CONNECT` tunnels and plain-HTTP forwarding.

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

    #[error("unsupported target {0:?}: expected host:port or an absolute http:// URL")]
    UnsupportedTarget(String),

    #[error("missing port")]
    MissingPort,

    #[error("invalid port")]
    InvalidPort,

    #[error(transparent)]
    Destination(#[from] DestinationError),
}

/// A parsed client request.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// `CONNECT host:port`: open a tunnel, reply `200`, then relay bytes.
    Connect(Destination),
    /// Absolute-form `GET http://host/path`: send `head` to the origin.
    Forward(Forward),
}

/// A plain-HTTP request rewritten for the origin server.
#[derive(Debug, PartialEq, Eq)]
pub struct Forward {
    pub dest: Destination,
    /// Origin-form request head, ending with the blank line.
    pub head: String,
}

impl Request {
    pub fn destination(&self) -> &Destination {
        match self {
            Self::Connect(dest) => dest,
            Self::Forward(forward) => &forward.dest,
        }
    }
}

/// Headers that apply to the client<->proxy hop only and must not be relayed.
const HOP_HEADERS: [&str; 4] = ["proxy-connection", "proxy-authorization", "connection", "keep-alive"];

/// Parse the request head sent to the proxy.
///
/// Accepts `CONNECT host:port` and absolute-form `METHOD http://host[:port]/path`.
/// Forwarded requests are rewritten to origin-form with `Connection: close`,
/// so each client connection carries exactly one request and routing is
/// decided per destination.
pub fn parse_request(head: &str) -> Result<Request, ParseError> {
    let line = head.lines().next().ok_or(ParseError::Empty)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or(ParseError::MalformedLine)?;
    if method.eq_ignore_ascii_case("CONNECT") {
        return parse_connect(head).map(Request::Connect);
    }
    let target = parts.next().ok_or(ParseError::MalformedLine)?;
    let version = parts.next().ok_or(ParseError::MalformedLine)?;
    let rest = strip_prefix_ignore_case(target, "http://")
        .ok_or_else(|| ParseError::UnsupportedTarget(target.to_string()))?;

    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let (host, port) = split_authority(authority);
    let port = port.map_or(Ok(80), |p| p.parse().map_err(|_| ParseError::InvalidPort))?;
    let dest = Destination::new(host, port)?;

    let path = match tail {
        t if t.starts_with('/') => t.to_string(),
        t if t.starts_with('?') => format!("/{t}"),
        _ => "/".to_string(),
    };
    let headers: Vec<&str> = head
        .lines()
        .skip(1)
        .take_while(|l| !l.is_empty())
        .filter(|l| !header_name(l).is_some_and(is_hop_header))
        .collect();
    let has_host = headers
        .iter()
        .any(|l| header_name(l).is_some_and(|n| n.eq_ignore_ascii_case("host")));
    let host_header = if has_host { String::new() } else { format!("Host: {authority}\r\n") };

    let head = format!(
        "{method} {path} {version}\r\n{}{host_header}Connection: close\r\n\r\n",
        headers.iter().map(|l| format!("{l}\r\n")).collect::<String>(),
    );
    Ok(Request::Forward(Forward { dest, head }))
}

fn header_name(line: &str) -> Option<&str> {
    line.split_once(':').map(|(name, _)| name.trim())
}

fn is_hop_header(name: &str) -> bool {
    HOP_HEADERS.iter().any(|h| name.eq_ignore_ascii_case(h))
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    s.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &s[prefix.len()..])
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

/// Split an authority into host and optional port, accepting bracketed IPv6.
fn split_authority(authority: &str) -> (&str, Option<&str>) {
    if let Some((host, after)) = authority.strip_prefix('[').and_then(|a| a.split_once(']')) {
        return (host, after.strip_prefix(':'));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    }
}

fn parse_host_port(target: &str) -> Result<Destination, ParseError> {
    let (host, port) = split_authority(target);
    let port = port.ok_or(ParseError::MissingPort)?;
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

    fn forward(req: &str) -> Forward {
        match parse_request(req).unwrap() {
            Request::Forward(f) => f,
            other => panic!("expected forward, got {other:?}"),
        }
    }

    #[test]
    fn forwards_absolute_form_as_origin_form() {
        let f = forward(
            "GET http://example.com/a/b?x=1 HTTP/1.1\r\nHost: example.com\r\n\
             Proxy-Connection: keep-alive\r\nConnection: keep-alive\r\nAccept: */*\r\n\r\n",
        );
        assert_eq!(f.dest, dest("example.com", 80));
        assert_eq!(
            f.head,
            "GET /a/b?x=1 HTTP/1.1\r\nHost: example.com\r\nAccept: */*\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn forward_handles_ports_paths_userinfo_and_missing_host_header() {
        let f = forward("POST http://u:p@[::1]:8080?q=1 HTTP/1.1\r\n\r\n");
        assert_eq!(f.dest, dest("::1", 8080));
        assert_eq!(
            f.head,
            "POST /?q=1 HTTP/1.1\r\nHost: [::1]:8080\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(forward("GET HTTP://a.com HTTP/1.1\r\n\r\n").head.lines().next(), Some("GET / HTTP/1.1"));
    }

    #[test]
    fn connect_is_still_a_tunnel_and_origin_form_is_rejected() {
        assert_eq!(
            parse_request("CONNECT a.com:443 HTTP/1.1\r\n\r\n"),
            Ok(Request::Connect(dest("a.com", 443)))
        );
        assert_eq!(
            parse_request("GET /health HTTP/1.1\r\n\r\n"),
            Err(ParseError::UnsupportedTarget("/health".into()))
        );
        assert_eq!(parse_request("GET http://a.com:x/ HTTP/1.1"), Err(ParseError::InvalidPort));
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
