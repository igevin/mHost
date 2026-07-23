import { invoke } from "@tauri-apps/api/core";
import type {
  Profile,
  ApplyPlan,
  ApplyOutcome,
  ValidateResult,
  ExportFormat,
  SnapshotMeta,
  DnsStatus,
  ProfileMode,
} from "../types";

// ---- Profile commands ----

export async function listProfiles(mode?: ProfileMode): Promise<Profile[]> {
  return invoke("list_profiles", { mode });
}

export async function getProfile(id: string): Promise<Profile> {
  return invoke("get_profile", { id });
}

export async function createProfile(name: string, mode?: ProfileMode): Promise<Profile> {
  return invoke("create_profile", { name, mode });
}

export async function updateProfile(profile: Profile): Promise<Profile> {
  // **fix issue #67 bug 2**: 显式带上 mode。后端 update_profile 默认从 disk
  // 读 mode，如果 disk 上的 mode 是错的（Hypothesis A：create 时
  // Tauri 反序列化 Option<ProfileMode> 漏掉 → 落盘为 Hosts default），
  // 编辑规则后仍然错。显式传 mode 后每次 update 都会强制 reassert。
  return invoke("update_profile", {
    id: profile.id,
    name: profile.name,
    description: profile.description,
    rules: profile.rules,
    mode: profile.mode,
  });
}

export async function deleteProfile(id: string): Promise<void> {
  return invoke("delete_profile", { id });
}

export async function setProfileEnabled(
  id: string,
  enabled: boolean,
): Promise<Profile> {
  return invoke("set_profile_enabled", { id, enabled });
}

// ---- Enable & Apply (single atomic command) ----

export async function enableAndApply(
  id: string,
  enabled: boolean,
): Promise<ApplyOutcome> {
  return invoke<ApplyOutcome>("enable_and_apply", { id, enabled });
}

/// Read-only IPC: compute what an `enableAndApply(id, enabled)` call would
/// produce, without writing anything. Refs #127.
export async function previewApplyOutcome(
  id: string,
  enabled: boolean,
): Promise<ApplyOutcome> {
  return invoke<ApplyOutcome>("preview_apply_outcome", { id, enabled });
}

// ---- Apply commands ----

export async function generateApplyPlan(): Promise<ApplyPlan> {
  return invoke("generate_apply_plan");
}

export async function applyHosts(): Promise<void> {
  return invoke("apply_hosts");
}

export async function rollbackHosts(): Promise<void> {
  return invoke("rollback_hosts");
}

export async function readSystemHosts(): Promise<string> {
  return invoke("read_system_hosts");
}

// ---- Validate commands ----

export async function validateHostsText(text: string): Promise<ValidateResult> {
  return invoke("validate_hosts_text", { text });
}

// ---- Import / Export / Duplicate commands ----

export async function importProfile(name: string, hostsText: string): Promise<Profile> {
  return invoke("import_profile", { name, hostsText });
}

export async function importProfileFromFile(name: string, path: string): Promise<Profile> {
  return invoke("import_profile_from_file", { name, path });
}

export async function exportProfile(id: string, format: ExportFormat): Promise<string> {
  return invoke("export_profile", { id, format });
}

export async function exportProfileToFile(id: string, format: ExportFormat, path: string): Promise<void> {
  return invoke("export_profile_to_file", { id, format, path });
}

export async function duplicateProfile(id: string, newName: string): Promise<Profile> {
  return invoke("duplicate_profile", { id, newName });
}

// ---- Hosts block commands ----

export async function getManagedBlockContent(): Promise<string | null> {
  return invoke("get_managed_block_content");
}

export async function getLastApplied(): Promise<string | null> {
  return invoke("get_last_applied");
}

export async function generatePreviewPlan(id: string, enabled: boolean): Promise<ApplyPlan> {
  return invoke("generate_preview_plan", { id, enabled });
}

// ---- Snapshot commands ----

export async function saveSnapshot(name: string, description?: string): Promise<SnapshotMeta> {
  return invoke<SnapshotMeta>("save_snapshot", { name, description });
}

export async function listSnapshots(): Promise<SnapshotMeta[]> {
  return invoke<SnapshotMeta[]>("list_snapshots");
}

export async function loadSnapshot(id: string): Promise<void> {
  return invoke<void>("load_snapshot", { id });
}

export async function deleteSnapshot(id: string): Promise<void> {
  return invoke<void>("delete_snapshot", { id });
}

// ---- DNS commands ----

export async function setDnsMode(enabled: boolean): Promise<void> {
  return invoke("set_dns_mode", { enabled });
}

export async function getDnsMode(): Promise<boolean> {
  return invoke("get_dns_mode");
}

export async function reloadDnsRules(): Promise<void> {
  return invoke("reload_dns_rules");
}

export async function getDnsStatus(): Promise<DnsStatus> {
  return invoke("get_dns_status");
}

export async function listDnsProfiles(): Promise<Profile[]> {
  return invoke("list_dns_profiles");
}

// ---- Update commands ----

export interface LatestRelease {
  tag: string;
  url: string;
  title: string | null;
  body: string | null;
}

export async function checkUpdate(currentVersion: string): Promise<LatestRelease | null> {
  return invoke<LatestRelease | null>("check_update", { currentVersion });
}
