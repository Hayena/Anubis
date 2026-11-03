//! HTTP request parsing for CONNECT and GET requests.

/// Parse a CONNECT request line and return the target host and port.
///
/// Expected form (absolute-form CONNECT or host:port target):
///   CONNECT dev.company.com:443 HTTP/1.1
///   CONNECT https://dev.company.com:443 HTTP/1.1
pub fn parse_connect(req: &str) -> Result<(String, u16), String> {
    let line = req.lines().next().ok_or_else(|| "empty request".to_string())?;
    let parts: Vec<&str> = line.splitn(2, |c: char| c.is_ascii_whitespace()).collect();
    if parts.len() < 2 {
        return Err("malformed request line".to_string());
    }
    let target = parts[1].trim();
    // Skip a leading scheme (CONNECT https://host:port)
    let target = if target.starts_with("https://") {
        &target["https://".len()..]
    } else if target.starts_with("http://") {
        &target["http://".len()..]
    } else {
        target
    };
    let (host, port) = parse_host_port(target)?;
    Ok((host, port))
}

/// Parse a GET request line and return the absolute-form URL's host and port.
fn parse_host_port(target: &str) -> Result<(String, u16), String> {
    let mut parts = target.rsplitn(2, ':');
    let port_str = parts.next().unwrap_or("");
    let host = parts.next().unwrap_or("").to_string();
    if host.is_empty() {
        return Err("empty host".to_string());
    }
    let port: u16 = port_str
        .parse()
        .map_err(|_| "invalid port".to_string())?;
    if port == 0 {
        return Err("port must be 1-65535".to_string());
    }
    Ok((host, port))
}
