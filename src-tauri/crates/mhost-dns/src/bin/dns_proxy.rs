//! mhost-dns-proxy binary 入口。
//! 用法: mhost-dns-proxy [--listen PORT] [--target PORT]
//!
//! 以 root 权限运行，监听特权端口 53，
//! 将 UDP DNS 请求转发到本地非特权端口 1053 上的 mhost DNS server。

#[tokio::main]
async fn main() {
    mhost_dns::proxy::run_proxy().await;
}
