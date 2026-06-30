import { useSetAtom } from "jotai";
import { rollbackHostsActionAtom } from "../stores/profiles";
import BackupPanel from "../components/BackupPanel";
import styles from "./Backup.module.css";

function Backup() {
  const rollback = useSetAtom(rollbackHostsActionAtom);

  return (
    <div className="mhost-page">
      <header className="mhost-page-header">
        <h1 className="mhost-page-title">Backup</h1>
        <p className={styles.pageDesc}>
          View and restore previous versions of your hosts file.
        </p>
      </header>

      <div className={styles.backupCard}>
        <BackupPanel onRollback={rollback} />
      </div>
    </div>
  );
}

export default Backup;
