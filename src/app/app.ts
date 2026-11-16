import { Component, OnInit, inject, signal } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { AppInfo, AppInfoService } from './core/app-info.service';
import { AppModeService } from './core/app-mode.service';
import { STRINGS } from './core/strings';
import { Recovery } from './recovery/recovery';

@Component({
  selector: 'app-root',
  imports: [RouterOutlet, RouterLink, RouterLinkActive, Recovery],
  templateUrl: './app.html',
  styleUrl: './app.css',
})
export class App implements OnInit {
  private readonly appInfoService = inject(AppInfoService);

  protected readonly t = STRINGS;
  protected readonly navItems = [
    { label: STRINGS.nav.summary, path: '/resumen' },
    { label: STRINGS.nav.transactions, path: '/movimientos' },
    { label: STRINGS.nav.accounts, path: '/cuentas' },
    { label: STRINGS.nav.settings, path: '/ajustes' },
  ];
  protected readonly appMode = inject(AppModeService);
  protected readonly info = signal<AppInfo | null>(null);
  protected readonly infoError = signal(false);

  ngOnInit(): void {
    void this.appMode.load();
    this.appInfoService
      .get()
      .then((i) => this.info.set(i))
      .catch(() => this.infoError.set(true));
  }
}
