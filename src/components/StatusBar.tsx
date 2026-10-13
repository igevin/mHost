import { useCallback } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { useNavigate } from "react-router-dom";
import {
  enabledProfileAtom,
  isApplyingAtom,
  dnsEnabledAtom,
  enabledDnsProfilesAtom,
  dnsRuleCountAtom,
  dnsProfilesAtom,
  isDnsLoadingAtom,
  toggleDnsModeAtom,
  cancelActiveDnsToggle,
  // issue #232
  dnsDiscrepancyAtom,
} from "../stores/profiles";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import styles from "./StatusBar.module.css";

/**
 * 侧栏底部状态条 —— issue #67 改为双栏：
 *  - 左栏：Hosts 模式 active profile（v0.3 既有行为）
 *  - 右栏：DNS 模式摘要（N 个 profile · M 个启用 · K 条规则）
 *
 * 右栏行为（Phase 2 起）：
 *  - 卡内嵌一个 mini switch，直接走 `toggleDnsModeAtom` 开关 DNS 模式；
 *    toggle 进行中（isDnsLoadingAtom）时 switch 禁用并露出 Cancel
 *    （`cancelActiveDnsToggle`，issue #149 的 AbortController 取消路径，
 *    与 Settings 页同一约定：Cancel 是**兄弟节点**，不做同位 swap，
 *    否则 trailing click 会落到新挂载的 Cancel 上）。
 *  - 点卡片本体：DNS on → /dns-profiles；DNS off → /settings。
 *  - `stuck_at_loopback`（issue #232）：不显示 switch（修复必须走
 *    Settings 的 Restore 路径），点卡片导航到 /settings 并带
 *    `location.state.highlightDnsDiscrepancy`，Settings 会滚动并高亮
 *    分歧横幅。
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
  // Phase 2: 卡内 switch 直接开关 DNS 模式。isDnsLoadingAtom 的写入只在
  // toggle 起止两次发生，订阅它不会引入 #90 点名的那类高频重渲染。
  const isDnsLoading = useAtomValue(isDnsLoadingAtom);
  const toggleDnsMode = useSetAtom(toggleDnsModeAtom);
  // issue #232: 与 `dnsEnabledAtom` 的分歧。**新增的订阅是纯派生的读**，
  // 不发任何 IPC；它只在 `dnsEnabledAtom` 或 `systemDnsAtom` 变化时才可能
  // 触发本组件重渲染 —— 后者只在启动 / 窗口 focus / DNS toggle 时写。
  // 两个都不是 apply 对话框的状态，所以不构成 #90 点过的「sidebar atom
  // 订阅」那类问题（那个的根因是 Layout 重渲染拖垮整棵侧栏树，已由 P-F1
  // 的 React.memo 拆分修掉）。
  const dnsDiscrepancy = useAtomValue(dnsDiscrepancyAtom);

  const navigate = useNavigate();
  // WebKit pointerdown/click 去重 —— 与 Settings 的 handleToggleDns 同一
  // pattern B（见 useWebKitPointerDown 文档）。没有这道 guard，一次点击
  // 会调两次 toggleDnsMode：第二次覆盖进行中的 AbortController slot，
  // 且同步重渲染会让 trailing click 落到新挂载的 Cancel 上，把 Rust 侧
  // 的 set_dns_mode 在 TCC 弹窗前就取消掉（issue #149 的症状）。
  const { fire, releaseSoon } = useWebKitPointerDown();

  const totalDns = dnsProfiles.length;
  const enabledDns = enabledDnsProfiles.length;
  const dnsStuck = dnsDiscrepancy === "stuck_at_loopback";

  // issue #232: stuck ⟹ `dnsEnabled === false`（见 `dnsDiscrepancyAtom`
  // 的真值表），所以既有的 `handleDnsClick` 已经会落到 `/settings`，
  // 不需要为警示加一条分支。这里点警示 = 点整张卡 = 去 Settings 修复。
  // Phase 2: 带 location.state 让 Settings 滚动并高亮分歧横幅。
  const handleDnsClick = () => {
    if (dnsStuck) {
      navigate("/settings", { state: { highlightDnsDiscrepancy: true } });
    } else {
      navigate(dnsEnabled ? "/dns-profiles" : "/settings");
    }
  };

  const handleToggleDns = useCallback(
    (enabled: boolean) => {
      if (!fire()) return;
      releaseSoon();
      toggleDnsMode(enabled);
    },
    [fire, releaseSoon, toggleDnsMode],
  );

  // issue #149: 取消进行中的 DNS toggle（AbortController + cancel_dns_mode
  // IPC）。无在途 toggle 时是 no-op。
  const handleCancelDns = useCallback(() => {
    cancelActiveDnsToggle();
  }, []);

  const dnsTitle = dnsStuck
    ? "System DNS still points at 127.0.0.1 — domain resolution is likely broken. Click to open Settings and restore it."
    : dnsEnabled
      ? `${enabledDns}/${totalDns} DNS profiles enabled · ${dnsRuleCount} active rules`
      : "DNS mode off — flip the switch or click to open Settings";

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
              UI 文案统一英文：同栏的 "Off" / "Applying..." / "2/3 enabled"
              以及 Settings 页那段横幅全是英文 —— 状态标签
              不该是那个唯一的例外。

              `aria-live="polite"`：Settings 横幅有 `role="alert"`，侧栏
              这张卡没有等价的可达性信号 —— 而 #232 的目标就是「让用户
              注意到」，非视觉用户没有别的通道。放这里而不是整张卡上，
              是为了只播报这行状态文字。噪音可控：这一行的内容只随
              用户主动操作（开关 DNS / 切 profile / 规则重载）变化。*/}
          <div
            className={
              dnsStuck
                ? `${styles.statusProfile} ${styles.statusProfileDanger}`
                : styles.statusProfile
            }
            aria-live="polite"
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
          {/* Phase 2: 卡内 mini switch 直接开关 DNS 模式。stuck 态刻意
              不渲染 switch —— 修复必须走 Settings 的 Restore 路径
              （后端 disable 分支会真的执行系统 DNS 还原）。Cancel 与
              switch 是兄弟节点，不做同位 swap（issue #149）。 */}
          {!dnsStuck && (
            <div className={styles.dnsToggleRow}>
              <label
                className={styles.toggle}
                title={dnsEnabled ? "Disable DNS mode" : "Enable DNS mode"}
                aria-label="Toggle DNS mode"
                onClick={(e) => {
                  e.stopPropagation();
                  e.preventDefault();
                  if (isDnsLoading) return;
                  handleToggleDns(!dnsEnabled);
                }}
                onPointerDown={(e) => {
                  if (e.button !== 0) return;
                  e.stopPropagation();
                  if (isDnsLoading) return;
                  handleToggleDns(!dnsEnabled);
                }}
              >
                <input
                  type="checkbox"
                  role="switch"
                  checked={dnsEnabled}
                  readOnly
                  disabled={isDnsLoading}
                  data-testid="dns-mode-switch"
                />
                <span className={styles.toggleSlider} />
              </label>
              {isDnsLoading && (
                <button
                  className={styles.dnsCancelBtn}
                  data-testid="dns-cancel-button"
                  onClick={(e) => {
                    e.stopPropagation();
                    handleCancelDns();
                  }}
                >
                  Cancel
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default StatusBar;
