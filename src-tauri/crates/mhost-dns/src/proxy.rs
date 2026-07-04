//! DNS UDP 端口转发代理。
//!
//! 以 root 权限运行，监听特权端口（如 53），
//! 将收到的 UDP DNS 请求转发到本地非特权端口（如 1053）上的 DNS server。
//!
//! ## 并发模型（fix #76）
//!
//! 主循环只接收客户端查询并 spawn task；每个 task 用**临时绑定的 UdpSocket + connect()**
//! 到 `target_addr` 做 upstream 往返。靠 4-tuple（src_ip, src_port, dst_ip, dst_port）
//! 让响应归属唯一，避免不同 client 的响应交叉。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Notify;
use tracing::{debug, warn};

/// DNS proxy 错误。
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("bind failed on {addr}: {reason}")]
    BindFailed { addr: SocketAddr, reason: String },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("upstream timeout after {0:?}")]
    UpstreamTimeout(Duration),
}

/// UDP 转发代理。
/// 监听 `listen_addr`（特权端口），转发到 `target_addr`（非特权端口）。
pub struct DnsProxy {
    listen_addr: SocketAddr,
    target_addr: SocketAddr,
    /// 关闭信号。`run()` 收到 notify 后立即退出主循环（已 spawn 的 task 自然结束）。
    shutdown: Arc<Notify>,
}

impl DnsProxy {
    pub fn new(listen_port: u16, target_port: u16) -> Self {
        let shutdown = Arc::new(Notify::new());
        Self {
            listen_addr: ([127, 0, 0, 1], listen_port).into(),
            target_addr: ([127, 0, 0, 1], target_port).into(),
            shutdown,
        }
    }

    /// 拿到 shutdown 句柄，外部可在 signal handler 中调用 `notify_one()`。
    pub fn shutdown_handle(&self) -> Arc<Notify> {
        Arc::clone(&self.shutdown)
    }

    /// 运行代理（阻塞），直到收到 shutdown 信号或主 socket 不可恢复错误。
    pub async fn run(&self) -> Result<(), ProxyError> {
        // 绑定特权端口（需要 root）
        let listen_socket =
            UdpSocket::bind(self.listen_addr)
                .await
                .map_err(|e| ProxyError::BindFailed {
                    addr: self.listen_addr,
                    reason: e.to_string(),
                })?;
        // UdpSocket 内部是 Arc，clone 便宜，spawn 时把 Arc 形式的引用传给 task
        let listen_socket = Arc::new(listen_socket);

        eprintln!(
            "[mhost-dns-proxy] listening on {} -> {}",
            self.listen_addr, self.target_addr
        );

        // 主循环：接收客户端查询 → spawn task 处理
        // 缓冲区 4096 字节支持 EDNS(0) 协商后的最大响应（fix #80 也在改 server buf，这里先跟上）
        let mut buf = vec![0u8; 4096];
        let shutdown = Arc::clone(&self.shutdown);
        loop {
            tokio::select! {
                biased;
                _ = shutdown.notified() => {
                    eprintln!("[mhost-dns-proxy] shutdown signal received");
                    break;
                }
                result = listen_socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, src)) => {
                            let query = buf[..len].to_vec();
                            let listen = Arc::clone(&listen_socket);
                            let target = self.target_addr;
                            tokio::spawn(async move {
                                if let Err(e) =
                                    handle_client_query(&listen, query, src, target).await
                                {
                                    warn!("[mhost-dns-proxy] client {:?} error: {}", src, e);
                                }
                            });
                        }
                        Err(e) => {
                            warn!("[mhost-dns-proxy] recv_from error: {}", e);
                            // 短暂退避避免 busy loop
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        }

        eprintln!("[mhost-dns-proxy] shutting down");
        Ok(())
    }
}

/// 处理单个客户端查询：用临时 socket 做 upstream 往返，再回包给客户端。
async fn handle_client_query(
    listen: &UdpSocket,
    query: Vec<u8>,
    client: SocketAddr,
    target: SocketAddr,
) -> Result<(), ProxyError> {
    // 1. 临时 socket + connect 到 upstream，让响应只回这个 socket
    let upstream = UdpSocket::bind("0.0.0.0:0").await?;
    upstream.connect(target).await?;

    // 2. 转发 query
    upstream.send(&query).await?;

    // 3. 等响应（5s 超时）
    let mut resp_buf = vec![0u8; 4096];
    let resp_len = tokio::time::timeout(Duration::from_secs(5), upstream.recv(&mut resp_buf))
        .await
        .map_err(|_| ProxyError::UpstreamTimeout(Duration::from_secs(5)))??;

    debug!(
        "[mhost-dns-proxy] {} -> {} ({} bytes), reply to client",
        client, target, resp_len
    );

    // 4. 回包给原客户端（用 listen_socket；这里有内部 Arc，跨 task 安全）
    listen.send_to(&resp_buf[..resp_len], client).await?;
    Ok(())
}

/// dns-proxy 独立 binary 入口点。
/// 用法: mhost-dns-proxy [--listen PORT] [--target PORT]
pub async fn run_proxy() {
    let args: Vec<String> = std::env::args().collect();

    let mut listen_port: u16 = 53;
    let mut target_port: u16 = 1053;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" => {
                i += 1;
                if i < args.len() {
                    listen_port = args[i].parse().unwrap_or_else(|_| {
                        eprintln!("Invalid listen port, using default 53");
                        53
                    });
                }
            }
            "--target" => {
                i += 1;
                if i < args.len() {
                    target_port = args[i].parse().unwrap_or_else(|_| {
                        eprintln!("Invalid target port, using default 1053");
                        1053
                    });
                }
            }
            _ => {}
        }
        i += 1;
    }

    let proxy = DnsProxy::new(listen_port, target_port);
    let shutdown = proxy.shutdown_handle();

    // 注册 SIGTERM / SIGINT 信号处理
    tokio::spawn(async move {
        let ctrl_c = async {
            tokio::signal::ctrl_c().await.ok();
        };
        let sigterm = async {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                let mut sigterm = signal(SignalKind::terminate()).ok();
                if let Some(ref mut st) = sigterm {
                    st.recv().await;
                }
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            _ = ctrl_c => eprintln!("[mhost-dns-proxy] received SIGINT"),
            _ = sigterm => eprintln!("[mhost-dns-proxy] received SIGTERM"),
        }
        shutdown.notify_waiters();
    });

    if let Err(e) = proxy.run().await {
        eprintln!("[mhost-dns-proxy] error: {}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    //! 回归测试 fix #76：proxy 不能把 client A 的响应回给 client B。
    //!
    //! 测试方法：开一个本地 UDP "upstream" 模拟 DNS server，对不同 query
    //! 返回不同 response；然后用真 proxy 监听 → 转发 → 回包。两个 client 并发
    //! 发不同 query，断言每个 client 收到的是自己 query 对应的 response。

    use std::net::SocketAddr;
    use std::time::Duration;

    use tokio::net::UdpSocket;
    use tokio::sync::Notify;

    use super::*;

    /// 启动一个 mock upstream：每个 query 收到后回一段固定 response。
    /// query -> response 映射由 `responses` 提供。
    async fn start_mock_upstream(responses: Vec<(Vec<u8>, Vec<u8>)>) -> SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            while let Ok((len, src)) = socket.recv_from(&mut buf).await {
                let query = &buf[..len];
                // 找匹配的 response
                for (q, r) in &responses {
                    if q.as_slice() == query {
                        let _ = socket.send_to(r, src).await;
                        break;
                    }
                }
            }
        });
        addr
    }

    #[tokio::test]
    async fn test_proxy_concurrent_clients() {
        // 关键测试：两个 client 并发，proxy 不能把 response 交叉
        let query_a = b"QUERY_A".to_vec();
        let response_a = b"RESPONSE_A_AAAAA".to_vec();
        let query_b = b"QUERY_B".to_vec();
        let response_b = b"RESPONSE_B_BBBBB".to_vec();
        let upstream_addr = start_mock_upstream(vec![
            (query_a.clone(), response_a.clone()),
            (query_b.clone(), response_b.clone()),
        ])
        .await;

        // 启动 proxy 在指定端口（先拿一个空闲端口）
        let listen_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let listen_port = listen_socket.local_addr().unwrap().port();
        drop(listen_socket);
        let proxy = DnsProxy {
            listen_addr: SocketAddr::from(([127, 0, 0, 1], listen_port)),
            target_addr: upstream_addr,
            shutdown: Arc::new(Notify::new()),
        };
        let shutdown = proxy.shutdown_handle();
        let proxy_handle = tokio::spawn(async move { proxy.run().await });

        // 两个 client 并发查询
        let port_a = listen_port;
        let port_b = listen_port;
        let client_a = tokio::spawn(async move {
            let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            sock.send_to(&query_a, SocketAddr::from(([127, 0, 0, 1], port_a)))
                .await
                .unwrap();
            let mut buf = vec![0u8; 4096];
            let (len, _) = tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
                .await
                .unwrap()
                .unwrap();
            buf[..len].to_vec()
        });
        let client_b = tokio::spawn(async move {
            let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            sock.send_to(&query_b, SocketAddr::from(([127, 0, 0, 1], port_b)))
                .await
                .unwrap();
            let mut buf = vec![0u8; 4096];
            let (len, _) = tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
                .await
                .unwrap()
                .unwrap();
            buf[..len].to_vec()
        });

        let resp_a = client_a.await.unwrap();
        let resp_b = client_b.await.unwrap();

        // 关键断言：每个 client 收到自己 query 对应的 response（fix #76 回归）
        assert_eq!(
            resp_a, response_a,
            "client A 应收到 RESPONSE_A，不应收到 RESPONSE_B"
        );
        assert_eq!(
            resp_b, response_b,
            "client B 应收到 RESPONSE_B，不应收到 RESPONSE_A"
        );

        // 收尾
        shutdown.notify_waiters();
        let _ = proxy_handle.await;
    }

    #[tokio::test]
    async fn test_proxy_shutdown() {
        // 启动 proxy 后立刻 shutdown，验证 run() 在 1s 内返回
        let listen_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let listen_port = listen_socket.local_addr().unwrap().port();
        drop(listen_socket);
        let proxy = DnsProxy {
            listen_addr: SocketAddr::from(([127, 0, 0, 1], listen_port)),
            target_addr: SocketAddr::from(([127, 0, 0, 1], 1053)),
            shutdown: Arc::new(Notify::new()),
        };
        let _shutdown = proxy.shutdown_handle();
        let proxy_handle = tokio::spawn(async move { proxy.run().await });

        // 给 proxy 50ms 启动
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 触发 shutdown
        let start = std::time::Instant::now();
        _shutdown.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(1), proxy_handle)
            .await
            .expect("proxy 应在 1s 内退出")
            .expect("proxy task 不应 panic");
        let elapsed = start.elapsed();
        assert!(result.is_ok(), "proxy.run() 应返回 Ok，实际 {:?}", result);
        assert!(
            elapsed < Duration::from_secs(1),
            "shutdown 应 < 1s，实际 {:?}",
            elapsed
        );
    }
}
