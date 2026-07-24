import { useState } from "react";
import type { ApplyPlan } from "../types";
import styles from "./DiffView.module.css";

interface DiffViewProps {
  plan: ApplyPlan;
  /** When true, render only the added/removed diff (no conflicts block).
   *  Used by the QuickApplyToast's "View Diff" affordance which has its
   *  own toast-level framing for conflicts. */
  compact?: boolean;
}

/**
 * Renders an `ApplyPlan`'s diff (added / removed / unchanged lines) plus
 * its conflict list. Extracted from `ApplyConfirmDialog` so both the
 * preview dialog and the Quick Apply toast can share the same renderer.
 *
 * **Key stability contract** (Refs #90 P-F10): keys MUST be content-stable,
 * not array indices, so toggling "show unchanged" doesn't reuse DOM nodes
 * for the wrong content. Keep these keys verbatim:
 *   - added:    `+${line}`
 *   - removed:  `-${line}`
 *   - unchanged: `u${line}`
 *   - conflict: `conflict.domain`
 */
function DiffView({ plan, compact = false }: DiffViewProps) {
  const [showUnchanged, setShowUnchanged] = useState(false);
  const diffEmpty = plan.diff.added.length === 0 && plan.diff.removed.length === 0;

  return (
    <div className={styles.diffSection}>
      {diffEmpty ? (
        <div className={styles.diffEmpty}>No changes detected</div>
      ) : (
        <div className={styles.diffPreview}>
          {plan.diff.added.map((line) => (
            <div key={`+${line}`} className={`${styles.diffLine} ${styles.diffAdded}`}>
              + {line}
            </div>
          ))}
          {plan.diff.removed.map((line) => (
            <div key={`-${line}`} className={`${styles.diffLine} ${styles.diffRemoved}`}>
              - {line}
            </div>
          ))}
          {plan.diff.unchanged.length > 0 && (
            <>
              {!showUnchanged ? (
                <button
                  className={styles.diffUnchangedCollapsed}
                  onClick={() => setShowUnchanged(true)}
                >
                  ...{plan.diff.unchanged.length} unchanged lines...
                </button>
              ) : (
                <>
                  {plan.diff.unchanged.map((line) => (
                    <div
                      key={`u${line}`}
                      className={`${styles.diffLine} ${styles.diffUnchanged}`}
                    >
                      {`  ${line}`}
                    </div>
                  ))}
                  <button
                    className={styles.diffUnchangedCollapsed}
                    onClick={() => setShowUnchanged(false)}
                  >
                    Collapse unchanged lines
                  </button>
                </>
              )}
            </>
          )}
        </div>
      )}
      {!compact && plan.conflicts.length > 0 && (
        <div className={styles.conflictSection}>
          <div className={styles.conflictWarning}>
            Warning: {plan.conflicts.length} conflict(s) detected
          </div>
          <div className={styles.conflictList}>
            {plan.conflicts.map((conflict) => (
              <div key={conflict.domain} className={styles.conflictItem}>
                <span className={styles.conflictDomain}>{conflict.domain}</span>
                <span>— {conflict.rules.map((r) => r.source_profile_name).join(", ")}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

export default DiffView;
