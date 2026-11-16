import { CdkTrapFocus } from '@angular/cdk/a11y';
import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';

/**
 * Modal dialog shell: backdrop, focus trap (focus returns to the opener on close) and Escape to dismiss.
 * Content is projected; the caller provides the heading with the id given in `labelledBy`.
 */
@Component({
  selector: 'app-modal',
  imports: [CdkTrapFocus],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="backdrop" (mousedown)="onBackdrop($event)" (keydown.escape)="dismiss.emit()">
      <div
        class="dialog"
        role="dialog"
        aria-modal="true"
        tabindex="-1"
        cdkTrapFocus
        [cdkTrapFocusAutoCapture]="true"
        [attr.aria-labelledby]="labelledBy()"
      >
        <ng-content />
      </div>
    </div>
  `,
})
export class Modal {
  readonly labelledBy = input.required<string>();
  readonly dismiss = output<void>();

  protected onBackdrop(event: MouseEvent): void {
    if (event.target === event.currentTarget) this.dismiss.emit();
  }
}
