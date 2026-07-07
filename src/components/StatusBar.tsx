import { useAtomValue } from "jotai";
import { useNavigate } from "react-router-dom";
import {
  enabledProfileAtom,
  isApplyingAtom,
  dnsEnabledAtom,
  enabledDnsProfilesAtom,
  dnsRuleCountAtom,
  dnsProfilesAtom,
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
 */
function StatusBar() {
  const enabledProfile = useAtomValue(enabledProfileAtom);
  const isApplying = useAtomValue(isApplyingAtom);

  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const dnsProfiles = useAtomValue(dnsProfilesAtom);
  const enabledDnsProfiles = useAtomValue(enabledDnsProfilesAtom);
  const dnsRuleCount = useAtomValue(dnsRuleCountAtom);

  const navigate = useNavigate();

  const totalDns = dnsProfiles.length;
  const enabledDns = enabledDnsProfiles.length;

  const handleDnsClick = () => {
    navigate(dnsEnabled ? "/dns-profiles" : "/settings");
  };

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
          title={
            dnsEnabled
              ? `${enabledDns}/${totalDns} DNS profiles enabled · ${dnsRuleCount} active rules`
              : "DNS mode off — click to open Settings"
          }
        >
          <div className={styles.statusRow}>
            <span className={styles.statusLabel}>DNS</span>
            <span
              className={`${styles.statusDot} ${
                dnsEnabled ? styles.statusDotOn : styles.statusDotOff
              }`}
            />
          </div>
          <div className={styles.statusProfile}>
            {dnsEnabled
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
