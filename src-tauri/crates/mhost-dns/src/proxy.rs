//! DNS UDP 端口转发代理。
//!
//! 以 root 权限运行，监听特权端口（如 53），
//! 将收到的 UDP DNS 请求转发到本地非特权端口（如 1053）上的 DNS server。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::net::UdpSocket;

/// DNS proxy 错误。
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("bind failed on {addr}: {reason}")]
    BindFailed { addr: SocketAddr, reason: String },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// UDP 转发代理。
/// 监听 `listen_addr`（特权端口），转发到 `target_addr`（非特权端口）。
pub struct DnsProxy {
    listen_addr: SocketAddr,
    target_addr: SocketAddr,
}

impl DnsProxy {
    pub fn new(listen_port: u16, target_port: u16) -> Self {
        Self {
            listen_addr: ([127, 0, 0, 1], listen_port).into(),
            target_addr: ([127, 0, 0, 1], target_port).into(),
        }
    }

    /// 运行代理（阻塞），直到收到关闭信号或出错。
    pub async fn run(&self) -> Result<(), ProxyError> {
        // 绑定特权端口（需要 root）
        let listen_socket = UdpSocket::bind(self.listen_addr).await.map_err(|e| {
            ProxyError::BindFailed {
                addr: self.listen_addr,
                reason: e.to_string(),
            }
        })?;

        let running = Arc::new(AtomicBool::new(true));

        // 注册 SIGTERM / SIGINT 信号处理
        let sig_running = running.clone();
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
                _ = ctrl_c => {}
                _ = sigterm => {}
            }
            sig_running.store(false, Ordering::SeqCst);
        });

        eprintln!(
            "[mhost-dns-proxy] listening on {} -> {}",
            self.listen_addr, self.target_addr
        );

        let mut buf = vec![0u8; 512];

        while running.load(Ordering::SeqCst) {
            // 设置 1 秒超时，避免信号到达时卡在 recv_from
            match tokio::time::timeout(
                std::time::Duration::from_secs(1),
                listen_socket.recv_from(&mut buf),
            ).await {
                Ok(Ok((len, src))) => {
                    // 转发到 DNS server
                    if let Err(e) = listen_socket.send_to(&buf[..len], self.target_addr).await {
                        if running.load(Ordering::SeqCst) {
                            eprintln!("[mhost-dns-proxy] forward error: {}", e);
                        }
                        continue;
                    }
                    // 等待 DNS server 响应并回传给原始客户端
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        listen_socket.recv_from(&mut buf),
                    ).await {
                        Ok(Ok((resp_len, _server))) => {
                            if let Err(e) = listen_socket.send_to(&buf[..resp_len], src).await {
                                if running.load(Ordering::SeqCst) {
                                    eprintln!("[mhost-dns-proxy] reply error: {}", e);
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            if running.load(Ordering::SeqCst) {
                                eprintln!("[mhost-dns-proxy] recv from server error: {}", e);
                            }
                        }
                        Err(_) => {
                            eprintln!("[mhost-dns-proxy] response timeout");
                        }
                    }
                }
                Ok(Err(e)) => {
                    if running.load(Ordering::SeqCst) {
                        eprintln!("[mhost-dns-proxy] recv error: {}", e);
                    }
                }
                Err(_) => {
                    // timeout, check running flag and loop
                }
            }
        }

        eprintln!("[mhost-dns-proxy] shutting down");
        Ok(())
    }
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
    if let Err(e) = proxy.run().await {
        eprintln!("[mhost-dns-proxy] error: {}", e);
        std::process::exit(1);
    }
}
