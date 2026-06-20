import { invoke } from "@tauri-apps/api/core";
import type { Profile, ApplyPlan } from "../types";

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
