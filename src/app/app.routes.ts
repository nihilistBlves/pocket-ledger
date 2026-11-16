import { Routes } from '@angular/router';
import { Accounts } from './bank/accounts';

export const routes: Routes = [
  { path: '', pathMatch: 'full', redirectTo: 'resumen' },
  { path: 'resumen', loadComponent: () => import('./dashboard/dashboard').then((m) => m.Dashboard) },
  {
    path: 'movimientos',
    loadComponent: () => import('./transactions/transactions').then((m) => m.Transactions),
  },
  { path: 'cuentas', component: Accounts },
  { path: '**', redirectTo: 'resumen' },
];
