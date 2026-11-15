import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { App } from './app';
import { AppInfoService } from './core/app-info.service';
import { routes } from './app.routes';

describe('App', () => {
  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [App],
      providers: [
        provideRouter(routes),
        { provide: AppInfoService, useValue: { get: async () => ({ name: 'Pocket Ledger', version: '9.9.9' }) } },
      ],
    }).compileComponents();
  });

  it('renders the nav items and the app info', async () => {
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const el = fixture.nativeElement as HTMLElement;
    const links = Array.from(el.querySelectorAll('nav a')).map((a) => a.textContent?.trim());
    expect(links).toEqual(['Resumen', 'Movimientos', 'Cuentas']);
    expect(el.querySelector('.app-info')?.textContent).toContain('9.9.9');
  });
});
