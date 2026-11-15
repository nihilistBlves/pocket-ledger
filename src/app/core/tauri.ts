import { InjectionToken } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type InvokeFn = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
export type ListenFn = (event: string, handler: () => void) => Promise<() => void>;

/** Tauri `invoke`, injectable so specs can replace it. */
export const TAURI_INVOKE = new InjectionToken<InvokeFn>('TAURI_INVOKE', {
  providedIn: 'root',
  factory: () => (command, args) => invoke(command, args),
});

/** Tauri event subscription, injectable so specs can replace it. */
export const TAURI_LISTEN = new InjectionToken<ListenFn>('TAURI_LISTEN', {
  providedIn: 'root',
  factory: () => (event, handler) => listen(event, () => handler()),
});
