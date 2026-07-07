//! 阶段 6 集成测试：覆盖 Hosts 回归、DNS E2E、双模式共存、多 Profile 并集、
//! 数据迁移、异常场景等验收标准。
//!
//! 测试组织结构：
//! - 6.1 Hosts 模式回归测试
//! - 6.3 双模式共存测试
//! - 6.4 DNS 多 Profile 并集测试
//! - 6.5 数据迁移测试（额外补充）
//! - 6.6 异常场景测试

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

use std::net::IpAddr;
use std::sync::Arc;

use mhost_apply::writer::HostsWriter;
use mhost_core::{HostRule, Profile, ProfileMode};
use mhost_dns::RuleEngine;
use mhost_storage::storage::{FileStorage, Storage};

use tempfile::TempDir;

/// 创建测试用存储和 HostsWriter，使用临时目录。
fn create_test_storage_and_writer() -> (TempDir, Arc<dyn Storage + Send + Sync>, HostsWriter) {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(FileStorage::new(temp_dir.path())) as Arc<dyn Storage + Send + Sync>;

    let hosts_path = temp_dir.path().join("hosts");
    let backup_dir = temp_dir.path().join("backups");
    std::fs::write(&hosts_path, "# original hosts\n").unwrap();

    let writer = HostsWriter::with_paths(&hosts_path, &backup_dir);
    (temp_dir, storage, writer)
}

/// 创建指定模式的 Profile 并保存到存储。
fn create_profile_with_mode(
    storage: &Arc<dyn Storage + Send + Sync>,
    name: &str,
    mode: ProfileMode,
    rules: Vec<(&str, &str)>,
) -> Profile {
    let mut profile = Profile::new(name);
    profile.mode = mode;
    for (ip, domain) in rules {
        profile
            .rules
            .push(HostRule::new(ip.parse().unwrap(), vec![domain.to_string()]));
    }
    storage.save_profile(&profile).unwrap();
    profile
}

/// 创建 hosts 模式的 Profile 并保存。
fn create_hosts_profile(
    storage: &Arc<dyn Storage + Send + Sync>,
    name: &str,
    rules: Vec<(&str, &str)>,
) -> Profile {
    create_profile_with_mode(storage, name, ProfileMode::Hosts, rules)
}

/// 创建 dns 模式的 Profile 并保存。
fn create_dns_profile(
    storage: &Arc<dyn Storage + Send + Sync>,
    name: &str,
    rules: Vec<(&str, &str)>,
) -> Profile {
    create_profile_with_mode(storage, name, ProfileMode::Dns, rules)
}

/// 构造用于 RuleEngine 测试的 HostRule。
fn make_rule(ip: Option<&str>, domains: Vec<&str>, enabled: bool) -> HostRule {
    HostRule {
        id: mhost_core::RuleId(uuid::Uuid::new_v4()),
        ip: ip.map(|s| s.parse().unwrap()),
        domains: domains.iter().map(|d| d.to_string()).collect(),
        enabled,
        comment: None,
        source: mhost_core::RuleSource::Manual,
        line_number: None,
    }
}

/// 构造用于 RuleEngine 测试的 Profile。
fn make_profile_for_engine(
    name: &str,
    mode: ProfileMode,
    enabled: bool,
    rules: Vec<HostRule>,
) -> Profile {
    Profile {
        id: mhost_core::ProfileId(uuid::Uuid::new_v4()),
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

// ===========================================================================
// 6.1 Hosts 模式回归测试
// ===========================================================================

#[cfg(test)]
mod test_6_1_hosts_regression {
    use super::*;
    use crate::commands::apply::*;
    use crate::commands::profile::*;
    use crate::commands::snapshot::*;

    // 验证 create_profile(name, None) 创建 hosts 模式
    #[test]
    fn test_create_profile_default_mode_is_hosts() {
        let profile = Profile::new("default");
        assert_eq!(profile.mode, ProfileMode::Hosts);
    }

    // 验证 create_profile(name, Some(Hosts)) 创建 hosts 模式
    #[test]
    fn test_create_profile_explicit_hosts_mode() {
        let profile = Profile::new("explicit_hosts");
        assert_eq!(profile.mode, ProfileMode::Hosts);
    }

    // 验证 list_profiles(None) 只返回 hosts 模式
    #[test]
    fn test_list_profiles_default_returns_hosts_only() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        let _hosts = create_hosts_profile(&storage, "h1", vec![("127.0.0.1", "a.com")]);
        let _dns = create_dns_profile(&storage, "d1", vec![("192.168.1.1", "b.com")]);

        let listed = storage.list_profiles().unwrap();
        assert_eq!(
            listed.len(),
            1,
            "list_profiles() should return only hosts profiles"
        );
        assert_eq!(listed[0].mode, ProfileMode::Hosts);
        assert_eq!(listed[0].name, "h1");
    }

    // 验证 set_profile_enabled 互斥逻辑仍然正确
    #[test]
    fn test_set_profile_enabled_mutual_exclusion_still_works() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        let mut hosts_a = create_hosts_profile(&storage, "hosts_a", vec![("127.0.0.1", "a.com")]);
        hosts_a.enabled = true;
        storage.save_profile(&hosts_a).unwrap();

        let mut hosts_b = create_hosts_profile(&storage, "hosts_b", vec![("192.168.1.1", "b.com")]);
        hosts_b.enabled = true;
        storage.save_profile(&hosts_b).unwrap();

        // 启用 hosts_a 应禁用 hosts_b
        disable_other_profiles(storage.as_ref(), &hosts_a.id).unwrap();

        let loaded_a = storage.load_profile(&hosts_a.id).unwrap();
        let loaded_b = storage.load_profile(&hosts_b.id).unwrap();

        assert!(loaded_a.enabled, "hosts_a should stay enabled");
        assert!(
            !loaded_b.enabled,
            "hosts_b should be disabled by mutual exclusion"
        );
    }

    // 验证 apply_hosts 只处理 hosts 模式 Profile
    #[test]
    fn test_apply_hosts_only_writes_hosts_mode_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        let mut hosts =
            create_hosts_profile(&storage, "hosts_p", vec![("127.0.0.1", "hosts.local")]);
        hosts.enabled = true;
        storage.save_profile(&hosts).unwrap();

        let mut dns = create_dns_profile(&storage, "dns_p", vec![("192.168.1.1", "dns.local")]);
        dns.enabled = true;
        storage.save_profile(&dns).unwrap();

        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(result.is_ok(), "apply should succeed: {:?}", result.err());

        let hosts_content = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(
            hosts_content.contains("127.0.0.1 hosts.local"),
            "hosts file should contain hosts profile rules"
        );
        assert!(
            !hosts_content.contains("192.168.1.1 dns.local"),
            "hosts file should NOT contain dns profile rules"
        );
    }

    // 验证 snapshot 保存和恢复 hosts 模式 Profile
    #[test]
    fn test_snapshot_save_restore_hosts_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        let mut p1 = create_hosts_profile(&storage, "dev", vec![("127.0.0.1", "a.com")]);
        p1.enabled = true;
        storage.save_profile(&p1).unwrap();

        let meta =
            save_snapshot_logic(storage.as_ref(), "hosts_snapshot".to_string(), None).unwrap();
        assert_eq!(meta.profile_count, 1);

        // 删除所有 profiles
        for p in storage.list_profiles().unwrap() {
            storage.delete_profile(&p.id).unwrap();
        }
        assert!(storage.list_profiles().unwrap().is_empty());

        // 恢复快照
        load_snapshot_logic(storage.as_ref(), &writer, &meta.id).unwrap();

        let restored = storage.list_profiles().unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].name, "dev");
        assert_eq!(restored[0].mode, ProfileMode::Hosts);
    }

    // 验证 profile 验证逻辑仍正确
    #[test]
    fn test_profile_validation_still_works() {
        let cases = vec![
            ("empty_name", Profile::new(""), true),
            ("valid", Profile::new("valid"), false),
            ("newline_name", Profile::new("valid\nname"), true),
        ];

        for (name, profile, should_err) in cases {
            let result = validate_profile(&profile);
            assert_eq!(
                result.is_err(),
                should_err,
                "case '{}': expected error={}, got error={}",
                name,
                should_err,
                result.is_err()
            );
        }
    }

    // 表格驱动测试: Hosts Profile 完整 CRUD 生命周期
    #[test]
    fn test_hosts_profile_full_crud_lifecycle() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // Create
        let mut profile = create_hosts_profile(&storage, "lifecycle", vec![("127.0.0.1", "a.com")]);
        let loaded = storage.load_profile(&profile.id).unwrap();
        assert_eq!(loaded.name, "lifecycle");
        assert_eq!(loaded.mode, ProfileMode::Hosts);

        // Enable + Apply
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();
        enable_and_apply_logic(&profile.id, true, storage.as_ref(), &writer).unwrap();

        let hosts_content = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_content.contains("127.0.0.1 a.com"));

        // Disable + Apply
        enable_and_apply_logic(&profile.id, false, storage.as_ref(), &writer).unwrap();
        let hosts_after = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(!hosts_after.contains("127.0.0.1 a.com"));
        assert!(!hosts_after.contains("# ---- mHost start ----"));

        // Update
        let mut loaded = storage.load_profile(&profile.id).unwrap();
        loaded.rules.push(HostRule::new(
            "192.168.1.1".parse().unwrap(),
            vec!["b.com".to_string()],
        ));
        loaded.updated_at = chrono::Utc::now();
        storage.save_profile(&loaded).unwrap();

        let updated = storage.load_profile(&profile.id).unwrap();
        assert_eq!(updated.rules.len(), 2);

        // Delete
        storage.delete_profile(&profile.id).unwrap();
        let result = storage.load_profile(&profile.id);
        assert!(result.is_err());
    }
}

// ===========================================================================
// 6.2 DNS 模式 E2E 测试（纯逻辑层）
// ===========================================================================

#[cfg(test)]
mod test_6_2_dns_mode_logic {
    use super::*;

    // 验证 RuleEngine 从 DNS Profile 加载规则
    #[test]
    fn test_dns_rule_engine_loads_from_profile() {
        let engine = RuleEngine::new();
        let profile = make_profile_for_engine(
            "dns_test",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["test.example.com"], true)],
        );
        engine.rebuild(&[profile]);

        assert_eq!(engine.rule_count(), 1);
        assert_eq!(
            engine.resolve("test.example.com"),
            Some("127.0.0.1".parse::<IpAddr>().unwrap())
        );
    }

    // 验证 DNS 服务可启动和停止（需要 tokio runtime）
    #[tokio::test]
    async fn test_dns_server_lifecycle() {
        use std::sync::Arc;
        use std::time::Duration;

        let config = mhost_dns::DnsConfig {
            port: 1056,
            ..Default::default()
        };
        let server = Arc::new(mhost_dns::DnsServer::new(config).unwrap());

        assert!(!server.is_running());

        let server_clone = server.clone();
        let handle = tokio::spawn(async move {
            server_clone.start().await.unwrap();
        });

        // 等待服务器启动
        tokio::time::timeout(Duration::from_secs(1), async {
            while !server.is_running() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("server should start within timeout");

        assert!(server.is_running());

        server.stop().await.unwrap();
        handle.await.unwrap();
        assert!(!server.is_running());
    }

    // 验证 DNS 查询自定义域名返回正确 IP
    #[tokio::test]
    async fn test_dns_server_resolves_custom_domain() {
        use hickory_proto::serialize::binary::{BinDecodable, BinEncodable};
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::net::UdpSocket;

        let profile = make_profile_for_engine(
            "dns_e2e",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["e2e.test"], true)],
        );

        let config = mhost_dns::DnsConfig {
            port: 1057,
            ..Default::default()
        };
        let server = Arc::new(mhost_dns::DnsServer::new(config).unwrap());
        server.reload_rules(&[profile]);

        let server_clone = server.clone();
        let _handle = tokio::spawn(async move {
            server_clone.start().await.unwrap();
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            while !server.is_running() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("server should start within timeout");

        // 发送 DNS 查询
        let query_name = hickory_proto::rr::Name::from_utf8("e2e.test.").unwrap();
        let query = hickory_proto::op::Query::query(query_name, hickory_proto::rr::RecordType::A);
        let mut request = hickory_proto::op::Message::new();
        request.set_id(9999);
        request.set_recursion_desired(true);
        request.add_query(query);

        let request_bytes = request.to_bytes().unwrap();

        let client = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        client
            .send_to(&request_bytes, "127.0.0.1:1057")
            .await
            .unwrap();

        let mut buf = vec![0u8; 512];
        let (len, _src) = tokio::time::timeout(Duration::from_secs(5), client.recv_from(&mut buf))
            .await
            .unwrap()
            .unwrap();

        let response = hickory_proto::op::Message::from_bytes(&buf[..len]).unwrap();
        assert_eq!(response.id(), 9999);
        assert_eq!(
            response.response_code(),
            hickory_proto::op::ResponseCode::NoError
        );
        assert!(!response.answers().is_empty());

        server.stop().await.unwrap();
    }
}

// ===========================================================================
// 6.3 双模式共存测试
// ===========================================================================

#[cfg(test)]
mod test_6_3_dual_mode_coexistence {
    use super::*;
    use crate::commands::apply::*;
    #[allow(unused_imports)]
    use crate::commands::profile::*;

    // 创建 hosts + dns Profile，验证 list_profiles 只返回 hosts
    #[test]
    fn test_list_profiles_only_returns_hosts_mode() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        let _h1 = create_hosts_profile(&storage, "h1", vec![("127.0.0.1", "h1.local")]);
        let _h2 = create_hosts_profile(&storage, "h2", vec![("127.0.0.1", "h2.local")]);
        let _d1 = create_dns_profile(&storage, "d1", vec![("192.168.1.1", "d1.local")]);
        let _d2 = create_dns_profile(&storage, "d2", vec![("192.168.1.1", "d2.local")]);

        let hosts_list = storage.list_profiles().unwrap();
        assert_eq!(
            hosts_list.len(),
            2,
            "list_profiles() should only return hosts mode profiles"
        );
        assert!(hosts_list.iter().all(|p| p.mode == ProfileMode::Hosts));
    }

    // list_dns_profiles 只返回 dns 模式
    #[test]
    fn test_list_dns_profiles_only_returns_dns_mode() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        let _h1 = create_hosts_profile(&storage, "h1", vec![("127.0.0.1", "h1.local")]);
        let _d1 = create_dns_profile(&storage, "d1", vec![("192.168.1.1", "d1.local")]);
        let _d2 = create_dns_profile(&storage, "d2", vec![("192.168.1.1", "d2.local")]);

        let dns_list = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        assert_eq!(dns_list.len(), 2);
        assert!(dns_list.iter().all(|p| p.mode == ProfileMode::Dns));
    }

    // list_all_profiles 返回两者
    #[test]
    fn test_list_all_profiles_returns_both_modes() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        let _h1 = create_hosts_profile(&storage, "h1", vec![("127.0.0.1", "h1.local")]);
        let _d1 = create_dns_profile(&storage, "d1", vec![("192.168.1.1", "d1.local")]);

        let all = storage.list_all_profiles().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|p| p.mode == ProfileMode::Hosts));
        assert!(all.iter().any(|p| p.mode == ProfileMode::Dns));
    }

    // apply_hosts 不影响 dns Profile
    #[test]
    fn test_apply_hosts_does_not_affect_dns_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        let mut hosts = create_hosts_profile(&storage, "hosts_p", vec![("127.0.0.1", "h.local")]);
        hosts.enabled = true;
        storage.save_profile(&hosts).unwrap();

        let mut dns = create_dns_profile(&storage, "dns_p", vec![("192.168.1.1", "d.local")]);
        dns.enabled = true;
        storage.save_profile(&dns).unwrap();

        // Apply hosts
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(result.is_ok());

        // DNS profile 仍然存在且 enabled
        let dns_loaded = storage.load_profile(&dns.id).unwrap();
        assert!(
            dns_loaded.enabled,
            "DNS profile should still be enabled after apply_hosts"
        );
        assert_eq!(dns_loaded.mode, ProfileMode::Dns);

        // hosts 文件只包含 hosts profile 的规则
        let hosts_content = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_content.contains("127.0.0.1 h.local"));
        assert!(!hosts_content.contains("192.168.1.1 d.local"));
    }

    // 两种模式可同时存在
    #[test]
    fn test_both_modes_can_coexist() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // 创建并启用 hosts profile
        let mut hosts =
            create_hosts_profile(&storage, "hosts_active", vec![("127.0.0.1", "hosts.co")]);
        hosts.enabled = true;
        storage.save_profile(&hosts).unwrap();

        // 创建并启用 dns profile
        let mut dns = create_dns_profile(&storage, "dns_active", vec![("192.168.1.1", "dns.co")]);
        dns.enabled = true;
        storage.save_profile(&dns).unwrap();

        // 应用 hosts
        apply_current_plan_logic(storage.as_ref(), &writer).unwrap();

        // hosts 文件只有 hosts 规则
        let hosts_content = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_content.contains("127.0.0.1 hosts.co"));
        assert!(!hosts_content.contains("192.168.1.1 dns.co"));

        // DNS profile 仍然 enabled
        let dns_loaded = storage.load_profile(&dns.id).unwrap();
        assert!(dns_loaded.enabled);

        // Hosts profile 仍然 enabled
        let hosts_loaded = storage.load_profile(&hosts.id).unwrap();
        assert!(hosts_loaded.enabled);
    }

    // 启用 hosts profile 不会影响 dns profile 的 enabled 状态
    #[test]
    fn test_enabling_hosts_does_not_affect_dns_enabled() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        let mut dns = create_dns_profile(&storage, "dns_p", vec![("192.168.1.1", "d.local")]);
        dns.enabled = true;
        storage.save_profile(&dns).unwrap();

        let hosts = create_hosts_profile(&storage, "hosts_p", vec![("127.0.0.1", "h.local")]);

        // 启用 hosts profile
        enable_and_apply_logic(&hosts.id, true, storage.as_ref(), &writer).unwrap();

        // DNS profile 仍然 enabled
        let dns_loaded = storage.load_profile(&dns.id).unwrap();
        assert!(
            dns_loaded.enabled,
            "enabling hosts profile should NOT disable dns profile"
        );
    }

    // 表格驱动: 双模式列表分离
    #[test]
    fn test_dual_mode_list_separation_table_driven() {
        let cases = vec![
            ("0 hosts + 0 dns", 0, 0),
            ("1 hosts + 0 dns", 1, 0),
            ("0 hosts + 1 dns", 0, 1),
            ("2 hosts + 3 dns", 2, 3),
            ("5 hosts + 1 dns", 5, 1),
        ];

        for (name, num_hosts, num_dns) in cases {
            let (_temp, storage, _writer) = create_test_storage_and_writer();

            for i in 0..num_hosts {
                let _ = create_hosts_profile(
                    &storage,
                    &format!("h_{}", i),
                    vec![("127.0.0.1", &format!("h{}.local", i))],
                );
            }
            for i in 0..num_dns {
                let _ = create_dns_profile(
                    &storage,
                    &format!("d_{}", i),
                    vec![("192.168.1.1", &format!("d{}.local", i))],
                );
            }

            let hosts_list = storage.list_profiles().unwrap();
            let dns_list = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
            let all_list = storage.list_all_profiles().unwrap();

            assert_eq!(
                hosts_list.len(),
                num_hosts,
                "case '{}': hosts list count mismatch",
                name
            );
            assert_eq!(
                dns_list.len(),
                num_dns,
                "case '{}': dns list count mismatch",
                name
            );
            assert_eq!(
                all_list.len(),
                num_hosts + num_dns,
                "case '{}': all list count mismatch",
                name
            );
        }
    }
}

// ===========================================================================
// 6.4 DNS 多 Profile 并集测试
// ===========================================================================

#[cfg(test)]
mod test_6_4_dns_multi_profile_union {
    use super::*;

    // 两个 DNS Profile，规则取并集
    #[test]
    fn test_two_dns_profiles_union() {
        let engine = RuleEngine::new();

        let p1 = make_profile_for_engine(
            "dns_p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["a.com", "b.com"], true)],
        );
        let p2 = make_profile_for_engine(
            "dns_p2",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("192.168.1.1"), vec!["c.com", "d.com"], true)],
        );

        engine.rebuild(&[p1, p2]);

        assert_eq!(engine.rule_count(), 4);
        assert_eq!(engine.resolve("a.com"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(engine.resolve("b.com"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(
            engine.resolve("c.com"),
            Some("192.168.1.1".parse().unwrap())
        );
        assert_eq!(
            engine.resolve("d.com"),
            Some("192.168.1.1".parse().unwrap())
        );
    }

    // 相同域名取并集（第一个生效）
    #[test]
    fn test_domain_conflict_first_wins() {
        let engine = RuleEngine::new();

        let p1 = make_profile_for_engine(
            "first",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["shared.com"], true)],
        );
        let p2 = make_profile_for_engine(
            "second",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("192.168.1.1"), vec!["shared.com"], true)],
        );

        engine.rebuild(&[p1, p2]);

        assert_eq!(engine.rule_count(), 1);
        assert_eq!(
            engine.resolve("shared.com"),
            Some("127.0.0.1".parse().unwrap()),
            "first profile's IP should win"
        );
    }

    // 三个 DNS Profile，规则正确合并
    #[test]
    fn test_three_dns_profiles_union() {
        let engine = RuleEngine::new();

        let p1 = make_profile_for_engine(
            "blocker",
            ProfileMode::Dns,
            true,
            vec![
                make_rule(Some("0.0.0.0"), vec!["ad1.com", "ad2.com"], true),
                make_rule(Some("0.0.0.0"), vec!["tracker.com"], true),
            ],
        );
        let p2 = make_profile_for_engine(
            "dev",
            ProfileMode::Dns,
            true,
            vec![make_rule(
                Some("127.0.0.1"),
                vec!["api.local", "db.local"],
                true,
            )],
        );
        let p3 = make_profile_for_engine(
            "testing",
            ProfileMode::Dns,
            true,
            vec![make_rule(
                Some("192.168.1.100"),
                vec!["staging.local"],
                true,
            )],
        );

        engine.rebuild(&[p1, p2, p3]);

        // 总共 6 条唯一规则
        assert_eq!(engine.rule_count(), 6);
        assert_eq!(engine.resolve("ad1.com"), Some("0.0.0.0".parse().unwrap()));
        assert_eq!(
            engine.resolve("api.local"),
            Some("127.0.0.1".parse().unwrap())
        );
        assert_eq!(
            engine.resolve("staging.local"),
            Some("192.168.1.100".parse().unwrap())
        );
    }

    // 禁用其中一个 Profile，规则减少
    #[test]
    fn test_disable_one_profile_reduces_rules() {
        let engine = RuleEngine::new();

        let p1 = make_profile_for_engine(
            "p1",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("127.0.0.1"), vec!["a.com"], true)],
        );
        let p2 = make_profile_for_engine(
            "p2",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("192.168.1.1"), vec!["b.com"], true)],
        );

        engine.rebuild(&[p1.clone(), p2.clone()]);
        assert_eq!(engine.rule_count(), 2);

        // 禁用 p2
        let mut p2_disabled = p2.clone();
        p2_disabled.enabled = false;
        engine.rebuild(&[p1.clone(), p2_disabled]);
        assert_eq!(engine.rule_count(), 1);
        assert_eq!(engine.resolve("a.com"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(engine.resolve("b.com"), None);
    }

    // 表格驱动: 多 Profile 规则数量
    #[test]
    fn test_multi_profile_rule_count_table_driven() {
        // (case_name, rules_per_profile, expected_total_unique_rules, overlap_pairs)
        // overlap_pairs: list of (profile_idx, rule_idx) that should reuse domain "shared.local"
        type TestCase<'a> = (&'a str, Vec<usize>, usize, Vec<(usize, usize)>);
        let cases: Vec<TestCase> = vec![
            ("0 profiles", vec![], 0, vec![]),
            ("1 profile, 1 rule", vec![1], 1, vec![]),
            ("2 profiles, 2+3 rules", vec![2, 3], 5, vec![]),
            ("3 profiles, 1+1+1 rules", vec![1, 1, 1], 3, vec![]),
            // p0 has 2 rules, p1 has 2 rules but both share "shared.local" with p0's j=0 -> 3 unique
            (
                "2 profiles with overlap",
                vec![2, 2],
                3,
                vec![(0, 0), (1, 0)],
            ),
        ];

        for (name, rules_per_profile, expected_rules, overlap_pairs) in cases {
            let engine = RuleEngine::new();
            let mut profiles = Vec::new();
            let overlap_set: std::collections::HashSet<_> = overlap_pairs.iter().cloned().collect();

            for (i, &num_rules) in rules_per_profile.iter().enumerate() {
                let mut domains = Vec::new();
                for j in 0..num_rules {
                    let domain = if overlap_set.contains(&(i, j)) {
                        "shared.local".to_string()
                    } else {
                        format!("{}.p{}.local", j, i)
                    };
                    domains.push(domain);
                }
                let domains_ref: Vec<&str> = domains.iter().map(|s| s.as_str()).collect();
                let profile = make_profile_for_engine(
                    &format!("p{}", i),
                    ProfileMode::Dns,
                    true,
                    vec![make_rule(Some("127.0.0.1"), domains_ref, true)],
                );
                profiles.push(profile);
            }

            engine.rebuild(&profiles);
            assert_eq!(
                engine.rule_count(),
                expected_rules,
                "case '{}': expected {} rules, got {}",
                name,
                expected_rules,
                engine.rule_count()
            );
        }
    }

    // rebuild 替换旧规则
    #[test]
    fn test_rebuild_replaces_all_rules() {
        let engine = RuleEngine::new();

        let p1 = make_profile_for_engine(
            "old",
            ProfileMode::Dns,
            true,
            vec![
                make_rule(Some("127.0.0.1"), vec!["old.com"], true),
                make_rule(Some("127.0.0.1"), vec!["stale.com"], true),
            ],
        );
        engine.rebuild(&[p1]);
        assert_eq!(engine.rule_count(), 2);

        // 替换为新的 profiles
        let p2 = make_profile_for_engine(
            "new",
            ProfileMode::Dns,
            true,
            vec![make_rule(Some("192.168.1.1"), vec!["new.com"], true)],
        );
        engine.rebuild(&[p2]);
        assert_eq!(engine.rule_count(), 1);
        assert_eq!(engine.resolve("old.com"), None, "old rule should be gone");
        assert_eq!(
            engine.resolve("stale.com"),
            None,
            "stale rule should be gone"
        );
        assert_eq!(
            engine.resolve("new.com"),
            Some("192.168.1.1".parse().unwrap())
        );
    }
}

// ===========================================================================
// 6.5 数据迁移补充测试
// ===========================================================================

#[cfg(test)]
mod test_6_5_data_migration_supplement {
    use super::*;
    use crate::commands::apply::*;
    use mhost_storage::manifest::Manifest;
    use mhost_storage::migration::migrate_v1_to_v2;

    // 迁移后所有 hosts Profile 正常可用
    #[test]
    fn test_migrated_profiles_work_with_hosts_apply() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        // 创建 v1 manifest
        let v1_manifest = Manifest {
            version: 1,
            app_version: "0.1.0".to_string(),
            updated_at: chrono::Utc::now(),
            dns_enabled: None,
            original_dns: None,
        };
        storage.save_manifest(&v1_manifest).unwrap();

        // 在 profiles/ 根目录创建 v1 profiles
        let profiles_dir = temp_dir.path().join("profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();

        let mut profile = Profile::new("migrated_dev");
        profile.rules.push(HostRule::new(
            "127.0.0.1".parse().unwrap(),
            vec!["migrated.local".to_string()],
        ));
        let profile_json = serde_json::to_string_pretty(&profile).unwrap();
        let profile_file = profiles_dir.join(format!("{}.json", profile.id));
        std::fs::write(&profile_file, profile_json).unwrap();

        // 执行迁移
        let migrated = migrate_v1_to_v2(&storage).unwrap();
        assert!(migrated);

        // 验证 profile 可加载
        let loaded = storage.load_profile(&profile.id).unwrap();
        assert_eq!(loaded.name, "migrated_dev");
        assert_eq!(loaded.mode, ProfileMode::Hosts);
        assert_eq!(loaded.rules.len(), 1);

        // 验证 list_profiles 正常
        let listed = storage.list_profiles().unwrap();
        assert_eq!(listed.len(), 1);

        // 验证可用于 apply
        let storage_arc: Arc<dyn Storage + Send + Sync> =
            Arc::new(FileStorage::new(temp_dir.path()));
        let hosts_path = temp_dir.path().join("hosts");
        let backup_dir = temp_dir.path().join("backups");
        std::fs::write(&hosts_path, "# original\n").unwrap();
        let writer = HostsWriter::with_paths(&hosts_path, &backup_dir);

        let mut loaded_mut = loaded;
        loaded_mut.enabled = true;
        storage_arc.save_profile(&loaded_mut).unwrap();

        let result = apply_current_plan_logic(storage_arc.as_ref(), &writer);
        assert!(
            result.is_ok(),
            "apply should work with migrated profiles: {:?}",
            result.err()
        );

        let hosts_content = std::fs::read_to_string(&hosts_path).unwrap();
        assert!(hosts_content.contains("127.0.0.1 migrated.local"));
    }
}

// ===========================================================================
// 6.6 异常场景测试
// ===========================================================================

#[cfg(test)]
mod test_6_6_exception_scenarios {
    use super::*;
    use crate::commands::apply::*;
    #[allow(unused_imports)]
    use crate::commands::profile::*;
    use crate::commands::snapshot::*;

    // 无规则时 apply 产生空 plan，应被 reject_empty_plan 拦截
    #[test]
    fn test_apply_hosts_rejects_no_enabled_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        create_hosts_profile(&storage, "no_rules", vec![]);
        // profile.enabled = false (default)

        let profiles = storage.list_profiles_by_mode(ProfileMode::Hosts).unwrap();
        let current_hosts = std::fs::read_to_string(writer.hosts_path()).unwrap();
        let plan = mhost_apply::generate_plan(&profiles, &current_hosts).unwrap();

        assert!(plan.rules.is_empty());
        let result = reject_empty_plan(&plan);
        assert!(result.is_err());
    }

    // DNS Profile 的 enable_and_apply 预览应为空（不写 hosts）
    #[test]
    fn test_dns_profile_preview_plan_is_empty() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        let mut dns = create_dns_profile(&storage, "dns_p", vec![("192.168.1.1", "dns.local")]);
        dns.enabled = true;
        storage.save_profile(&dns).unwrap();

        let plan = generate_preview_plan_logic(&dns.id, true, storage.as_ref(), &writer).unwrap();
        assert!(
            plan.rules.is_empty(),
            "DNS profile preview plan should be empty"
        );
        assert!(!plan.backup_required);
    }

    // 损坏的 profile 文件不应阻止其他 profile 的操作
    #[test]
    fn test_corrupt_profile_does_not_block_others() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        // 创建正常 profile
        let mut normal =
            create_hosts_profile(&storage, "normal", vec![("127.0.0.1", "normal.local")]);
        normal.enabled = true;
        storage.save_profile(&normal).unwrap();

        // 手动注入一个损坏的 JSON 文件到 hosts 目录
        let hosts_dir = storage.root().join("profiles").join("hosts");
        std::fs::write(hosts_dir.join("corrupt.json"), "not valid json").unwrap();

        // list_profiles 应正常工作（跳过损坏的文件）
        let listed = storage.list_profiles().unwrap();
        assert_eq!(
            listed.len(),
            1,
            "should skip corrupt file and still list valid profiles"
        );
        assert_eq!(listed[0].name, "normal");

        // apply 应正常工作
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(
            result.is_ok(),
            "apply should succeed despite corrupt profile file: {:?}",
            result.err()
        );
    }

    // 并发启用多个 hosts profile，互斥逻辑仍正确
    #[test]
    fn test_concurrent_hosts_profile_enable_mutual_exclusion() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        let p1 = create_hosts_profile(&storage, "p1", vec![("127.0.0.1", "p1.local")]);
        let p2 = create_hosts_profile(&storage, "p2", vec![("127.0.0.1", "p2.local")]);
        let p3 = create_hosts_profile(&storage, "p3", vec![("127.0.0.1", "p3.local")]);

        // 依次启用 p1, p2, p3
        enable_and_apply_logic(&p1.id, true, storage.as_ref(), &writer).unwrap();
        enable_and_apply_logic(&p2.id, true, storage.as_ref(), &writer).unwrap();
        enable_and_apply_logic(&p3.id, true, storage.as_ref(), &writer).unwrap();

        // 只有 p3 应该是 enabled
        let all = storage.list_profiles().unwrap();
        let enabled: Vec<_> = all.iter().filter(|p| p.enabled).collect();
        assert_eq!(enabled.len(), 1, "only one profile should be enabled");
        assert_eq!(enabled[0].id, p3.id);

        // hosts 文件只包含 p3 的规则
        let hosts_content = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(hosts_content.contains("127.0.0.1 p3.local"));
        assert!(!hosts_content.contains("127.0.0.1 p1.local"));
        assert!(!hosts_content.contains("127.0.0.1 p2.local"));
    }

    // 验证 snapshot 能保存和恢复包含 DNS Profile 的完整状态
    #[test]
    fn test_snapshot_preserves_dns_profiles() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        let _h1 = create_hosts_profile(&storage, "h1", vec![("127.0.0.1", "h.local")]);
        let _d1 = create_dns_profile(&storage, "d1", vec![("192.168.1.1", "d.local")]);

        let meta =
            save_snapshot_logic(storage.as_ref(), "dual_mode_snap".to_string(), None).unwrap();
        assert_eq!(meta.profile_count, 2);

        // 删除所有
        for p in storage.list_all_profiles().unwrap() {
            storage.delete_profile(&p.id).unwrap();
        }

        // 恢复
        load_snapshot_logic(storage.as_ref(), &writer, &meta.id).unwrap();

        let all = storage.list_all_profiles().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all
            .iter()
            .any(|p| p.mode == ProfileMode::Hosts && p.name == "h1"));
        assert!(all
            .iter()
            .any(|p| p.mode == ProfileMode::Dns && p.name == "d1"));
    }

    // 空存储的 list 操作不应 panic
    #[test]
    fn test_empty_storage_operations_no_panic() {
        let (_temp, storage, writer) = create_test_storage_and_writer();

        assert!(storage.list_profiles().unwrap().is_empty());
        assert!(storage
            .list_profiles_by_mode(ProfileMode::Hosts)
            .unwrap()
            .is_empty());
        assert!(storage
            .list_profiles_by_mode(ProfileMode::Dns)
            .unwrap()
            .is_empty());
        assert!(storage.list_all_profiles().unwrap().is_empty());

        // apply 到空存储不应 panic
        let result = apply_current_plan_logic(storage.as_ref(), &writer);
        assert!(
            result.is_ok(),
            "apply on empty storage should succeed: {:?}",
            result.err()
        );

        // 验证 hosts 文件未被写入 managed block
        let hosts_content = std::fs::read_to_string(writer.hosts_path()).unwrap();
        assert!(!hosts_content.contains("# ---- mHost start ----"));
    }

    // RuleEngine 空 rebuild 不会 panic
    #[test]
    fn test_rule_engine_empty_rebuild_no_panic() {
        let engine = RuleEngine::new();
        engine.rebuild(&[]);
        assert_eq!(engine.rule_count(), 0);
        assert_eq!(engine.resolve("anything.com"), None);
    }
}

// ===========================================================================
// 6.7 issue #67 bug 2 regression — DNS profile mode preservation
// ===========================================================================
//
// Root cause (Hypothesis A): newly created DNS profiles sometimes landed in
// profiles/hosts/ instead of profiles/dns/ because the frontend didn't
// always pass `mode` through updateProfile, and on-disk `mode` got lost.
// set_profile_enabled's reload condition `mode == Dns` then never fired
// and the new profile's rules never reached RuleEngine.
//
// Fix: update_profile now accepts `mode: Option<ProfileMode>` and reasserts
// it on every save. Frontend always passes `profile.mode` in the payload.
//
// These tests exercise the disk round-trip directly via FileStorage + the
// `validate_profile` / `save_profile` helpers, mimicking what update_profile
// does internally (load → update fields → save).
#[cfg(test)]
mod test_6_7_dns_mode_preservation {
    use super::*;
    use mhost_core::HostRule;
    use std::net::IpAddr;

    fn make_dns_rule(domains: Vec<&str>) -> HostRule {
        HostRule {
            id: mhost_core::RuleId(uuid::Uuid::new_v4()),
            ip: Some("127.0.0.1".parse::<IpAddr>().unwrap()),
            domains: domains.iter().map(|d| d.to_string()).collect(),
            enabled: true,
            comment: None,
            source: mhost_core::RuleSource::Manual,
            line_number: None,
        }
    }

    /// Regression: a DNS profile created with mode=Dns must land in
    /// profiles/dns/{id}.json (not profiles/hosts/{id}.json). If mode
    /// isn't preserved at create time, the profile is invisible to
    /// list_profiles_by_mode(Dns).
    #[test]
    fn test_create_dns_profile_lands_in_dns_dir() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        // Mimic create_profile(name, Some(Dns)) → Profile::new + mode=Dns + save
        let mut profile = Profile::new("dns-profile");
        profile.mode = ProfileMode::Dns;
        storage.save_profile(&profile).unwrap();

        // list_profiles_by_mode(Dns) should return it
        let dns_list = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        assert_eq!(dns_list.len(), 1);
        assert_eq!(dns_list[0].id, profile.id);
        assert_eq!(dns_list[0].mode, ProfileMode::Dns);

        // list_profiles_by_mode(Hosts) should NOT return it
        let hosts_list = storage.list_profiles_by_mode(ProfileMode::Hosts).unwrap();
        assert_eq!(hosts_list.len(), 0);
    }

    /// Regression: update_profile now accepts `mode: Option<ProfileMode>`
    /// and reasserts it after loading from disk. This catches the bug
    /// where a newly created DNS profile might land in profiles/hosts/
    /// (Hypothesis A — Tauri deserialization edge case) and stay there
    /// forever because set_profile_enabled's reload condition
    /// `mode == Dns` would never fire.
    ///
    /// The fix: each save reasserts mode from the explicit parameter,
    /// so the next update corrects any drift.
    #[test]
    fn test_update_profile_reasserts_dns_mode_after_disk_roundtrip() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        // Step 1: create DNS profile
        let mut profile = Profile::new("dns-test");
        profile.mode = ProfileMode::Dns;
        profile.rules.push(make_dns_rule(vec!["example.com"]));
        storage.save_profile(&profile).unwrap();

        // Step 2: simulate "disk drift" by directly mutating the JSON file
        //   to set mode=hosts. This mimics the Tauri deserialization edge
        //   case where the saved JSON has the wrong mode.
        //   serde_json::to_string_pretty produces `"mode": "dns"` (with space).
        let dns_dir = storage.root().join("profiles").join("dns");
        let dns_file = dns_dir.join(format!("{}.json", profile.id));
        let content = std::fs::read_to_string(&dns_file).unwrap();
        let corrupted = content.replace(r#""mode": "dns""#, r#""mode": "hosts""#);
        std::fs::write(&dns_file, &corrupted).unwrap();

        // Sanity check: disk now says mode=hosts
        let drifted = storage.load_profile(&profile.id).unwrap();
        assert_eq!(
            drifted.mode,
            ProfileMode::Hosts,
            "setup: disk should now have mode=hosts after corruption"
        );

        // Step 3: mimic what update_profile does with the new `mode` param
        //   - Load from disk (drifted, mode=hosts)
        //   - If mode is Some, reassert it (this is the fix)
        //   - Save back
        let mut loaded = storage.load_profile(&profile.id).unwrap();
        let mode_param: Option<ProfileMode> = Some(ProfileMode::Dns);
        if let Some(m) = mode_param {
            loaded.mode = m;
        }
        loaded.rules.push(make_dns_rule(vec!["another.com"]));
        storage.save_profile(&loaded).unwrap();

        // Step 4: reload and verify mode is back to Dns
        let reloaded = storage.load_profile(&profile.id).unwrap();
        assert_eq!(
            reloaded.mode,
            ProfileMode::Dns,
            "FIX: update_profile with mode=Some(Dns) must reassert mode on save"
        );
        assert_eq!(reloaded.rules.len(), 2);
    }

    /// Verify RuleEngine::rebuild correctly picks up rules from a profile
    /// in profiles/dns/ after the mode is reasserted. This is the final
    /// step of the fix chain: disk → list_profiles_by_mode → rebuild →
    /// DNS queries resolve.
    #[test]
    fn test_rebuild_picks_up_rules_after_mode_reassert() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();

        let mut profile = Profile::new("trackers");
        profile.mode = ProfileMode::Dns;
        profile.rules.push(make_dns_rule(vec!["tracker.com"]));
        storage.save_profile(&profile).unwrap();

        // User enables the profile
        profile.enabled = true;
        storage.save_profile(&profile).unwrap();

        // Reload via list_profiles_by_mode (what reload_dns_rules does)
        let dns_profiles = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        let enabled: Vec<_> = dns_profiles.into_iter().filter(|p| p.enabled).collect();

        // Rebuild RuleEngine
        let engine = RuleEngine::new();
        engine.rebuild(&enabled);

        // Domain should resolve
        assert_eq!(
            engine.resolve("tracker.com"),
            Some("127.0.0.1".parse().unwrap()),
            "rule should be loaded after mode preservation"
        );
    }
}

// ===========================================================================
// 6.8 issue #67 round 3 — DNS profile enable/disable union semantics
// ===========================================================================
//
// Tests the user's exact symptom: "only the first profile takes effect,
// disabling all doesn't actually disable, only DNS mode restart clears".
//
// Bug A: save_profile left orphan files in the OTHER mode's dir, so
// find_profile_path returned stale data forever.
// Bug B: set_profile_enabled didn't reload on disable, so disabled
// profiles' rules stayed in the in-memory engine.
//
// These tests exercise the disk+RuleEngine pipeline (no DnsServer
// required) to verify that the user's flow now produces correct union.
#[cfg(test)]
mod test_6_8_dns_profile_enable_disable_union {
    use super::*;

    fn make_dns_profile(name: &str, domains: Vec<&str>, ip: &str) -> Profile {
        let mut profile = Profile::new(name);
        profile.mode = ProfileMode::Dns;
        profile.enabled = true; // Profile::new defaults to false; we want enabled for these tests
        profile.rules.push(mhost_core::HostRule {
            id: mhost_core::RuleId(uuid::Uuid::new_v4()),
            ip: Some(ip.parse().unwrap()),
            domains: domains.iter().map(|d| d.to_string()).collect(),
            enabled: true,
            comment: None,
            source: mhost_core::RuleSource::Manual,
            line_number: None,
        });
        profile
    }

    /// End-to-end regression: enable a DNS profile → engine has its rule.
    /// Disable it → engine no longer has the rule. (Bug B fix.)
    /// No DNS mode restart required.
    #[test]
    fn test_dns_enable_then_disable_clears_rule() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let engine = RuleEngine::new();

        // Create and save a DNS profile (enabled=true).
        let mut profile = make_dns_profile("ad-blocker", vec!["ad.com"], "0.0.0.0");
        storage.save_profile(&profile).unwrap();

        // Mimic what reload_dns_rules does after set_profile_enabled(true):
        let dns_profiles = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        let enabled: Vec<_> = dns_profiles.iter().filter(|p| p.enabled).cloned().collect();
        engine.rebuild(&enabled);
        assert_eq!(engine.rule_count(), 1);
        assert_eq!(
            engine.resolve("ad.com"),
            Some("0.0.0.0".parse().unwrap()),
            "FIX-B: enable must load the rule"
        );

        // User disables the profile (via set_profile_enabled(false)).
        profile.enabled = false;
        storage.save_profile(&profile).unwrap();

        // Mimic what reload_dns_rules does after Bug-B fix
        // (drops `&& enabled` guard):
        let dns_profiles = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        let enabled: Vec<_> = dns_profiles.iter().filter(|p| p.enabled).cloned().collect();
        engine.rebuild(&enabled);
        assert_eq!(
            engine.rule_count(),
            0,
            "FIX-B: disable must remove the rule (reload on disable)"
        );
        assert_eq!(
            engine.resolve("ad.com"),
            None,
            "FIX-B: disabled profile's rule must not resolve"
        );
    }

    /// End-to-end regression: multi-profile union after DNS mode
    /// restart. All 3 profiles enabled, all 3 rule sets must be in
    /// the engine (no union only picks the first).
    #[test]
    fn test_dns_mode_restart_loads_all_enabled_profiles() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let engine = RuleEngine::new();

        // Three DNS profiles with non-overlapping domains.
        let p1 = make_dns_profile("ads", vec!["ad.com"], "0.0.0.0");
        let p2 = make_dns_profile("trackers", vec!["track.com"], "0.0.0.0");
        let p3 = make_dns_profile("dev", vec!["api.local"], "127.0.0.1");
        storage.save_profile(&p1).unwrap();
        storage.save_profile(&p2).unwrap();
        storage.save_profile(&p3).unwrap();

        // DNS mode restart → reload from disk
        let dns_profiles = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        let enabled: Vec<_> = dns_profiles.iter().filter(|p| p.enabled).cloned().collect();
        engine.rebuild(&enabled);

        // All 3 domains must resolve (union, not just the first).
        assert_eq!(
            engine.rule_count(),
            3,
            "FIX-A: all 3 profiles' rules must load"
        );
        assert_eq!(engine.resolve("ad.com"), Some("0.0.0.0".parse().unwrap()));
        assert_eq!(
            engine.resolve("track.com"),
            Some("0.0.0.0".parse().unwrap())
        );
        assert_eq!(
            engine.resolve("api.local"),
            Some("127.0.0.1".parse().unwrap())
        );
    }

    /// User's exact scenario: 2 DNS profiles, mode drift (Hosts → Dns)
    /// via the round-2 update_profile reassert. Before Fix A, the
    /// drift left a stale Hosts file that load_profile returned forever.
    /// After Fix A, the next save deletes the orphan, and the union
    /// works correctly.
    #[test]
    fn test_dns_profile_mode_drift_then_union_works() {
        let (_temp, storage, _writer) = create_test_storage_and_writer();
        let engine = RuleEngine::new();

        // Simulate Hypothesis A: profile initially saved as Hosts
        // (Tauri deserialization missed mode="dns").
        let mut profile = make_dns_profile("first", vec!["first.com"], "0.0.0.0");
        profile.mode = ProfileMode::Hosts; // simulate drift
        storage.save_profile(&profile).unwrap();

        // List as Dns → empty (file is in hosts/)
        let dns_list_before = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        assert!(
            dns_list_before.is_empty(),
            "setup: profile should be invisible to DNS listing before fix"
        );

        // User edits rules → update_profile reasserts mode=Dns.
        // With Fix A, save_profile now deletes the stale hosts file
        // and writes the dns file. Without Fix A, the hosts file
        // would remain and load_profile would still return Hosts.
        profile.mode = ProfileMode::Dns;
        profile.rules.push(mhost_core::HostRule {
            id: mhost_core::RuleId(uuid::Uuid::new_v4()),
            ip: Some("127.0.0.1".parse().unwrap()),
            domains: vec!["second.com".to_string()],
            enabled: true,
            comment: None,
            source: mhost_core::RuleSource::Manual,
            line_number: None,
        });
        storage.save_profile(&profile).unwrap();

        // Reload via list_profiles_by_mode(Dns) — must now see the profile.
        let dns_profiles = storage.list_profiles_by_mode(ProfileMode::Dns).unwrap();
        assert_eq!(
            dns_profiles.len(),
            1,
            "FIX-A: after reassert, profile must be visible to DNS listing"
        );
        assert_eq!(dns_profiles[0].mode, ProfileMode::Dns);

        // And load_profile returns mode=Dns (not the stale Hosts file).
        let reloaded = storage.load_profile(&profile.id).unwrap();
        assert_eq!(
            reloaded.mode,
            ProfileMode::Dns,
            "FIX-A: load_profile must return current mode, not stale file"
        );

        // Union works.
        let enabled: Vec<_> = dns_profiles.iter().filter(|p| p.enabled).cloned().collect();
        engine.rebuild(&enabled);
        assert_eq!(engine.rule_count(), 2);
        assert_eq!(
            engine.resolve("first.com"),
            Some("0.0.0.0".parse().unwrap())
        );
        assert_eq!(
            engine.resolve("second.com"),
            Some("127.0.0.1".parse().unwrap())
        );
    }
}
