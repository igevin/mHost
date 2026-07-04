use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use hickory_proto::op::{Header, Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::rdata::A;
use hickory_proto::rr::{Name, RData, Record, RecordType};
use hickory_proto::serialize::binary::{BinDecodable, BinEncodable};
use hickory_resolver::config::{NameServerConfig, Protocol, ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use crate::config::DnsConfig;
use crate::resolver::RuleEngine;

/// DNS 服务错误。
#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("failed to bind DNS socket: {0}")]
    Bind(String),
    #[error("DNS server error: {0}")]
    Server(String),
    #[error("failed to resolve upstream: {0}")]
    Upstream(String),
    #[error("DNS message codec error: {0}")]
    Codec(String),
}

/// DNS 服务核心。
/// TODO: TCP 监听支持计划在后续迭代中添加。
pub struct DnsServer {
    config: DnsConfig,
    rule_engine: Arc<RuleEngine>,
    running: AtomicBool,
    shutdown_tx: Mutex<Option<tokio::sync::mpsc::Sender<()>>>,
    server_handle: Mutex<Option<JoinHandle<Result<(), DnsError>>>>,
    resolver: std::sync::Mutex<TokioAsyncResolver>,
}

impl DnsServer {
    pub fn new(config: DnsConfig) -> Self {
        let resolver = build_resolver(&config.upstream, config.timeout_ms).unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to build upstream resolver: {}, falling back to system config",
                e
            );
            TokioAsyncResolver::tokio_from_system_conf()
                .expect("system resolver config must be valid")
        });
        Self {
            config,
            rule_engine: Arc::new(RuleEngine::new()),
            running: AtomicBool::new(false),
            shutdown_tx: Mutex::new(None),
            server_handle: Mutex::new(None),
            resolver: std::sync::Mutex::new(resolver),
        }
    }

    /// 启动 DNS 服务（异步，在后台运行）。
    /// TODO: 当前仅监听 UDP，TCP 支持计划在后续迭代中添加。
    pub async fn start(&self) -> Result<(), DnsError> {
        if self.is_running() {
            return Ok(());
        }

        let addr = SocketAddr::from(([127, 0, 0, 1], self.config.port));
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|e| DnsError::Bind(format!("{}: {}", addr, e)))?;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        match self.shutdown_tx.lock() {
            Ok(mut guard) => *guard = Some(shutdown_tx),
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = Some(shutdown_tx);
            }
        }

        let rule_engine = self.rule_engine.clone();
        let resolver = {
            let guard = self.resolver.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };

        let handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 512];

            loop {
                tokio::select! {
                    result = socket.recv_from(&mut buf) => {
                        let (len, src) = result
                            .map_err(|e| DnsError::Server(format!("recv failed: {}", e)))?;

                        let request_data = &buf[..len];
                        let response_data = match handle_dns_request(
                            request_data,
                            &rule_engine,
                            &resolver,
                        ).await {
                            Some(data) => data,
                            None => {
                                // 构造 FormErr 响应，保留原始 request id
                                if len < 2 {
                                    continue;
                                }
                                let id = u16::from_be_bytes([buf[0], buf[1]]);
                                let mut header = Header::new();
                                header.set_id(id);
                                header.set_message_type(MessageType::Response);
                                header.set_response_code(ResponseCode::FormErr);
                                let mut response = Message::new();
                                response.set_header(header);
                                match response.to_bytes() {
                                    Ok(data) => data,
                                    Err(e) => {
                                        tracing::warn!("Failed to encode FormErr response: {}", e);
                                        continue;
                                    }
                                }
                            }
                        };

                        if let Err(e) = socket.send_to(&response_data, src).await {
                            tracing::warn!("Failed to send DNS response to {}: {}", src, e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("DNS server received shutdown signal");
                        break;
                    }
                }
            }

            Ok(())
        });

        match self.server_handle.lock() {
            Ok(mut guard) => *guard = Some(handle),
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = Some(handle);
            }
        }

        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// 优雅停止 DNS 服务。
    pub async fn stop(&self) -> Result<(), DnsError> {
        let tx = {
            match self.shutdown_tx.lock() {
                Ok(mut guard) => guard.take(),
                Err(poisoned) => {
                    let mut guard = poisoned.into_inner();
                    guard.take()
                }
            }
        };
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }

        let handle = {
            match self.server_handle.lock() {
                Ok(mut guard) => guard.take(),
                Err(poisoned) => {
                    let mut guard = poisoned.into_inner();
                    guard.take()
                }
            }
        };
        if let Some(handle) = handle {
            match handle.await {
                Ok(result) => result?,
                Err(e) if e.is_cancelled() => (),
                Err(e) => return Err(DnsError::Server(e.to_string())),
            }
        }

        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// 重新加载规则。
    pub fn reload_rules(&self, profiles: &[mhost_core::Profile]) {
        self.rule_engine.rebuild(profiles);
    }

    /// 是否正在运行。
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 返回监听端口。
    pub fn port(&self) -> u16 {
        self.config.port
    }

    /// 返回上游 DNS 服务器列表。
    pub fn upstream(&self) -> &[String] {
        &self.config.upstream
    }

    /// 返回缓存容量（配置值）。
    pub fn cache_capacity(&self) -> usize {
        self.config.cache_size
    }

    /// 返回当前规则数量。
    pub fn rule_count(&self) -> usize {
        self.rule_engine.rule_count()
    }
}

/// 处理单个 DNS 请求，返回编码后的响应数据。
async fn handle_dns_request(
    request_data: &[u8],
    rule_engine: &RuleEngine,
    resolver: &TokioAsyncResolver,
) -> Option<Vec<u8>> {
    let request = match Message::from_bytes(request_data) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::warn!("Failed to decode DNS request: {}", e);
            return None;
        }
    };

    // 只处理标准查询
    if request.message_type() != MessageType::Query {
        return None;
    }
    if request.op_code() != OpCode::Query {
        return None;
    }

    let query = match request.queries().first() {
        Some(q) => q,
        None => return None,
    };

    let name = query.name().to_utf8();
    let name_str = name.trim_end_matches('.');
    let record_type = query.query_type();

    let mut header = Header::response_from_request(request.header());
    header.set_authoritative(false);
    header.set_recursion_available(true);

    let mut response = Message::new();
    response.set_header(header);
    response.set_id(request.id());
    response.add_query(query.clone());

    match record_type {
        RecordType::A => {
            match handle_a_query(name_str, query.name(), rule_engine, resolver).await {
                QueryResult::Answer(record) => {
                    response.add_answer(*record);
                }
                QueryResult::NoError => {
                    // 返回空答案
                }
                QueryResult::ServFail => {
                    let mut h = *response.header();
                    h.set_response_code(ResponseCode::ServFail);
                    response.set_header(h);
                }
            }
        }
        _ => {
            let mut h = *response.header();
            h.set_response_code(ResponseCode::NotImp);
            response.set_header(h);
        }
    }

    match response.to_bytes() {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            tracing::warn!("Failed to encode DNS response: {}", e);
            None
        }
    }
}

enum QueryResult {
    Answer(Box<Record>),
    NoError,
    ServFail,
}

async fn handle_a_query(
    name_str: &str,
    name: &Name,
    rule_engine: &RuleEngine,
    resolver: &TokioAsyncResolver,
) -> QueryResult {
    // 优先匹配本地规则
    if let Some(ip) = rule_engine.resolve(name_str) {
        if let IpAddr::V4(ipv4) = ip {
            let rdata = RData::A(A(ipv4));
            let record = Record::from_rdata(name.clone(), 300, rdata);
            return QueryResult::Answer(Box::new(record));
        }
        return QueryResult::NoError;
    }

    // 未命中，转发上游
    match resolve_upstream(name_str, resolver).await {
        Ok(IpAddr::V4(ipv4)) => {
            let rdata = RData::A(A(ipv4));
            let record = Record::from_rdata(name.clone(), 300, rdata);
            QueryResult::Answer(Box::new(record))
        }
        Ok(IpAddr::V6(_)) => QueryResult::NoError,
        Err(e) => {
            tracing::warn!("Upstream resolution failed for {}: {}", name_str, e);
            QueryResult::ServFail
        }
    }
}

async fn resolve_upstream(domain: &str, resolver: &TokioAsyncResolver) -> Result<IpAddr, DnsError> {
    let response = resolver
        .lookup_ip(domain)
        .await
        .map_err(|e| DnsError::Upstream(e.to_string()))?;

    response
        .iter()
        .next()
        .ok_or_else(|| DnsError::Upstream("no records found".into()))
}

fn build_resolver(upstream: &[String], timeout_ms: u64) -> Result<TokioAsyncResolver, DnsError> {
    if upstream.is_empty() {
        TokioAsyncResolver::tokio_from_system_conf().map_err(|e| DnsError::Upstream(e.to_string()))
    } else {
        let mut config = ResolverConfig::new();
        for server in upstream {
            let socket_addr = match server.parse::<SocketAddr>() {
                Ok(addr) => addr,
                Err(_) => {
                    let ip = server.parse::<IpAddr>().map_err(|e| {
                        DnsError::Upstream(format!("invalid server '{}': {}", server, e))
                    })?;
                    SocketAddr::from((ip, 53))
                }
            };
            config.add_name_server(NameServerConfig::new(socket_addr, Protocol::Udp));
        }
        let mut opts = ResolverOpts::default();
        opts.timeout = std::time::Duration::from_millis(timeout_ms);
        Ok(TokioAsyncResolver::tokio(config, opts))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use mhost_core::{HostRule, Profile, ProfileMode, RuleId};
    use tokio::net::UdpSocket;
    use uuid::Uuid;

    use super::*;

    fn make_profile(name: &str, mode: ProfileMode, enabled: bool, rules: Vec<HostRule>) -> Profile {
        Profile {
            id: mhost_core::ProfileId(Uuid::new_v4()),
            name: name.to_string(),
            description: None,
            enabled,
            protected: false,
            tags: vec![],
            rules,
            mode,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn make_rule(ip: Option<&str>, domains: Vec<&str>, enabled: bool) -> HostRule {
        HostRule {
            id: RuleId(Uuid::new_v4()),
            ip: ip.map(|s| s.parse().unwrap()),
            domains: domains.iter().map(|d| d.to_string()).collect(),
            enabled,
            comment: None,
            source: mhost_core::RuleSource::Manual,
            line_number: None,
        }
    }

    async fn wait_for_server_running(server: &DnsServer, timeout_ms: u64) {
        tokio::time::timeout(Duration::from_millis(timeout_ms), async {
            while !server.is_running() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("server should start within timeout");
    }

    #[tokio::test]
    async fn test_dns_server_start_stop() {
        let config = DnsConfig {
            port: 1053,
            ..Default::default()
        };
        let server = Arc::new(DnsServer::new(config));

        assert!(!server.is_running());

        let server_clone = server.clone();
        let handle = tokio::spawn(async move {
            server_clone.start().await.unwrap();
        });

        wait_for_server_running(&server, 1000).await;
        assert!(server.is_running());

        server.stop().await.unwrap();
        handle.await.unwrap();
        assert!(!server.is_running());
    }

    #[tokio::test]
    async fn test_dns_server_resolve_custom_domain() {
        let profile = make_profile(
            "dns_test",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["test.example.com"], true)],
        );

        let config = DnsConfig {
            port: 1054,
            ..Default::default()
        };
        let server = Arc::new(DnsServer::new(config));
        server.reload_rules(&[profile]);

        let server_clone = server.clone();
        let _handle = tokio::spawn(async move {
            server_clone.start().await.unwrap();
        });

        wait_for_server_running(&server, 1000).await;
        assert!(server.is_running());

        // 构造 DNS 查询
        let query_name = Name::from_utf8("test.example.com.").unwrap();
        let query = hickory_proto::op::Query::query(query_name, RecordType::A);
        let mut request = Message::new();
        request.set_id(1234);
        request.set_recursion_desired(true);
        request.add_query(query);

        let request_bytes = request.to_bytes().unwrap();

        let client = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        client
            .send_to(&request_bytes, "127.0.0.1:1054")
            .await
            .unwrap();

        let mut buf = vec![0u8; 512];
        let (len, _src) = tokio::time::timeout(Duration::from_secs(5), client.recv_from(&mut buf))
            .await
            .unwrap()
            .unwrap();

        let response = Message::from_bytes(&buf[..len]).unwrap();
        assert_eq!(response.id(), 1234);
        assert_eq!(response.response_code(), ResponseCode::NoError);
        assert!(!response.answers().is_empty());

        let answer = &response.answers()[0];
        if let Some(RData::A(a)) = answer.data() {
            assert_eq!(a.0, std::net::Ipv4Addr::new(127, 0, 0, 1));
        } else {
            panic!("Expected A record");
        }

        server.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_dns_server_not_found() {
        // 使用本地不可达端口作为 upstream，短超时确保测试快速失败
        let config = DnsConfig {
            port: 1055,
            upstream: vec!["127.0.0.1:1".to_string()],
            timeout_ms: 500,
            ..Default::default()
        };
        let server = Arc::new(DnsServer::new(config));

        let server_clone = server.clone();
        let _handle = tokio::spawn(async move {
            server_clone.start().await.unwrap();
        });

        wait_for_server_running(&server, 1000).await;

        let query_name = Name::from_utf8("nonexistent.test.").unwrap();
        let query = hickory_proto::op::Query::query(query_name, RecordType::A);
        let mut request = Message::new();
        request.set_id(5678);
        request.set_recursion_desired(true);
        request.add_query(query);

        let request_bytes = request.to_bytes().unwrap();

        let client = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        client
            .send_to(&request_bytes, "127.0.0.1:1055")
            .await
            .unwrap();

        let mut buf = vec![0u8; 512];
        let (len, _src) = tokio::time::timeout(Duration::from_secs(5), client.recv_from(&mut buf))
            .await
            .unwrap()
            .unwrap();

        let response = Message::from_bytes(&buf[..len]).unwrap();
        assert_eq!(response.id(), 5678);
        // 无规则匹配时 upstream 查询失败应返回 ServFail
        assert_eq!(response.response_code(), ResponseCode::ServFail);

        server.stop().await.unwrap();
    }
}
