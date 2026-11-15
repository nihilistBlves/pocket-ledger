import { Routes } from '@angular/router';
import { Placeholder } from './shell/placeholder';

export const routes: Routes = [
  { path: '', pathMatch: 'full', redirectTo: 'resumen' },
  { path: 'resumen', component: Placeholder },
  { path: 'movimientos', component: Placeholder },
  { path: 'cuentas', component: Placeholder },
  { path: '**', redirectTo: 'resumen' },
];
