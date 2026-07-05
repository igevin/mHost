import { useCallback, useState } from "react";
import { Outlet } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import {
  applyConfirmOpenAtom,
  applyPlanAtom,
  applyResultAtom,
  applyErrorAtom,
  isApplyingAtom,
  executeApplyAtom,
  closeApplyConfirmAtom,
  rollbackHostsActionAtom,
} from "../stores/profiles";
import StatusBar from "./StatusBar";
import ManagementDrawer from "./ManagementDrawer";
import ApplyConfirmDialog from "./ApplyConfirmDialog";
import Sidebar, { type NavItem } from "./Sidebar";
import styles from "./Layout.module.css";

/* ---- SVG Icons ---- */

function ShieldIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
    </svg>
  );
}

function GlobeIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <circle cx="12 12" r="10" />
      <line x1="2 12" x2="22 12" />
      <path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" />
    </svg>
  );
}

function BackupIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
      <polyline points="7 10 12 15 17 10" />
      <line x1="12 15" x2="12 3" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <circle cx="12 12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
    </svg>
  );
}

function FileTextIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
      <polyline points="14 2 14 8 20 8" />
      <path d="M16 13H8M16 17H8M10 9H9H8" />
    </svg>
  );
}

function DnsIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      width="18"
      height="18"
    >
      <circle cx="12 12" r="10" />
      <path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" />
    </svg>
  );
}

/* ---- Static tool nav items (module-level constant, created once) ---- */

const toolNavItems: NavItem[] = [
  {
    to: "#adblock",
    label: "Ad Block",
    icon: <ShieldIcon />,
    badge: 10,
    disabled: true,
  },
  {
    to: "#remote",
    label: "Remote Rules",
    icon: <GlobeIcon />,
    badge: 3,
    disabled: true,
  },
  {
    to: "/snapshot",
    label: "Snapshots",
    icon: <BackupIcon />,
    disabled: false,
  },
  {
    to: "/dns-profiles",
    label: "DNS Profiles",
    icon: <DnsIcon />,
    disabled: false,
  },
  {
    to: "/hosts",
    label: "System Hosts",
    icon: <FileTextIcon />,
    disabled: false,
  },
  { to: "/settings", label: "Settings", icon: <SettingsIcon /> },
];

function Layout() {
  const [showManagement, setShowManagement] = useState(false);

  const applyConfirmOpen = useAtomValue(applyConfirmOpenAtom);
  const applyPlan = useAtomValue(applyPlanAtom);
  const applyResult = useAtomValue(applyResultAtom);
  const applyError = useAtomValue(applyErrorAtom);
  const isApplying = useAtomValue(isApplyingAtom);
  const executeApply = useSetAtom(executeApplyAtom);
  const closeApplyConfirm = useSetAtom(closeApplyConfirmAtom);
  const rollbackHostsAction = useSetAtom(rollbackHostsActionAtom);
  const setApplyError = useSetAtom(applyErrorAtom);

  // **fix (P-F1, issue #90)**: useCallback 保持 onOpenManagement 引用稳定，
  // 否则 inline arrow 会让 Sidebar 的 React.memo 失效（每次 Layout 渲染
  // 都看到新 props）。
  const handleOpenManagement = useCallback(() => {
    setShowManagement(true);
  }, []);

  return (
    <div className={styles.mhostLayout}>
      <Sidebar
        toolNavItems={toolNavItems}
        onOpenManagement={handleOpenManagement}
      />

      <main className={styles.mhostMain}>
        {applyError && !applyConfirmOpen && (
          <div className="alert alert-error">
            {applyError}
            <button
              className="btn btn-ghost btn-sm"
              onClick={() => setApplyError(null)}
              style={{ marginLeft: 8 }}
            >
              Dismiss
            </button>
          </div>
        )}
        <Outlet />
      </main>

      <StatusBar />

      <ManagementDrawer
        open={showManagement}
        onClose={() => setShowManagement(false)}
      />

      <ApplyConfirmDialog
        open={applyConfirmOpen}
        plan={applyPlan}
        onConfirm={() => executeApply()}
        onCancel={() => closeApplyConfirm()}
        isApplying={isApplying}
        applyResult={applyResult}
        applyError={applyError}
        onRollback={() => rollbackHostsAction()}
      />
    </div>
  );
}

export default Layout;