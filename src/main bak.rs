#![deny(warnings)]

use std::error::Error;
use std::sync::{Arc, LazyLock};

use bytes::Bytes;
use dingding::config::CONFIG;
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::server::conn::http1::Builder as Http1Builder;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use rustls::ClientConfig;
use rustls_native_certs::load_native_certs;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;
use tracing::{debug, error, info};
use tracing_subscriber;

static CLIENT_CONFIG: LazyLock<Arc<ClientConfig>> = LazyLock::new(|| {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add_parsable_certificates(load_native_certs().unwrap());
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Arc::new(config)
});

// 将 CONNECT 请求中固定部分预计算后存储
static CONNECT_SUFFIX: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{}:{}HTTP/1.1\r\nHost: {}\r\nProxy-Connection: Keep-Alive\r\nUser-Agent: {}\r\nX-T5-Auth: {}\r\n\r\n",
        CONFIG.obfuscation.host,
        CONFIG.obfuscation.port,
        CONFIG.obfuscation.host,
        CONFIG.remote_proxy_address.user_agent,
        CONFIG.remote_proxy_address.x_t5_auth
    )
});

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();
    let addr = CONFIG.local_address;
    let listener = TcpListener::bind(addr).await?;
    info!("Local HTTP Proxy listening on http://{}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        tokio::spawn(async move {
            if let Err(e) = Http1Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(io, service_fn(handle_request))
                .with_upgrades()
                .await
            {
                error!("Serve connection error: {:?}", e);
            }
        });
    }
}

// 处理客户端请求，先与远程代理服务器 CONNECT，再转发请求并返回结果
async fn handle_request(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    info!("Received request: {:?}", req);

    // 提取 host、port
    let host = match req.uri().host() {
        Some(h) => h.to_string(),
        None => {
            let mut bad = Response::new(full("No valid host in URI"));
            *bad.status_mut() = StatusCode::BAD_REQUEST;
            return Ok(bad);
        }
    };
    let port = req.uri().port_u16().unwrap_or_else(|| {
        if req.uri().scheme_str() == Some("https") {
            443
        } else {
            80
        }
    });

    let mut proxy_stream = match TcpStream::connect(&CONFIG.remote_proxy_address.address).await {
        Ok(s) => s,
        Err(e) => {
            error!("Connect to remote proxy error: {}", e);
            let mut bad = Response::new(full("Cannot connect to remote proxy"));
            *bad.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(bad);
        }
    };

    // 修改: 动态拼接 CONNECT 请求
    let connect_req = format!("CONNECT {}:{}@{}", host, port, *CONNECT_SUFFIX);

    debug!("CONNECT request: {}", connect_req);

    if let Err(e) = proxy_stream.write_all(connect_req.as_bytes()).await {
        error!("Write CONNECT request to remote proxy error: {}", e);
        let mut bad = Response::new(full("Failed to send CONNECT to remote proxy"));
        *bad.status_mut() = StatusCode::BAD_GATEWAY;
        return Ok(bad);
    }

    // 读取远程代理的响应，检查是否成功建立隧道
    let mut response_buf = [0u8; 128];
    let n = match proxy_stream.read(&mut response_buf).await {
        Ok(n) => n,
        Err(e) => {
            error!("Read CONNECT response error: {}", e);
            let mut bad = Response::new(full("No response from remote proxy"));
            *bad.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(bad);
        }
    };

    let resp_str = String::from_utf8_lossy(&response_buf[..n]);
    if !resp_str.contains("200") {
        // error!("Remote proxy CONNECT failed: {}", resp_str);
        let mut bad = Response::new(full("Remote proxy refused our CONNECT request"));
        *bad.status_mut() = StatusCode::BAD_GATEWAY;
        return Ok(bad);
    }

    // 如果客户端是 CONNECT 方法，则直接升级到隧道：复制客户端与代理之间的数据
    // 否则，进行 TLS 握手并转发请求
    if req.method() == Method::CONNECT {
        tokio::task::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    let mut upgraded = TokioIo::new(upgraded);
                    let _ = tokio::io::copy_bidirectional(&mut upgraded, &mut proxy_stream).await;
                }
                Err(e) => error!("upgrade error: {:?}", e),
            }
        });
        return Ok(Response::new(empty()));
    } else {
        error!("Not a CONNECT request, forwarding request to remote server");
        let rc_config = CLIENT_CONFIG.clone();
        let server_name = match ServerName::try_from(host.to_owned()) {
            Ok(s) => s,
            Err(_) => {
                let mut bad = Response::new(full("Invalid ServerName"));
                *bad.status_mut() = StatusCode::BAD_REQUEST;
                return Ok(bad);
            }
        };

        let connector: TlsConnector = TlsConnector::from(rc_config);
        let mut tls_stream: tokio_rustls::client::TlsStream<TcpStream> =
            match connector.connect(server_name, proxy_stream).await {
                Ok(s) => s,
                Err(e) => {
                    error!("TLS handshake error: {}", e);
                    let mut bad = Response::new(full("TLS handshake with remote server failed"));
                    *bad.status_mut() = StatusCode::BAD_GATEWAY;
                    return Ok(bad);
                }
            };

        let (parts, body) = req.into_parts();
        let mut req_bytes = format!(
            "{} {} HTTP/1.1\r\n",
            parts.method,
            parts
                .uri
                .path_and_query()
                .map(|x| x.as_str())
                .unwrap_or("/")
        )
        .into_bytes();

        for (name, value) in parts.headers.iter() {
            req_bytes
                .extend_from_slice(format!("{}: {}\r\n", name, value.to_str().unwrap()).as_bytes());
        }
        req_bytes.extend_from_slice(b"\r\n");

        let body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                error!("Failed to read request body: {}", e);
                let mut bad = Response::new(full("Failed to read request body"));
                *bad.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                return Ok(bad);
            }
        };
        req_bytes.extend_from_slice(&body_bytes);

        if let Err(e) = tls_stream.write_all(&req_bytes).await {
            error!("Forward request error: {}", e);
            let mut bad = Response::new(full("Failed to forward request"));
            *bad.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(bad);
        }

        let mut buffer = Vec::new();
        if let Err(e) = tls_stream.read_to_end(&mut buffer).await {
            error!("Read remote response error: {}", e);
            let mut bad = Response::new(full("Fail to read response from upstream"));
            *bad.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(bad);
        }

        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(
                Full::new(Bytes::from(buffer))
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .unwrap();
        return Ok(resp);
    }
}

fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}
