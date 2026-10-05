import { useAtomValue } from "jotai";
import { useNavigate } from "react-router-dom";
import {
  enabledProfileAtom,
  isApplyingAtom,
  dnsEnabledAtom,
  enabledDnsProfilesAtom,
  dnsRuleCountAtom,
  dnsProfilesAtom,
  // issue #232
  dnsDiscrepancyAtom,
} from "../stores/profiles";
import styles from "./StatusBar.module.css";

/**
 * 侧栏底部状态条 —— issue #67 改为双栏：
 *  - 左栏：Hosts 模式 active profile（v0.3 既有行为）
 *  - 右栏：DNS 模式摘要（N 个 profile · M 个启用 · K 条规则）
 *
 * 右栏点击行为：
 *  - DNS on：导航到 /dns-profiles（列表落地页），可管理多个启用的 profile
 *  - DNS off：导航到 /settings（开关 DNS 模式的入口）
 *
 * Issue #232：`stuck_at_loopback` 的**常驻**警示入口。Settings 页的横幅
 * （`Settings.tsx` 的 `dns-discrepancy-banner`）只在用户主动去 Settings 时
 * 才看得到，但这个状态的含义是**用户的域名解析已经坏了** —— 他更可能在
 * 浏览器/终端里发现问题，然后回到 mHost 乱点，不会想到去 Settings。
 * 侧栏底栏在所有路由下都常驻，所以把警示放在这里。
 *
 * 为什么不挂一个全局横幅：会与 Settings 页那段文案 + Restore 按钮重复，
 * 而「不打扰」是明写的产品原则。分工是 —— 侧栏负责**让用户注意到**
 * （红点 + "DNS broken" 文字），Settings 负责**解释与修复**（详细文案 +
 * 一键 Restore）。所以这里不复制 Restore 按钮，只做跳转。
 *
 * 为什么只对 `stuck_at_loopback` 生效：`not_pointing` 方向 DNS 本身还能用，
 * 只是 mHost 规则没生效（issue #232「不在范围内」明确不处理）。用它去
 * 打扰一个网络正常的用户不划算。
 */
function StatusBar() {
  const enabledProfile = useAtomValue(enabledProfileAtom);
  const isApplying = useAtomValue(isApplyingAtom);

  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const dnsProfiles = useAtomValue(dnsProfilesAtom);
  const enabledDnsProfiles = useAtomValue(enabledDnsProfilesAtom);
  const dnsRuleCount = useAtomValue(dnsRuleCountAtom);
  // issue #232: 与 `dnsEnabledAtom` 的分歧。**新增的订阅是纯探测派生的
  // 读**，只在 `systemDnsAtom` 变化时（启动 / 窗口 focus / DNS toggle）
  // 才可能重渲染 StatusBar —— 与 apply 对话框状态无关，所以不构成 #90
  // 点过的「sidebar atom 订阅」那类问题（那个问题的根因是 Layout 重渲染
  // 拖垮整棵侧栏树，已由 P-F1 的 React.memo 拆分修掉）。
  const dnsDiscrepancy = useAtomValue(dnsDiscrepancyAtom);

  const navigate = useNavigate();

  const totalDns = dnsProfiles.length;
  const enabledDns = enabledDnsProfiles.length;
  const dnsStuck = dnsDiscrepancy === "stuck_at_loopback";

  // issue #232: stuck ⟹ `dnsEnabled === false`（见 `dnsDiscrepancyAtom`
  // 的真值表），所以既有的 `handleDnsClick` 已经会落到 `/settings`，
  // 不需要为警示加一条分支。这里点警示 = 点整张卡 = 去 Settings 修复。
  const handleDnsClick = () => {
    navigate(dnsEnabled ? "/dns-profiles" : "/settings");
  };

  const dnsTitle = dnsStuck
    ? "System DNS still points at 127.0.0.1 — domain resolution is likely broken. Click to open Settings and restore it."
    : dnsEnabled
      ? `${enabledDns}/${totalDns} DNS profiles enabled · ${dnsRuleCount} active rules`
      : "DNS mode off — click to open Settings";

  return (
    <div className={styles.sidebarFooter}>
      <div className={styles.statusRow2}>
        {/* Hosts column */}
        <div
          className={styles.statusCard}
          role="button"
          tabIndex={0}
          onClick={() => navigate("/profiles")}
          onKeyDown={(e) => {
            if (e.key === "Enter") navigate("/profiles");
          }}
        >
          <div className={styles.statusRow}>
            <span className={styles.statusLabel}>Hosts</span>
            <span
              className={`${styles.statusDot} ${
                enabledProfile ? styles.statusDotOn : styles.statusDotOff
              }`}
            />
          </div>
          <div className={styles.statusProfile}>
            {enabledProfile ? enabledProfile.name : "None"}
          </div>
          {isApplying && (
            <div className={styles.statusApplying}>Applying...</div>
          )}
        </div>

        {/* DNS column */}
        <div
          className={styles.statusCard}
          role="button"
          tabIndex={0}
          onClick={handleDnsClick}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleDnsClick();
          }}
          title={dnsTitle}
          data-testid="dns-status-card"
          data-dns-stuck={dnsStuck ? "true" : undefined}
        >
          <div className={styles.statusRow}>
            <span className={styles.statusLabel}>DNS</span>
            <span
              className={`${styles.statusDot} ${
                dnsStuck
                  ? styles.statusDotDanger
                  : dnsEnabled
                    ? styles.statusDotOn
                    : styles.statusDotOff
              }`}
            />
          </div>
          {/* issue #232: 警示态下文案必须**自解释** —— 一个 8px 红点
              在 11px 的 muted 底栏里太容易被划过去，而 "DNS broken"
              配合 danger 颜色即使被扫到也读得懂。
              刻意用英文：同栏的 "Off" / "Applying..." / "2/3 enabled"
              以及 Settings 页那段横幅全是英文，前端只有
              RuleEditor / ImportDialog 的冲突诊断是中文 —— 状态标签
              不该是那个唯一的例外。 */}
          <div
            className={
              dnsStuck
                ? `${styles.statusProfile} ${styles.statusProfileDanger}`
                : styles.statusProfile
            }
            data-testid={dnsStuck ? "dns-stuck-warning" : undefined}
          >
            {dnsStuck
              ? "DNS broken"
              : dnsEnabled
                ? `${enabledDns}/${totalDns} enabled · ${dnsRuleCount} ${
                    dnsRuleCount === 1 ? "rule" : "rules"
                  }`
                : "Off"}
          </div>
        </div>
      </div>
    </div>
  );
}

export default StatusBar;
