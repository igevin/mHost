import type { HostRule } from "../types";
import { isCommentOnly } from "../lib/rules";
import styles from "../pages/ProfileEdit.module.css";

interface RuleListProps {
  rules: HostRule[];
}

function RuleList({ rules }: RuleListProps) {
  if (rules.length === 0) {
    return (
      <div className="empty-state">
        <p>No rules in this profile.</p>
        <p className="empty-hint">
          Rule editing will be available in a later phase.
        </p>
      </div>
    );
  }

  return (
    <div className={styles.ruleList}>
      {rules.map((rule) => {
        // Comment-only rule: render as a muted comment line
        if (isCommentOnly(rule)) {
          return (
            <div key={rule.id} className={`${styles.ruleItem} ${styles.ruleItemComment}`}>
              <div className={styles.ruleComment}>{rule.comment}</div>
            </div>
          );
        }
        return (
          <div
            key={rule.id}
            className={`${styles.ruleItem} ${rule.enabled ? "" : styles.ruleItemDisabled}`}
          >
            <div className={styles.ruleHeader}>
              <span className={styles.ruleIp}>{rule.ip}</span>
              <span className={styles.ruleStatus}>
                {rule.enabled ? "On" : "Off"}
              </span>
            </div>
            <div className={styles.ruleDomains}>
              {rule.domains.join(", ")}
            </div>
            {rule.comment && (
              <div className={styles.ruleComment}>{rule.comment}</div>
            )}
          </div>
        );
      })}
    </div>
  );
}

export default RuleList;
