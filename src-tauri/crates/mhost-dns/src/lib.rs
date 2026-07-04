pub mod config;
pub mod platform;
pub mod proxy;
pub mod resolver;
pub mod server;

pub use config::DnsConfig;
pub use resolver::RuleEngine;
pub use server::{DnsError, DnsServer};

/// macOS 上 DNS server 监听的非特权端口。53 端口需要 root，
/// 所以 mhost-dns-proxy（以 root 跑）会监听 53 转发到 1053，
/// 真正 mhost 进程里的 `DnsServer` 监听 1053。
#[cfg(target_os = "macos")]
pub const MHOST_DNS_PORT: u16 = 1053;

/// 非 macOS 平台没有端口转发机制，直接监听 53。
#[cfg(not(target_os = "macos"))]
pub const MHOST_DNS_PORT: u16 = 53;
