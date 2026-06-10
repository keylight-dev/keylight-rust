import { invoke } from '@tauri-apps/api/core';

/**
 * Activate a license key on this device.
 *
 * The returned lease is Ed25519-verified by the Rust side before anything is
 * persisted.
 *
 * @param key - The end user's license key.
 * @returns `true` if the key was activated.
 */
export async function activate(key: string): Promise<boolean> {
  return await invoke<boolean>('plugin:keylight|activate', { key });
}

/**
 * Re-validate the stored license online.
 *
 * @returns `true` if the license is currently valid.
 */
export async function validate(): Promise<boolean> {
  return await invoke<boolean>('plugin:keylight|validate');
}

/**
 * Check whether the active license includes an entitlement. Resolves from the
 * cached, signature-verified lease, so it works offline.
 *
 * @param feature - The entitlement key to check (e.g. `"pro"`).
 */
export async function hasEntitlement(feature: string): Promise<boolean> {
  return await invoke<boolean>('plugin:keylight|has_entitlement', { feature });
}
