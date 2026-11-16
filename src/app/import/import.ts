import {
  Component,
  ElementRef,
  Injector,
  OnDestroy,
  OnInit,
  afterNextRender,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import { AccountView, CommandError } from '../bank/bank.models';
import { SYNCED_EVENT, toCommandError } from '../bank/bank.service';
import { formatCents, formatDate, formatDateTime, formatSignedCents } from '../core/format';
import { STRINGS } from '../core/strings';
import { TAURI_LISTEN } from '../core/tauri';
import { Modal } from '../ledger/modal';
import {
  AMOUNT_FORMATS,
  AmountFormatKey,
  AmountMode,
  ColumnOption,
  MappingForm,
  amountFormatKey,
  batchCounts,
  columnOptions,
  columnRoles,
  convertRefs,
  dateFormatOptions,
  decimalSpecFor,
  describeProfile,
  formFromProfile,
  formatKey,
  formatSize,
  guessHeaderRow,
  headerRowIndex,
  profileFromForm,
  rowErrorText,
  sameRef,
} from './import.helpers';
import {
  ColumnRef,
  CsvEncoding,
  ImportBatch,
  ImportCommitResult,
  ImportPreview,
  ImportProfile,
  PickedImport,
  PreviewRow,
  RawTable,
} from './import.models';
import { ImportService } from './import.service';

export type ImportStep = 'start' | 'format' | 'account' | 'review' | 'result';
type Busy = 'pick' | 'preview' | 'commit' | 'undo' | 'account' | null;
type ColumnKey =
  | 'bookingDate'
  | 'valueDate'
  | 'description'
  | 'amount'
  | 'debit'
  | 'credit'
  | 'counterparty';

/** Pause after the last change of a format option before the raw rows are read again. */
export const RAW_DEBOUNCE_MS = 250;
/** Raw rows requested for the mapping table. */
const RAW_ROWS = 40;
const HISTORY_LIMIT = 10;
const MAX_PROFILE_NAME = 80;
const STEP_ORDER: ImportStep[] = ['start', 'format', 'account', 'review', 'result'];

interface UndoTarget {
  batchId: number;
  fileName: string;
  accountName: string;
  inserted: number;
  from: 'history' | 'result';
}

interface ColumnField {
  key: ColumnKey;
  label: string;
  required: boolean;
  hint?: string;
}

/** "Importar" screen: one page with a step flow (file, format, account, review, result). */
@Component({
  selector: 'app-import',
  imports: [Modal, RouterLink],
  templateUrl: './import.html',
  styleUrl: './import.css',
})
export class ImportScreen implements OnInit, OnDestroy {
  private readonly service = inject(ImportService);
  private readonly listen = inject(TAURI_LISTEN);
  private readonly injector = inject(Injector);

  protected readonly t = STRINGS.import;
  protected readonly common = STRINGS.common;

  protected readonly step = signal<ImportStep>('start');
  protected readonly busy = signal<Busy>(null);
  protected readonly error = signal<CommandError | null>(null);
  protected readonly notice = signal<string | null>(null);

  protected readonly history = signal<ImportBatch[]>([]);
  protected readonly historyLoaded = signal(false);
  protected readonly accounts = signal<AccountView[]>([]);

  protected readonly picked = signal<PickedImport | null>(null);

  // Format
  protected readonly manual = signal(false);
  protected readonly profileSelect = signal('');
  protected readonly form = signal<MappingForm | null>(null);
  protected readonly raw = signal<RawTable | null>(null);
  protected readonly rawLoading = signal(false);
  protected readonly rawError = signal<CommandError | null>(null);
  protected readonly saveProfile = signal(false);
  protected readonly saveName = signal('');
  protected readonly formError = signal<string | null>(null);

  // Account and review
  protected readonly selected = signal<number | 'new' | null>(null);
  protected readonly newName = signal('');
  protected readonly newNameError = signal<string | null>(null);
  protected readonly preview = signal<ImportPreview | null>(null);

  // Result and undo
  protected readonly result = signal<ImportCommitResult | null>(null);
  protected readonly undoTarget = signal<UndoTarget | null>(null);

  private readonly heading = viewChild<ElementRef<HTMLElement>>('stepHeading');
  private unlisten: (() => void) | null = null;
  private destroyed = false;
  private rawTimer: ReturnType<typeof setTimeout> | null = null;
  private rawSeq = 0;
  private previewSeq = 0;
  /** Profile chosen when leaving the format step; used to preview and commit. */
  private activeProfile: ImportProfile | null = null;
  private activeProfileName = '';
  /** After the first raw read of a generic prefill, forget column names the file does not have. */
  private pruneNext = false;

  // ------------------------------------------------------------- derived state

  protected readonly stepItems = computed(() => {
    const current = STEP_ORDER.indexOf(this.step()) + 1;
    const labels = this.t.steps;
    return [
      { key: 'file', label: labels.file, index: 1 },
      { key: 'format', label: labels.format, index: 2 },
      { key: 'account', label: labels.account, index: 3 },
      { key: 'review', label: labels.review, index: 4 },
      { key: 'result', label: labels.result, index: 5 },
    ].map((s) => ({
      ...s,
      current: s.index === current,
      done: s.index < current,
    }));
  });

  protected readonly isExcel = computed(() => this.picked()?.suggestedFormat.type === 'Excel');
  protected readonly detected = computed(() => {
    const p = this.picked();
    return p?.detectedProfile ? (p.profiles.find((x) => x.name === p.detectedProfile) ?? null) : null;
  });
  protected readonly detectedText = computed(() => {
    const d = this.detected();
    return d ? describeProfile(d.profile) : '';
  });
  protected readonly fileMeta = computed(() => {
    const p = this.picked();
    if (!p) return '';
    const kind = this.isExcel() ? this.t.fileKind.excel : this.t.fileKind.csv;
    return this.t.fileMeta(kind, formatSize(p.sizeBytes), this.raw()?.totalRows ?? null);
  });
  protected readonly profileChoices = computed(() => {
    const p = this.picked();
    if (!p) return [];
    return p.profiles.filter((x) => x.profile.format.type === p.suggestedFormat.type);
  });

  protected readonly csv = computed(() => {
    const f = this.form()?.format;
    return f && f.type === 'Csv' ? f : null;
  });
  protected readonly delimiters = [
    { value: ';', label: ';' },
    { value: ',', label: ',' },
    { value: '\t', label: this.t.tab },
    { value: '|', label: '|' },
  ];
  protected readonly encodings: CsvEncoding[] = ['Windows1252', 'Utf8'];
  protected readonly amountFormats = AMOUNT_FORMATS;
  protected readonly dateOptions = computed(() => dateFormatOptions(this.form()?.dateFormats[0]));
  protected readonly amountKey = computed(() => {
    const f = this.form();
    return f ? amountFormatKey(f.decimal) : null;
  });

  protected readonly options = computed<ColumnOption[]>(() => {
    const f = this.form();
    return f ? columnOptions(this.raw(), f) : [];
  });
  protected readonly roles = computed(() => {
    const f = this.form();
    if (!f) return [];
    const t = this.t;
    return columnRoles(this.options(), f, {
      bookingDate: t.bookingDate.replace(' *', ''),
      valueDate: t.valueDate,
      description: t.description.replace(' *', ''),
      counterparty: t.counterparty,
      amount: t.amountColumn.replace(' *', ''),
      debit: t.debit.replace(' *', ''),
      credit: t.credit.replace(' *', ''),
    });
  });
  protected readonly headerIndex = computed(() => {
    const f = this.form();
    const r = this.raw();
    return f && r ? headerRowIndex(r.rows, f) : -1;
  });
  protected readonly rawRows = computed(() => {
    const f = this.form();
    const r = this.raw();
    if (!f || !r) return [];
    const header = this.headerIndex();
    const skipUntil = f.hasHeader ? Math.max(header, 0) : (f.skipRows ?? 0);
    return r.rows.map((cells, i) => ({
      number: r.rowNumbers[i] ?? i + 1,
      cells,
      skipped: i < skipUntil,
      header: i === header,
    }));
  });
  protected readonly mainFields = computed<ColumnField[]>(() => [
    { key: 'bookingDate', label: this.t.bookingDate, required: true },
    { key: 'valueDate', label: this.t.valueDate, required: false },
    { key: 'description', label: this.t.description, required: true },
  ]);
  protected readonly amountFields = computed<ColumnField[]>(() => {
    const split = this.form()?.amountMode === 'split';
    return [
      ...(split
        ? [
            { key: 'debit', label: this.t.debit, required: true, hint: this.t.debitHint } as ColumnField,
            { key: 'credit', label: this.t.credit, required: true } as ColumnField,
          ]
        : [{ key: 'amount', label: this.t.amountColumn, required: true } as ColumnField]),
      { key: 'counterparty', label: this.t.counterparty, required: false },
    ];
  });

  protected readonly accountItems = computed(() =>
    this.accounts().map((a) => ({
      id: a.id,
      label: a.display_suffix ? `${a.name} · •••• ${a.display_suffix}` : a.name,
    })),
  );
  protected readonly selectedAccount = computed(() => {
    const id = this.selected();
    return typeof id === 'number' ? (this.accounts().find((a) => a.id === id) ?? null) : null;
  });
  protected readonly accountName = computed(() => this.selectedAccount()?.name ?? '');
  protected readonly identity = computed(() => this.preview()?.identity ?? null);
  protected readonly otherAccount = computed(() => {
    const i = this.identity();
    return i?.status === 'belongs_to_other'
      ? (this.accounts().find((a) => a.id === i.existingAccountId) ?? null)
      : null;
  });
  protected readonly identityBlocked = computed(() => {
    const s = this.identity()?.status;
    return s === 'mismatch' || s === 'belongs_to_other' || s === 'account_not_found';
  });
  protected readonly canPreview = computed(
    () =>
      this.busy() === null &&
      typeof this.selected() === 'number' &&
      this.preview()?.counts != null &&
      !this.identityBlocked(),
  );

  protected readonly counts = computed(() => this.preview()?.counts ?? null);
  protected readonly importCount = computed(() => {
    const c = this.counts();
    return c ? c.wouldInsert + c.updated : 0;
  });
  protected readonly invalidCount = computed(() => {
    const p = this.preview();
    return p ? p.parseErrorCount + (p.counts?.rejected ?? 0) : 0;
  });
  protected readonly range = computed(() => {
    const p = this.preview();
    return p?.dateFrom && p.dateTo ? this.t.kpiRange(formatDate(p.dateFrom), formatDate(p.dateTo)) : '';
  });
  protected readonly moreErrors = computed(() => {
    const p = this.preview();
    return p ? Math.max(p.parseErrorCount - p.parseErrors.length, 0) : 0;
  });
  protected readonly canCommit = computed(() => this.busy() === null && this.importCount() > 0);

  protected readonly date = formatDate;
  protected readonly dateTime = formatDateTime;
  protected readonly rowError = rowErrorText;
  protected readonly batchCounts = batchCounts;

  // ---------------------------------------------------------------- lifecycle

  ngOnInit(): void {
    void this.refreshHistory();
    this.listen(SYNCED_EVENT, () => void this.onSynced()).then(
      (off) => {
        if (this.destroyed) off();
        else this.unlisten = off;
      },
      () => {
        // Outside the desktop shell there are no events.
      },
    );
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    this.clearRawTimer();
    this.unlisten?.();
    this.unlisten = null;
    if (this.picked() !== null && this.step() !== 'start' && this.step() !== 'result') {
      void this.service.cancel().catch(() => undefined);
    }
  }

  // ------------------------------------------------------------------ helpers

  protected money(cents: number, currency: string): string {
    return formatCents(cents, currency);
  }

  protected signed(r: PreviewRow): string {
    return formatSignedCents(r.amountCents, r.currency);
  }

  protected statusText(r: PreviewRow): string {
    return r.status ? this.t.status[r.status] : '';
  }

  protected fileName(b: ImportBatch): string {
    return b.fileName ?? this.t.fileNone;
  }

  protected mask(a: AccountView): string | null {
    return a.display_suffix ? `•••• ${a.display_suffix}` : null;
  }

  protected clearError(): void {
    this.error.set(null);
  }

  protected clearNotice(): void {
    this.notice.set(null);
  }

  protected stepClass(s: { current: boolean; done: boolean }): string {
    return s.current ? 'is-current' : s.done ? 'is-done' : '';
  }

  private goTo(step: ImportStep): void {
    this.step.set(step);
    afterNextRender(() => this.heading()?.nativeElement.focus(), { injector: this.injector });
  }

  private fail(e: unknown): void {
    this.error.set(toCommandError(e));
  }

  private async onSynced(): Promise<void> {
    await this.refreshHistory();
    if (this.step() === 'account' && this.busy() === null) await this.refreshAccounts();
  }

  private async refreshHistory(): Promise<void> {
    try {
      this.history.set(await this.service.history(HISTORY_LIMIT));
    } catch {
      // Outside the desktop shell there is no history.
    } finally {
      this.historyLoaded.set(true);
    }
  }

  private async refreshAccounts(): Promise<void> {
    try {
      this.accounts.set(await this.service.accounts());
    } catch (e) {
      this.fail(e);
    }
  }

  // ------------------------------------------------------------- step 1: file

  protected async pick(): Promise<void> {
    if (this.busy() !== null) return;
    this.busy.set('pick');
    this.error.set(null);
    this.notice.set(null);
    try {
      const file = await this.service.pickFile();
      if (file) this.startFormat(file);
    } catch (e) {
      this.fail(e);
    } finally {
      this.busy.set(null);
    }
  }

  private startFormat(file: PickedImport): void {
    this.clearRawTimer();
    this.rawSeq++;
    this.picked.set(file);
    this.raw.set(null);
    this.rawError.set(null);
    this.formError.set(null);
    this.preview.set(null);
    this.selected.set(null);
    this.saveProfile.set(false);
    this.saveName.set('');
    this.activeProfile = null;
    const detected = this.detected();
    if (detected) {
      this.manual.set(false);
      this.profileSelect.set(detected.name);
      this.form.set(formFromProfile(detected.profile));
      void this.loadRaw(); // For the row count in the file header.
    } else {
      this.openManual(null);
    }
    this.goTo('format');
  }

  // ------------------------------------------------------------ step 2: format

  /** Opens the mapping form prefilled with `base`, or with the generic built-in profile. */
  private openManual(base: ImportProfile | null): void {
    const file = this.picked();
    if (!file) return;
    let profile = base;
    let name = this.detected()?.name ?? '';
    if (!profile) {
      const choices = this.profileChoices();
      const generic =
        choices.find((p) => p.builtin && p.name !== this.detected()?.name) ?? choices.find((p) => p.builtin) ?? choices[0];
      if (generic) {
        profile = { ...structuredClone(generic.profile), format: structuredClone(file.suggestedFormat) };
        name = generic.name;
        this.pruneNext = true;
      }
    }
    this.manual.set(true);
    this.profileSelect.set(name);
    if (profile) this.form.set(formFromProfile(profile));
    void this.loadRaw();
  }

  protected configureManually(): void {
    const d = this.detected();
    if (d) this.openManual(d.profile);
  }

  protected selectProfile(name: string): void {
    const p = this.profileChoices().find((x) => x.name === name);
    if (!p) return;
    this.profileSelect.set(name);
    this.form.set(formFromProfile(p.profile));
    this.formError.set(null);
    this.clearRawTimer();
    void this.loadRaw();
  }

  /** Applies a change to the form; reads the raw rows again when the file format changed. */
  private patch(change: Partial<MappingForm>): void {
    const before = this.form();
    if (!before) return;
    const after = { ...before, ...change };
    this.form.set(after);
    this.formError.set(null);
    if (formatKey(after.format) !== formatKey(before.format)) this.scheduleRaw();
  }

  protected setEncoding(value: string): void {
    const f = this.csv();
    if (f) this.patch({ format: { ...f, encoding: value as CsvEncoding } });
  }

  protected setDelimiter(value: string): void {
    const f = this.csv();
    if (f) this.patch({ format: { ...f, delimiter: value } });
  }

  protected setSkipRows(text: string): void {
    const trimmed = text.trim();
    const n = Number(trimmed);
    this.patch({ skipRows: trimmed === '' || !Number.isFinite(n) ? null : Math.max(0, Math.trunc(n)) });
  }

  protected setHasHeader(value: boolean): void {
    const f = this.form();
    if (f) this.form.set(convertRefs(f, this.raw(), value));
  }

  protected setDateFormat(pattern: string): void {
    const f = this.form();
    if (f) this.patch({ dateFormats: [pattern, ...f.dateFormats.slice(1).filter((x) => x !== pattern)] });
  }

  protected setAmountFormat(key: AmountFormatKey): void {
    this.patch({ decimal: decimalSpecFor(key) });
  }

  protected setAmountMode(mode: AmountMode): void {
    this.patch({ amountMode: mode });
  }

  protected setColumn(key: ColumnKey, value: string): void {
    const opt = this.options().find((o) => String(o.index) === value);
    this.patch({ [key]: opt ? opt.ref : null } as Partial<MappingForm>);
  }

  protected columnRef(key: ColumnKey): ColumnRef | null {
    return this.form()?.[key] ?? null;
  }

  /** Value of the select: the option that matches the current ref, `missing` when none does. */
  protected columnValue(key: ColumnKey): string {
    const ref = this.columnRef(key);
    if (!ref) return '';
    const hit = this.options().find((o) => sameRef(o.ref, ref));
    return hit ? String(hit.index) : 'missing';
  }

  protected missingLabel(key: ColumnKey): string {
    const ref = this.columnRef(key);
    if (!ref) return '';
    return this.t.columnMissing('Name' in ref ? ref.Name : `#${ref.Index + 1}`);
  }

  protected setSaveName(value: string): void {
    this.saveName.set(value);
    this.formError.set(null);
  }

  private clearRawTimer(): void {
    if (this.rawTimer !== null) clearTimeout(this.rawTimer);
    this.rawTimer = null;
  }

  private scheduleRaw(): void {
    this.clearRawTimer();
    this.rawLoading.set(true);
    this.rawTimer = setTimeout(() => {
      this.rawTimer = null;
      void this.loadRaw();
    }, RAW_DEBOUNCE_MS);
  }

  private async loadRaw(): Promise<void> {
    const f = this.form();
    if (!f) return;
    const seq = ++this.rawSeq;
    this.rawLoading.set(true);
    try {
      const table = await this.service.rawRows(f.format, RAW_ROWS);
      if (seq !== this.rawSeq) return;
      this.raw.set(table);
      this.rawError.set(null);
      if (this.pruneNext) {
        this.pruneNext = false;
        this.pruneUnresolved();
      }
    } catch (e) {
      if (seq !== this.rawSeq) return;
      this.raw.set(null);
      this.rawError.set(toCommandError(e));
    } finally {
      if (seq === this.rawSeq) this.rawLoading.set(false);
    }
  }

  /**
   * A generic prefill names columns the file may not have: fix the header row by a guess
   * (the automatic search needs the names) and leave unresolved columns to the user.
   */
  private pruneUnresolved(): void {
    const f = this.form();
    const raw = this.raw();
    if (!f || !raw) return;
    const base: MappingForm =
      f.hasHeader && f.skipRows === null && headerRowIndex(raw.rows, f) < 0
        ? { ...f, skipRows: guessHeaderRow(raw.rows) }
        : f;
    const opts = columnOptions(raw, base);
    const keep = (r: ColumnRef | null): ColumnRef | null => (r && opts.some((o) => sameRef(o.ref, r)) ? r : null);
    this.form.set({
      ...base,
      bookingDate: keep(base.bookingDate),
      valueDate: keep(base.valueDate),
      description: keep(base.description),
      counterparty: keep(base.counterparty),
      amount: keep(base.amount),
      debit: keep(base.debit),
      credit: keep(base.credit),
    });
  }

  /** Format step "Continuar": fixes the profile and moves on to the account. */
  protected async continueFromFormat(): Promise<void> {
    if (this.busy() !== null) return;
    let profile: ImportProfile | null;
    let name: string;
    if (!this.manual()) {
      const d = this.detected();
      if (!d) return;
      profile = d.profile;
      name = d.name;
    } else {
      const f = this.form();
      profile = f ? profileFromForm(f) : null;
      if (!profile) {
        this.formError.set(f && f.dateFormats.length === 0 ? this.t.formErrors.date : this.t.formErrors.columns);
        return;
      }
      const save = this.saveProfile() ? this.saveName().replace(/\s+/g, ' ').trim() : '';
      if (this.saveProfile() && (save === '' || save.length > MAX_PROFILE_NAME)) {
        this.formError.set(this.t.formErrors.profileName);
        return;
      }
      name = save || this.t.manualProfileName;
    }
    this.busy.set('preview');
    this.error.set(null);
    try {
      const [preview, accounts] = await Promise.all([this.service.preview(profile, null), this.service.accounts()]);
      this.activeProfile = profile;
      this.activeProfileName = name;
      this.accounts.set(accounts);
      this.preview.set(preview);
      this.newName.set('');
      this.newNameError.set(null);
      const matched = preview.matchedAccountId;
      this.selected.set(matched !== null && accounts.some((a) => a.id === matched) ? matched : null);
      this.goTo('account');
      if (typeof this.selected() === 'number') await this.runPreview(this.selected() as number, true);
    } catch (e) {
      this.fail(e);
    } finally {
      this.busy.set(null);
    }
  }

  // ------------------------------------------------------------ step 3: account

  protected async onAccount(value: string): Promise<void> {
    if (value === 'new') {
      this.selected.set('new');
      this.preview.update((p) => (p ? { ...p, identity: null, counts: null } : p));
      return;
    }
    if (value === '') {
      this.selected.set(null);
      return;
    }
    await this.runPreview(Number(value));
  }

  protected async useAccount(id: number): Promise<void> {
    await this.runPreview(id);
  }

  protected setNewName(value: string): void {
    this.newName.set(value);
    this.newNameError.set(null);
  }

  protected async createAccount(): Promise<void> {
    const name = this.newName().replace(/\s+/g, ' ').trim();
    if (!name) {
      this.newNameError.set(this.t.newAccountNameRequired);
      return;
    }
    if (this.busy() !== null) return;
    this.busy.set('account');
    this.error.set(null);
    try {
      const account = await this.service.createAccount(name);
      this.accounts.set(await this.service.accounts());
      this.newName.set('');
      await this.runPreview(account.id, true);
    } catch (e) {
      this.fail(e);
    } finally {
      this.busy.set(null);
    }
  }

  /** Dry run for `accountId`; the identity check and counts come back with it. */
  private async runPreview(accountId: number, keepBusy = false): Promise<void> {
    const profile = this.activeProfile;
    if (!profile) return;
    const seq = ++this.previewSeq;
    this.selected.set(accountId);
    if (!keepBusy) this.busy.set('preview');
    this.error.set(null);
    try {
      const preview = await this.service.preview(profile, accountId);
      if (seq === this.previewSeq) this.preview.set(preview);
    } catch (e) {
      if (seq === this.previewSeq) {
        this.preview.update((p) => (p ? { ...p, identity: null, counts: null } : p));
        this.fail(e);
      }
    } finally {
      if (!keepBusy && seq === this.previewSeq) this.busy.set(null);
    }
  }

  protected toReview(): void {
    if (this.canPreview()) this.goTo('review');
  }

  // ------------------------------------------------------------- step 4 and 5

  protected async commit(): Promise<void> {
    const profile = this.activeProfile;
    const account = this.selected();
    if (!profile || typeof account !== 'number' || !this.canCommit()) return;
    this.busy.set('commit');
    this.error.set(null);
    try {
      const save = this.manual() && this.saveProfile() ? this.saveName().replace(/\s+/g, ' ').trim() : '';
      const r = await this.service.commit(profile, this.activeProfileName, account, save || null);
      this.result.set(r);
      this.goTo('result');
      void this.refreshHistory();
    } catch (e) {
      this.fail(e);
    } finally {
      this.busy.set(null);
    }
  }

  protected resultAccountName(): string {
    const id = this.result()?.accountId;
    return this.accounts().find((a) => a.id === id)?.name ?? this.accountName();
  }

  // ---------------------------------------------------------------- navigation

  protected back(): void {
    const s = this.step();
    if (s === 'account') this.goTo('format');
    else if (s === 'review') this.goTo('account');
  }

  /** Drops the stashed file and returns to the start view. */
  protected async cancel(): Promise<void> {
    try {
      await this.service.cancel();
    } catch {
      // Nothing to drop.
    }
    this.reset();
  }

  /** "Importar otro archivo" after a finished import. */
  protected anotherFile(): void {
    this.reset();
  }

  private reset(): void {
    this.clearRawTimer();
    this.rawSeq++;
    this.previewSeq++;
    this.picked.set(null);
    this.form.set(null);
    this.raw.set(null);
    this.preview.set(null);
    this.result.set(null);
    this.selected.set(null);
    this.manual.set(false);
    this.error.set(null);
    this.formError.set(null);
    this.activeProfile = null;
    this.goTo('start');
    void this.refreshHistory();
  }

  // ---------------------------------------------------------------------- undo

  protected askUndoBatch(b: ImportBatch): void {
    this.undoTarget.set({
      batchId: b.id,
      fileName: this.fileName(b),
      accountName: b.accountName,
      inserted: b.inserted,
      from: 'history',
    });
  }

  protected askUndoResult(): void {
    const r = this.result();
    if (!r) return;
    this.undoTarget.set({
      batchId: r.batchId,
      fileName: this.picked()?.fileName ?? this.t.fileNone,
      accountName: this.resultAccountName(),
      inserted: r.inserted,
      from: 'result',
    });
  }

  protected closeUndo(): void {
    this.undoTarget.set(null);
  }

  protected async confirmUndo(): Promise<void> {
    const target = this.undoTarget();
    if (!target || this.busy() !== null) return;
    this.undoTarget.set(null);
    this.busy.set('undo');
    this.error.set(null);
    this.notice.set(null);
    try {
      const report = await this.service.undo(target.batchId);
      this.notice.set(this.t.undoDone(report.deleted));
      if (target.from === 'result') this.reset();
    } catch (e) {
      this.fail(e);
    } finally {
      this.busy.set(null);
    }
    await this.refreshHistory();
  }
}
