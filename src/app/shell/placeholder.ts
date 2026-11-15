import { Component } from '@angular/core';
import { STRINGS } from '../core/strings';

@Component({
  selector: 'app-placeholder',
  template: `<p class="muted">{{ text }}</p>`,
})
export class Placeholder {
  protected readonly text = STRINGS.placeholder;
}
