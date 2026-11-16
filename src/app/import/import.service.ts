import { Injectable, inject } from '@angular/core';
import { AccountView } from '../bank/bank.models';
import { TAURI_INVOKE } from '../core/tauri';
import {
  FileFormat,
  ImportBatch,
  ImportCommitResult,
  ImportPreview,
  ImportProfile,
  PickedImport,
  RawTable,
  UndoReport,
} from './import.models';

/** Wraps the `import_*` Tauri commands. The WebView never handles a file path: Rust opens the dialog. */
@Injectable({ providedIn: 'root' })
export class ImportService {
  private readonly invoke = inject(TAURI_INVOKE);

  /** `null` when the dialog was cancelled (the previous pick stays). */
  pickFile(): Promise<PickedImport | null> {
    return this.invoke<PickedImport | null>('import_pick_file');
  }

  rawRows(format: FileFormat, maxRows: number): Promise<RawTable> {
    return this.invoke<RawTable>('import_raw_rows', { format, maxRows });
  }

  preview(profile: ImportProfile, accountId: number | null): Promise<ImportPreview> {
    return this.invoke<ImportPreview>('import_preview', { profile, accountId });
  }

  commit(
    profile: ImportProfile,
    profileName: string,
    accountId: number,
    saveProfileAs: string | null,
  ): Promise<ImportCommitResult> {
    return this.invoke<ImportCommitResult>('import_commit', { profile, profileName, accountId, saveProfileAs });
  }

  cancel(): Promise<void> {
    return this.invoke<void>('import_cancel');
  }

  history(limit: number): Promise<ImportBatch[]> {
    return this.invoke<ImportBatch[]>('import_history', { limit });
  }

  undo(batchId: number): Promise<UndoReport> {
    return this.invoke<UndoReport>('import_undo', { batchId });
  }

  accounts(): Promise<AccountView[]> {
    return this.invoke<AccountView[]>('import_accounts');
  }

  createAccount(name: string): Promise<AccountView> {
    return this.invoke<AccountView>('ledger_create_account', { name });
  }
}
