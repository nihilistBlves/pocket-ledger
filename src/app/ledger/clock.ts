import { InjectionToken } from '@angular/core';

/** Current date, injectable so specs can pin it. */
export const NOW = new InjectionToken<() => Date>('NOW', {
  providedIn: 'root',
  factory: () => () => new Date(),
});
