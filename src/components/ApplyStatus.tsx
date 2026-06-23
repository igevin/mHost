import { useEffect, useState } from "react";
import { useAtomValue } from "jotai";
import { enabledProfileAtom } from "../stores/profiles";
import {
  getManagedBlockContent,
  getLastApplied,
  generateApplyPlan,
} from "../lib/tauri";
import type { ApplyPlan } from "../types";
import styles from "./ApplyStatus.module.css";

function ApplyStatus() {
  const enabledProfile = useAtomValue(enabledProfileAtom);
  const [managedContent, setManagedContent] = useState<string | null>(null);
  const [lastApplied, setLastApplied] = useState<string | null>(null);
  const [applyPlan, setApplyPlan] = useState<ApplyPlan | null>(null);
  const [planFailed, setPlanFailed] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;

    async function fetchData() {
      setLoading(true);
      try {
        const [content, applied, plan] = await Promise.all([
          getManagedBlockContent(),
          getLastApplied(),
          generateApplyPlan().catch(() => null),
        ]);

        if (cancelled) return;

        setManagedContent(content);
        setLastApplied(applied);

        if (plan) {
          setApplyPlan(plan);
          setPlanFailed(false);
        } else {
          setPlanFailed(true);
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }

    fetchData();
    return () => {
      cancelled = true;
    };
  }, [enabledProfile]);

  if (loading) {
    return <div className={styles.loading}>Loading status...</div>;
  }

  const hasPendingChanges =
    applyPlan !== null &&
    (applyPlan.diff.added.length > 0 || applyPlan.diff.removed.length > 0);

  return (
    <div className={styles.applyStatusSection}>
      {/* Active Profile */}
      <div className="card">
        <h3 className="card-title">Current Active Profile</h3>
        {enabledProfile ? (
          <>
            <div className={styles.profileName}>{enabledProfile.name}</div>
            {enabledProfile.rules.length > 0 && (
              <div className={styles.rulesList}>
                {enabledProfile.rules
                  .filter((r) => r.enabled)
                  .map((rule) => (
                    <div key={rule.id} className={styles.ruleItem}>
                      <span className={styles.ruleIp}>{rule.ip ?? ""}</span>
                      <span className={styles.ruleDomains}>
                        {rule.domains.join(", ")}
                      </span>
                    </div>
                  ))}
              </div>
            )}
          </>
        ) : (
          <div className={styles.noProfile}>No active profile</div>
        )}
      </div>

      {/* Managed Block Content */}
      <div className="card">
        <h3 className="card-title">Managed Block in Hosts</h3>
        {managedContent ? (
          <pre className={styles.managedBlockContent}>{managedContent}</pre>
        ) : (
          <div className={styles.managedBlockEmpty}>
            No managed block found in system hosts.
          </div>
        )}
      </div>

      {/* Last Applied */}
      <div className="card">
        <h3 className="card-title">Apply History</h3>
        <div className={styles.lastAppliedRow}>
          <span className={styles.lastAppliedLabel}>Last Applied:</span>
          <span>
            {lastApplied
              ? new Date(lastApplied).toLocaleString()
              : "Never"}
          </span>
        </div>

        {/* Pending Changes */}
        {!planFailed && hasPendingChanges && (
          <div className={styles.pendingChanges}>
            Pending Changes: {applyPlan.diff.added.length} added,{" "}
            {applyPlan.diff.removed.length} removed
          </div>
        )}

        {/* Conflicts */}
        {applyPlan &&
          applyPlan.conflicts.length > 0 &&
          applyPlan.conflicts.map((conflict) => (
            <div key={conflict.domain} className={styles.conflictWarning}>
              Conflict on{" "}
              <span className={styles.conflictDomain}>
                {conflict.domain}
              </span>
              : {conflict.rules.length} profiles claim this domain
            </div>
          ))}
      </div>
    </div>
  );
}

export default ApplyStatus;
