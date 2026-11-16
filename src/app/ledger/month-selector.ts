import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';
import { formatMonthLong, monthOf, shiftMonth } from '../core/format';
import { STRINGS } from '../core/strings';
import { NOW } from './clock';

/** ‹ noviembre 2026 › selector. A `null` month means "no month" (whole history); next is capped at the current month. */
@Component({
  selector: 'app-month-selector',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="month" role="group" [attr.aria-label]="t.month">
      <button type="button" class="btn-icon" [attr.aria-label]="t.prevMonth" (click)="step(-1)">
        <svg class="icon" aria-hidden="true" focusable="false" viewBox="0 0 16 16"><path d="M10 3L5 8l5 5" /></svg>
      </button>
      <span class="month-label" aria-live="polite">{{ label() }}</span>
      <button type="button" class="btn-icon" [attr.aria-label]="t.nextMonth" [disabled]="atCurrent()" (click)="step(1)">
        <svg class="icon" aria-hidden="true" focusable="false" viewBox="0 0 16 16"><path d="M6 3l5 5-5 5" /></svg>
      </button>
    </div>
  `,
})
export class MonthSelector {
  private readonly now = inject(NOW);
  protected readonly t = STRINGS.ledger;

  readonly month = input<string | null>(null);
  readonly monthChange = output<string>();

  private readonly current = computed(() => monthOf(this.now()));
  protected readonly label = computed(() => {
    const m = this.month();
    return m ? formatMonthLong(m) : this.t.allTime;
  });
  protected readonly atCurrent = computed(() => {
    const m = this.month();
    return m === null || m >= this.current();
  });

  protected step(delta: number): void {
    const m = this.month();
    if (m === null) {
      if (delta < 0) this.monthChange.emit(this.current());
      return;
    }
    const next = shiftMonth(m, delta);
    if (next <= this.current()) this.monthChange.emit(next);
  }
}
