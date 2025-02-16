#![deny(warnings)]

use std::error::Error;
use std::sync::LazyLock;

use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use http_proxy::config::CONFIG;
use hyper::server::conn::http1::Builder as Http1Builder;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

static CONNECT_SUFFIX: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{}:{}HTTP/1.1\r\nHost: {}\r\n\r\n",
        CONFIG.obfuscation.host, CONFIG.obfuscation.port, CONFIG.obfuscation.host,
    )
});

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    loop {
        let addr = CONFIG.local_address;
        let listener = TcpListener::bind(addr).await?;
        println!("Local HTTP Proxy listening on http://{}", addr);

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
                    eprintln!("Serve connection error: {:?}", e);
                }
            });
        }
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    // println!("Received request: {:?}", req);

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

    let mut proxy_stream = match TcpStream::connect((
        CONFIG.remote_proxy_address.host.clone(),
        CONFIG.remote_proxy_address.port,
    ))
    .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Connect to remote proxy error: {}", e);
            let mut bad = Response::new(full("Cannot connect to remote proxy"));
            *bad.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(bad);
        }
    };

    let connect_req = format!("CONNECT {}:{}@{}", host, port, *CONNECT_SUFFIX);

    if let Err(e) = proxy_stream.write_all(connect_req.as_bytes()).await {
        eprintln!("Write CONNECT request to remote proxy error: {}", e);
        let mut bad = Response::new(full("Failed to send CONNECT to remote proxy"));
        *bad.status_mut() = StatusCode::BAD_GATEWAY;
        return Ok(bad);
    }

    let mut response_buf = [0u8; 128];
    let n = match proxy_stream.read(&mut response_buf).await {
        Ok(n) => n,
        Err(e) => {
            eprintln!("Read CONNECT response error: {}", e);
            let mut bad = Response::new(full("No response from remote proxy"));
            *bad.status_mut() = StatusCode::BAD_GATEWAY;
            return Ok(bad);
        }
    };

    let resp_str = String::from_utf8_lossy(&response_buf[..n]);
    if !resp_str.contains("200") {
        let mut bad = Response::new(full(
            "Remote proxy cannot connect to target server or refused our CONNECT request",
        ));
        *bad.status_mut() = StatusCode::BAD_GATEWAY;
        return Ok(bad);
    }

    if req.method() == Method::CONNECT {
        tokio::task::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    let mut upgraded = TokioIo::new(upgraded);
                    let _ = tokio::io::copy_bidirectional(&mut upgraded, &mut proxy_stream).await;
                }
                Err(e) => eprintln!("upgrade error: {:?}", e),
            }
        });
        return Ok(Response::new(empty()));
    } else {
        return Ok(Response::new(full("Only CONNECT method is supported")));
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
