import { invoke } from '@tauri-apps/api/core';

export const activate = (key: string) =>
  invoke<boolean>('plugin:keylight|activate', { key });

export const validate = () =>
  invoke<boolean>('plugin:keylight|validate');

export const hasEntitlement = (feature: string) =>
  invoke<boolean>('plugin:keylight|has_entitlement', { feature });
