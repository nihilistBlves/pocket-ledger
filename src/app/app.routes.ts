import { Routes } from '@angular/router';
import { Accounts } from './bank/accounts';
import { Placeholder } from './shell/placeholder';

export const routes: Routes = [
  { path: '', pathMatch: 'full', redirectTo: 'resumen' },
  { path: 'resumen', component: Placeholder },
  { path: 'movimientos', component: Placeholder },
  { path: 'cuentas', component: Accounts },
  { path: '**', redirectTo: 'resumen' },
];
