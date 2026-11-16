import { Routes } from '@angular/router';
import { Accounts } from './bank/accounts';
import { Backups } from './settings/backups';

export const routes: Routes = [
  { path: '', pathMatch: 'full', redirectTo: 'resumen' },
  { path: 'resumen', loadComponent: () => import('./dashboard/dashboard').then((m) => m.Dashboard) },
  {
    path: 'movimientos',
    loadComponent: () => import('./transactions/transactions').then((m) => m.Transactions),
  },
  {
    path: 'categorias',
    loadComponent: () => import('./categories/categories').then((m) => m.Categories),
  },
  { path: 'reglas', loadComponent: () => import('./rules/rules').then((m) => m.Rules) },
  {
    path: 'presupuestos',
    loadComponent: () => import('./budgets/budgets').then((m) => m.Budgets),
  },
  { path: 'cuentas', component: Accounts },
  { path: 'importar', loadComponent: () => import('./import/import').then((m) => m.ImportScreen) },
  { path: 'ajustes', component: Backups },
  { path: '**', redirectTo: 'resumen' },
];
