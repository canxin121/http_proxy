use reqwest;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 代理服务器地址
    let proxy = "http://127.0.0.1:3000";

    // 创建一个客户端，并配置代理
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy)?)
        .danger_accept_invalid_certs(true) // 开发测试时可以接受无效证书
        .build()?;

    // 发送 GET 请求到百度
    let response = client.get("https://www.bilibili.com/").header(
        "User-Agent",
        "I'm a teapot",
    ).send().await?;

    // 检查响应状态
    println!("Status: {}", response.status());

    // 获取响应内容
    let body = response.text().await?;
    println!("Response body:\n{}", body);

    Ok(())
}
