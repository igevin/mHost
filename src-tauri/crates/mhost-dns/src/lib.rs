pub mod adblock;
pub mod config;
pub mod matcher;
pub mod platform;
pub mod proxy;
pub mod resolver;
pub mod server;

pub use adblock::{AdBlockAction, AdBlockEngine};
pub use config::DnsConfig;
pub use platform::UpstreamTier;
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

/// **fix (issue #148 review 🟡 #2)**：跨 crate 测试串行化锁。
///
/// mhost-dns 平台侧 test 与 mhost crate 里的 `cleanup_dns_on_exit` test
/// 都会改 `MHOST_RUNTIME_DIR` —— 它们在 cargo test 下是独立二进制,
/// `proxy::tests::TEST_LOCK` 是 `pub(crate)`,跨 crate 看不到。
///
/// 这里暴露一个公开锁,让两边的测试都引用同一把 mutex,避免 env var
/// race(proxy pid_file 在一边被写,另一边同时改 runtime_dir 读到错的路径)。
///
/// **`#[cfg(test)]` 不行**:mhost crate 编译 test 时把 mhost_dns 作为依赖,
/// mhost_dns 自身的 `cfg(test)` 不被启用,这个 static 看不到。所以 always-on
/// + `#[doc(hidden)]` 阻止出现在公共 API 文档里;零运行时成本(Mutex
/// lazy-init)。
#[doc(hidden)]
pub static RUNTIME_DIR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
