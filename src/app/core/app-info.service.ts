import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';

export interface AppInfo {
  name: string;
  version: string;
}

/** Typed wrapper over the `app_info` Tauri command. */
@Injectable({ providedIn: 'root' })
export class AppInfoService {
  get(): Promise<AppInfo> {
    return invoke<AppInfo>('app_info');
  }
}
