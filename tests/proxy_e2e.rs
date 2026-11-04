//! End-to-end tests: real sockets through the proxy listener.

use std::net::SocketAddr;
use std::sync::Arc;

use anubis::config::holder::ConfigHolder;
use anubis::config::loader::reload;
use anubis::config::models::{Api, Config, Listen, RuleTarget};
use anubis::health::store::{HealthStore, InMemoryHealthStore};
use anubis::metrics::collector::Metrics;
use anubis::proxy::listener::Proxy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Echo server that prefixes every chunk with `prefix`.
async fn spawn_echo(prefix: &'static str) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 256];
                while let Ok(n) = s.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let _ = s.write_all(&[prefix.as_bytes(), &buf[..n]].concat()).await;
                }
            });
        }
    });
    addr
}

/// Minimal HTTP CONNECT proxy: replies 200, then echoes with `proxy:` prefix.
/// Sends the CONNECT request line it saw over `seen`.
async fn spawn_http_proxy(seen: tokio::sync::mpsc::UnboundedSender<String>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    match s.read_u8().await {
                        Ok(b) => head.push(b),
                        Err(_) => return,
                    }
                }
                let line = String::from_utf8_lossy(&head).lines().next().unwrap().to_string();
                let _ = seen.send(line);
                let _ = s.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await;
                let mut buf = [0u8; 256];
                while let Ok(n) = s.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let _ = s.write_all(&[b"proxy:", &buf[..n]].concat()).await;
                }
            });
        }
    });
    addr
}

/// Minimal no-auth SOCKS5 server: accepts CONNECT, reports the requested
/// `host:port` over `seen`, then echoes with a `socks:` prefix.
async fn spawn_socks5_proxy(seen: tokio::sync::mpsc::UnboundedSender<String>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let _ = socks5_session(&mut s, seen).await;
            });
        }
    });
    addr
}

async fn socks5_session(
    s: &mut TcpStream,
    seen: tokio::sync::mpsc::UnboundedSender<String>,
) -> std::io::Result<()> {
    let mut greeting = [0u8; 2];
    s.read_exact(&mut greeting).await?;
    let mut methods = vec![0u8; greeting[1] as usize];
    s.read_exact(&mut methods).await?;
    s.write_all(&[5, 0]).await?;

    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    let host = match head[3] {
        1 => {
            let mut ip = [0u8; 4];
            s.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let len = s.read_u8().await? as usize;
            let mut name = vec![0u8; len];
            s.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        _ => return Ok(()),
    };
    let port = s.read_u16().await?;
    let _ = seen.send(format!("{host}:{port}"));
    s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;

    let mut buf = [0u8; 256];
    loop {
        let n = s.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        s.write_all(&[b"socks:", &buf[..n]].concat()).await?;
    }
}

async fn start_proxy(holder: Arc<ConfigHolder>, metrics: Arc<Metrics>) -> SocketAddr {
    start_proxy_with_health(holder, metrics, Arc::new(InMemoryHealthStore::new())).await
}

async fn start_proxy_with_health(
    holder: Arc<ConfigHolder>,
    metrics: Arc<Metrics>,
    health: Arc<dyn HealthStore>,
) -> SocketAddr {
    let listener = Proxy::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let proxy = Proxy::new(holder, metrics, health);
    tokio::spawn(async move { proxy.serve(listener).await });
    addr
}

fn config(rules: Vec<anubis::Rule>, default: RuleTarget) -> Config {
    Config {
        listen: Listen::default(),
        api: Api::default(),
        proxies: vec![],
        rules,
        default,
    }
}

/// CONNECT through the proxy; returns the status line and the reply to `payload`.
async fn connect_and_send(proxy: SocketAddr, target: SocketAddr, payload: &[u8]) -> (String, Vec<u8>) {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut status = Vec::new();
    while !status.ends_with(b"\r\n\r\n") {
        status.push(s.read_u8().await.unwrap());
    }
    let status = String::from_utf8(status).unwrap().lines().next().unwrap().to_string();
    if !status.contains("200") {
        return (status, vec![]);
    }
    s.write_all(payload).await.unwrap();
    let mut buf = vec![0u8; payload.len() + 16];
    let n = s.read(&mut buf).await.unwrap();
    buf.truncate(n);
    (status, buf)
}

#[tokio::test]
async fn direct_tunnel_relays_both_directions() {
    let echo = spawn_echo("direct:").await;
    let metrics = Arc::new(Metrics::new());
    let holder = Arc::new(ConfigHolder::new(config(vec![], RuleTarget::Direct)));
    let proxy = start_proxy(holder, Arc::clone(&metrics)).await;

    let (status, reply) = connect_and_send(proxy, echo, b"hello").await;

    assert!(status.contains("200"), "{status}");
    assert_eq!(reply, b"direct:hello");
    assert_eq!(metrics.snapshot().connections_opened, 1);
}

#[tokio::test]
async fn unknown_upstream_returns_502_and_counts_failure() {
    let echo = spawn_echo("direct:").await;
    let metrics = Arc::new(Metrics::new());
    let holder = Arc::new(ConfigHolder::new(config(vec![], RuleTarget::Proxy("missing".into()))));
    let proxy = start_proxy(holder, Arc::clone(&metrics)).await;

    let (status, _) = connect_and_send(proxy, echo, b"x").await;

    assert!(status.contains("502"), "{status}");
    assert_eq!(metrics.snapshot().connections_failed, 1);
}

#[tokio::test]
async fn http_upstream_receives_connect_for_the_destination() {
    let echo = spawn_echo("direct:").await;
    let (tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let upstream = spawn_http_proxy(tx).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.conf");
    std::fs::write(&path, conf_via_proxy(upstream)).unwrap();
    let holder = Arc::new(ConfigHolder::new(reload(path.to_str().unwrap()).unwrap()));
    let metrics = Arc::new(Metrics::new());
    let proxy = start_proxy(holder, Arc::clone(&metrics)).await;

    let (status, reply) = connect_and_send(proxy, echo, b"hi").await;

    assert!(status.contains("200"), "{status}");
    assert_eq!(reply, b"proxy:hi");
    assert_eq!(seen.recv().await.unwrap(), format!("CONNECT {echo} HTTP/1.1"));
    assert_eq!(metrics.snapshot().proxy_selections.get("up"), Some(&1));
}

/// Holder whose default route is a SOCKS5 upstream named `up` at `upstream`.
fn socks_holder(upstream: SocketAddr) -> Arc<ConfigHolder> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.conf");
    std::fs::write(
        &path,
        format!(
            "proxies = [ {{ name = \"up\", type = \"socks5\", host = \"127.0.0.1\", port = {} }} ]\n\
             default = \"proxy:up\"\n",
            upstream.port()
        ),
    )
    .unwrap();
    Arc::new(ConfigHolder::new(reload(path.to_str().unwrap()).unwrap()))
}

#[tokio::test]
async fn socks5_upstream_tunnels_to_the_destination() {
    let echo = spawn_echo("direct:").await;
    let (tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let upstream = spawn_socks5_proxy(tx).await;
    let proxy = start_proxy(socks_holder(upstream), Arc::new(Metrics::new())).await;

    let (status, reply) = connect_and_send(proxy, echo, b"yo").await;

    assert!(status.contains("200"), "{status}");
    assert_eq!(reply, b"socks:yo");
    assert_eq!(seen.recv().await.unwrap(), echo.to_string());
}

#[tokio::test]
async fn unhealthy_upstream_fails_fast_and_recovers() {
    let echo = spawn_echo("direct:").await;
    let (tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let upstream = spawn_socks5_proxy(tx).await;
    let health = Arc::new(InMemoryHealthStore::new());
    let metrics = Arc::new(Metrics::new());
    let proxy = start_proxy_with_health(
        socks_holder(upstream),
        Arc::clone(&metrics),
        Arc::clone(&health) as Arc<dyn HealthStore>,
    )
    .await;

    health.set_healthy("up", false, None);
    let (status, _) = connect_and_send(proxy, echo, b"yo").await;
    assert!(status.contains("502"), "{status}");
    assert!(seen.try_recv().is_err(), "must not contact a down upstream");
    assert_eq!(metrics.snapshot().connections_failed, 1);

    health.set_healthy("up", true, Some(1.0));
    let (status, reply) = connect_and_send(proxy, echo, b"yo").await;
    assert!(status.contains("200"), "{status}");
    assert_eq!(reply, b"socks:yo");
}

#[tokio::test]
async fn reload_reroutes_new_connections_without_restart() {
    let echo = spawn_echo("direct:").await;
    let (tx, _seen) = tokio::sync::mpsc::unbounded_channel();
    let upstream = spawn_http_proxy(tx).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.conf");
    let path_str = path.to_str().unwrap();
    std::fs::write(&path, "default = \"direct\"\n").unwrap();
    let holder = Arc::new(ConfigHolder::new(reload(path_str).unwrap()));
    let proxy = start_proxy(Arc::clone(&holder), Arc::new(Metrics::new())).await;

    let (_, before) = connect_and_send(proxy, echo, b"a").await;
    std::fs::write(&path, conf_via_proxy(upstream)).unwrap();
    holder.reload(path_str).unwrap();
    let (_, after) = connect_and_send(proxy, echo, b"a").await;

    assert_eq!(before, b"direct:a");
    assert_eq!(after, b"proxy:a");
}

fn conf_via_proxy(upstream: SocketAddr) -> String {
    format!(
        "proxies = [ {{ name = \"up\", type = \"http\", host = \"127.0.0.1\", port = {} }} ]\n\
         default = \"proxy:up\"\n",
        upstream.port()
    )
}
