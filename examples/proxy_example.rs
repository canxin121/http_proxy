use rustls::pki_types::ServerName;
use rustls::ClientConfig;
use rustls_native_certs::load_native_certs;
use std::error::Error;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 代理服务器配置
    let proxy_host = "110.242.70.68";
    let proxy_port = 443;

    // 目标网站
    let target_host = "www.bilibili.com";
    let target_port = 443;

    // 连接到代理服务器
    let mut proxy_stream = TcpStream::connect((proxy_host, proxy_port)).await?;

    // 发送 CONNECT 请求
    let connect_req = format!(
        "CONNECT {}:{}@gw.alicdn.com:80HTTP/1.1\r\nHost: gw.alicdn.com:80\r\nConnection: keep-alive\r\n\r\n",
        target_host, target_port);

    proxy_stream.write_all(connect_req.as_bytes()).await?;

    // 读取代理服务器响应
    let mut response = [0u8; 1024];
    let n = proxy_stream.read(&mut response).await?;
    let response = String::from_utf8_lossy(&response[..n]);

    // 检查是否成功建立隧道
    if !response.contains("200") {
        return Err(format!("Failed to establish tunnel: {}", response).into());
    }

    // 配置 TLS
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add_parsable_certificates(load_native_certs().unwrap());
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let rc_config = Arc::new(config);
    let server_name = ServerName::try_from(target_host)?;

    // 创建 TLS 连接
    let connector = tokio_rustls::TlsConnector::from(rc_config);
    let mut tls_stream = connector.connect(server_name, proxy_stream).await?;

    // 发送 HTTPS GET 请求
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        target_host
    );

    tls_stream.write_all(request.as_bytes()).await?;

    // 读取响应
    let mut response = Vec::new();
    tls_stream.read_to_end(&mut response).await?;

    println!("Response: {}", String::from_utf8_lossy(&response));

    Ok(())
}
