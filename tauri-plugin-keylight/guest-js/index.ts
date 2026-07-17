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

/**
 * Validate the stored license against the server (no staleness gate). Call on
 * app launch so a dashboard revoke or expiry takes effect immediately.
 */
export async function checkOnLaunch(): Promise<void> {
  await invoke<void>('plugin:keylight|check_on_launch');
}

/**
 * Re-validate only if the SDK's debounce/staleness policy says it's time.
 *
 * @returns `true`/`false` when a validation ran, `null` when skipped.
 */
export async function refreshIfNeeded(): Promise<boolean | null> {
  return await invoke<boolean | null>('plugin:keylight|refresh_if_needed');
}

/**
 * Send the anonymous keyless beacon (debounced 24h on the Rust side).
 *
 * @param keylessState - `"trial"`, `"free_tier"`, or `"expired"`.
 */
export async function reportKeylessState(
  keylessState: 'trial' | 'free_tier' | 'expired'
): Promise<void> {
  await invoke<void>('plugin:keylight|report_keyless_state', { keylessState });
}
