import { EnvironmentProviders, Provider } from '@angular/core';
import { provideRouter } from '@angular/router';
import { TAURI_INVOKE, TAURI_LISTEN } from '../core/tauri';
import { NOW } from './clock';

export interface Call {
  command: string;
  args: Record<string, unknown> | undefined;
}

/** Spec harness: a fake `invoke` driven by a handler map, recorded calls and a manual `bank://synced` emitter. */
export function fakeBackend(handlers: Record<string, (args: Record<string, unknown>) => unknown>) {
  const calls: Call[] = [];
  const handlers_: Array<() => void> = [];
  const state = { emit: (): void => handlers_.forEach((h) => h()) };
  const providers: Array<Provider | EnvironmentProviders> = [
    provideRouter([]),
    { provide: NOW, useValue: () => new Date(2026, 10, 15, 12, 0) },
    {
      provide: TAURI_INVOKE,
      useValue: async (command: string, args?: Record<string, unknown>) => {
        calls.push({ command, args });
        const h = handlers[command];
        return h ? structuredClone(h(args ?? {})) : undefined;
      },
    },
    {
      provide: TAURI_LISTEN,
      useValue: async (_event: string, handler: () => void) => {
        handlers_.push(handler);
        return () => undefined;
      },
    },
  ];
  return {
    calls,
    providers,
    state,
    of: (command: string) => calls.filter((c) => c.command === command),
  };
}
