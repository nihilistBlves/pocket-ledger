import { Component, OnInit, inject, signal } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { AppInfo, AppInfoService } from './core/app-info.service';
import { STRINGS } from './core/strings';

@Component({
  selector: 'app-root',
  imports: [RouterOutlet, RouterLink, RouterLinkActive],
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
  ];
  protected readonly info = signal<AppInfo | null>(null);
  protected readonly infoError = signal(false);

  ngOnInit(): void {
    this.appInfoService
      .get()
      .then((i) => this.info.set(i))
      .catch(() => this.infoError.set(true));
  }
}
