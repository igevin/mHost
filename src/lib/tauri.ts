import { invoke } from "@tauri-apps/api/core";
import type { Profile, ApplyPlan, ValidateResult, ExportFormat } from "../types";

// ---- Profile commands ----

export async function listProfiles(): Promise<Profile[]> {
  return invoke("list_profiles");
}

export async function getProfile(id: string): Promise<Profile> {
  return invoke("get_profile", { id });
}

export async function createProfile(name: string): Promise<Profile> {
  return invoke("create_profile", { name });
}

export async function updateProfile(profile: Profile): Promise<Profile> {
  return invoke("update_profile", { profile });
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
): Promise<void> {
  return invoke("enable_and_apply", { id, enabled });
}

// ---- Apply commands ----

export async function generateApplyPlan(): Promise<ApplyPlan> {
  return invoke("generate_apply_plan");
}

export async function applyHosts(plan: ApplyPlan): Promise<void> {
  return invoke("apply_hosts", { plan });
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
