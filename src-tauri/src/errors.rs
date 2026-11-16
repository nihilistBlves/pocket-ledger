//! The single error type returned by every bank command: a fixed code and a fixed
//! Spanish message. Underlying errors are mapped by variant only, never by their
//! `Display` text, so paths, URLs, identifiers or response content can not reach the UI.

use ledger_core::backup::BackupError;
use ledger_core::bank::connect::{AuthError, ConnectError};
use ledger_core::bank::enable_banking::EbError;
use ledger_core::bank::sync::{SyncBlocked, SyncError};
use ledger_core::keystore::KeystoreError;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NotConfigured,
    KeyMissing,
    PortInUse,
    Timeout,
    Denied,
    StateMismatch,
    NotConnected,
    ReauthRequired,
    RateLimited,
    BlockedTooSoon,
    Busy,
    BankError,
    InvalidInput,
    NotFound,
    Conflict,
    Internal,
    RecoveryMode,
    PassphraseMismatch,
    WeakPassphrase,
    WrongPassphrase,
    NotABackup,
    UnsupportedBackup,
    NoBackupSelected,
    BackupKeyMismatch,
    RestoreRolledBack,
    RestoreFailed,
    BackupIo,
}

impl ErrorCode {
    #[cfg(test)]
    pub const ALL: [ErrorCode; 27] = [
        ErrorCode::NotConfigured,
        ErrorCode::KeyMissing,
        ErrorCode::PortInUse,
        ErrorCode::Timeout,
        ErrorCode::Denied,
        ErrorCode::StateMismatch,
        ErrorCode::NotConnected,
        ErrorCode::ReauthRequired,
        ErrorCode::RateLimited,
        ErrorCode::BlockedTooSoon,
        ErrorCode::Busy,
        ErrorCode::BankError,
        ErrorCode::InvalidInput,
        ErrorCode::NotFound,
        ErrorCode::Conflict,
        ErrorCode::Internal,
        ErrorCode::RecoveryMode,
        ErrorCode::PassphraseMismatch,
        ErrorCode::WeakPassphrase,
        ErrorCode::WrongPassphrase,
        ErrorCode::NotABackup,
        ErrorCode::UnsupportedBackup,
        ErrorCode::NoBackupSelected,
        ErrorCode::BackupKeyMismatch,
        ErrorCode::RestoreRolledBack,
        ErrorCode::RestoreFailed,
        ErrorCode::BackupIo,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::NotConfigured => "not_configured",
            ErrorCode::KeyMissing => "key_missing",
            ErrorCode::PortInUse => "port_in_use",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Denied => "denied",
            ErrorCode::StateMismatch => "state_mismatch",
            ErrorCode::NotConnected => "not_connected",
            ErrorCode::ReauthRequired => "reauth_required",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::BlockedTooSoon => "blocked_too_soon",
            ErrorCode::Busy => "busy",
            ErrorCode::BankError => "bank_error",
            ErrorCode::InvalidInput => "invalid_input",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Conflict => "conflict",
            ErrorCode::Internal => "internal",
            ErrorCode::RecoveryMode => "recovery_mode",
            ErrorCode::PassphraseMismatch => "passphrase_mismatch",
            ErrorCode::WeakPassphrase => "weak_passphrase",
            ErrorCode::WrongPassphrase => "wrong_passphrase",
            ErrorCode::NotABackup => "not_a_backup",
            ErrorCode::UnsupportedBackup => "unsupported_backup",
            ErrorCode::NoBackupSelected => "no_backup_selected",
            ErrorCode::BackupKeyMismatch => "backup_key_mismatch",
            ErrorCode::RestoreRolledBack => "restore_rolled_back",
            ErrorCode::RestoreFailed => "restore_failed",
            ErrorCode::BackupIo => "backup_io",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            ErrorCode::NotConfigured => {
                "La conexión bancaria no está configurada. Falta el archivo enable-banking.json en la carpeta de datos de la aplicación o su contenido no es válido."
            }
            ErrorCode::KeyMissing => {
                "No se encuentra la clave privada de Enable Banking. Comprueba la variable de entorno ENABLE_BANKING_KEY_PATH y que el archivo existe."
            }
            ErrorCode::PortInUse => {
                "El puerto 8080 está ocupado por otra aplicación. Ciérrala e inténtalo de nuevo."
            }
            ErrorCode::Timeout => {
                "Se agotó el tiempo de espera de la autorización (5 minutos). Vuelve a intentarlo."
            }
            ErrorCode::Denied => "La autorización fue rechazada o cancelada en el banco.",
            ErrorCode::StateMismatch => {
                "La respuesta de autorización no corresponde a esta solicitud. Vuelve a intentarlo."
            }
            ErrorCode::NotConnected => "No hay una conexión activa con el banco.",
            ErrorCode::ReauthRequired => {
                "El consentimiento con el banco ya no es válido. Renuévalo para seguir sincronizando."
            }
            ErrorCode::RateLimited => {
                "El banco rechazó la petición por exceso de solicitudes. Inténtalo de nuevo más tarde."
            }
            ErrorCode::BlockedTooSoon => {
                "Se sincronizó hace muy poco. Espera un momento antes de volver a intentarlo."
            }
            ErrorCode::Busy => "Ya hay otra operación bancaria en curso. Espera a que termine.",
            ErrorCode::BankError => {
                "El banco o Enable Banking devolvió un error. Inténtalo de nuevo más tarde."
            }
            ErrorCode::InvalidInput => "Los datos enviados no son válidos. Revisa los valores e inténtalo de nuevo.",
            ErrorCode::NotFound => "El movimiento o la categoría ya no existe.",
            ErrorCode::Conflict => "La operación entra en conflicto con datos existentes.",
            ErrorCode::Internal => "Error interno de la aplicación. Inténtalo de nuevo.",
            ErrorCode::RecoveryMode => {
                "La aplicación está en modo de recuperación. Restaura o usa una copia de seguridad para continuar."
            }
            ErrorCode::PassphraseMismatch => "Las dos frases de paso no coinciden.",
            ErrorCode::WeakPassphrase => {
                "La frase de paso es demasiado débil. Usa al menos 12 caracteres, por ejemplo 4 o 5 palabras al azar."
            }
            ErrorCode::WrongPassphrase => {
                "La frase de paso no es correcta o el archivo está dañado."
            }
            ErrorCode::NotABackup => "El archivo no es una copia de seguridad de Pocket Ledger.",
            ErrorCode::UnsupportedBackup => {
                "La copia usa un formato desconocido o fue creada con una versión más nueva de la aplicación."
            }
            ErrorCode::NoBackupSelected => "Primero elige el archivo de la copia.",
            ErrorCode::BackupKeyMismatch => {
                "La clave de esta copia no abre la base de datos actual. Usa «Restaurar una copia» en su lugar."
            }
            ErrorCode::RestoreRolledBack => {
                "No se pudo completar la restauración; tus datos actuales siguen intactos."
            }
            ErrorCode::RestoreFailed => {
                "La restauración falló y no se pudo recuperar la base de datos anterior. Cierra y vuelve a abrir la aplicación; en la carpeta de datos se conserva una copia llamada pocket-ledger.db.pre-restore-… con tus datos anteriores."
            }
            ErrorCode::BackupIo => "No se pudo leer o escribir el archivo de la copia.",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub fn new(code: ErrorCode) -> Self {
        Self { code: code.as_str().to_owned(), message: code.message().to_owned() }
    }
}

impl From<ErrorCode> for CommandError {
    fn from(code: ErrorCode) -> Self {
        Self::new(code)
    }
}

/// Maps by variant only: no path, key or passphrase can reach the UI.
pub fn code_of_backup(e: &BackupError) -> ErrorCode {
    match e {
        BackupError::NotABackup | BackupError::InvalidHeader | BackupError::TooLarge => {
            ErrorCode::NotABackup
        }
        BackupError::UnsupportedVersion(_) | BackupError::NewerSchema => {
            ErrorCode::UnsupportedBackup
        }
        BackupError::WrongPassphraseOrCorrupt => ErrorCode::WrongPassphrase,
        BackupError::WeakPassphrase => ErrorCode::WeakPassphrase,
        BackupError::BackupKeyDoesNotOpenDatabase => ErrorCode::BackupKeyMismatch,
        BackupError::RestoreRolledBack => ErrorCode::RestoreRolledBack,
        BackupError::RestoreFailedAndRollbackFailed => ErrorCode::RestoreFailed,
        BackupError::IntegrityCheckFailed | BackupError::Io(_) => ErrorCode::BackupIo,
        BackupError::Db(_) | BackupError::Keystore(_) | BackupError::Random => ErrorCode::Internal,
    }
}

pub fn code_of_eb(e: &EbError) -> ErrorCode {
    match e {
        EbError::KeyNotConfigured | EbError::KeyFileNotFound | EbError::KeyInvalid => {
            ErrorCode::KeyMissing
        }
        EbError::InvalidConfig => ErrorCode::NotConfigured,
        _ if e.requires_reauthorization() => ErrorCode::ReauthRequired,
        _ if e.is_rate_limited() => ErrorCode::RateLimited,
        _ => ErrorCode::BankError,
    }
}

pub fn code_of_auth(e: &AuthError) -> ErrorCode {
    match e {
        AuthError::PortInUse => ErrorCode::PortInUse,
        AuthError::Bind | AuthError::Random | AuthError::BrowserOpen => ErrorCode::Internal,
        AuthError::AspspNotFound | AuthError::MissingCode => ErrorCode::BankError,
        AuthError::Denied { .. } => ErrorCode::Denied,
        AuthError::StateMismatch => ErrorCode::StateMismatch,
        AuthError::Timeout => ErrorCode::Timeout,
        AuthError::Api(e) => code_of_eb(e),
    }
}

pub fn code_of_sync(e: &SyncError) -> ErrorCode {
    match e {
        SyncError::NotConnected => ErrorCode::NotConnected,
        SyncError::Blocked(SyncBlocked::AlreadyRunning) => ErrorCode::Busy,
        SyncError::Blocked(_) => ErrorCode::BlockedTooSoon,
        SyncError::ReauthRequired => ErrorCode::ReauthRequired,
        SyncError::RateLimited => ErrorCode::RateLimited,
        SyncError::Bank { .. } => ErrorCode::BankError,
        SyncError::Db(_) | SyncError::Store(_) | SyncError::Poisoned => ErrorCode::Internal,
    }
}

impl From<BackupError> for CommandError {
    fn from(e: BackupError) -> Self {
        Self::new(code_of_backup(&e))
    }
}

impl From<AuthError> for CommandError {
    fn from(e: AuthError) -> Self {
        Self::new(code_of_auth(&e))
    }
}

impl From<SyncError> for CommandError {
    fn from(e: SyncError) -> Self {
        Self::new(code_of_sync(&e))
    }
}

impl From<EbError> for CommandError {
    fn from(e: EbError) -> Self {
        Self::new(code_of_eb(&e))
    }
}

impl From<ConnectError> for CommandError {
    fn from(_: ConnectError) -> Self {
        Self::new(ErrorCode::Internal)
    }
}

impl From<KeystoreError> for CommandError {
    fn from(_: KeystoreError) -> Self {
        Self::new(ErrorCode::Internal)
    }
}

impl From<ledger_core::ledger::LedgerError> for CommandError {
    fn from(e: ledger_core::ledger::LedgerError) -> Self {
        use ledger_core::ledger::LedgerError;
        Self::new(match e {
            LedgerError::InvalidInput(_) => ErrorCode::InvalidInput,
            LedgerError::NotFound(_) => ErrorCode::NotFound,
            LedgerError::Conflict(_) => ErrorCode::Conflict,
            LedgerError::Db(_) => ErrorCode::Internal,
        })
    }
}

impl From<rusqlite::Error> for CommandError {
    fn from(_: rusqlite::Error) -> Self {
        Self::new(ErrorCode::Internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::bank::enable_banking::{ApiErrorCode, NetworkKind, KEY_PATH_ENV};

    #[test]
    fn every_code_has_a_unique_snake_case_code_and_a_spanish_message() {
        let mut seen = std::collections::HashSet::new();
        for c in ErrorCode::ALL {
            assert!(seen.insert(c.as_str()));
            assert!(c.as_str().chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
            assert!(c.message().ends_with('.'));
        }
        assert!(ErrorCode::KeyMissing.message().contains(KEY_PATH_ENV));
        assert!(ErrorCode::NotConfigured.message().contains("enable-banking.json"));
    }

    #[test]
    fn backup_errors_map_by_variant() {
        use ledger_core::db::DbError;
        use ledger_core::keystore::KeystoreError;
        let cases: Vec<(BackupError, ErrorCode)> = vec![
            (BackupError::NotABackup, ErrorCode::NotABackup),
            (BackupError::InvalidHeader, ErrorCode::NotABackup),
            (BackupError::TooLarge, ErrorCode::NotABackup),
            (BackupError::UnsupportedVersion(9), ErrorCode::UnsupportedBackup),
            (BackupError::NewerSchema, ErrorCode::UnsupportedBackup),
            (BackupError::WrongPassphraseOrCorrupt, ErrorCode::WrongPassphrase),
            (BackupError::WeakPassphrase, ErrorCode::WeakPassphrase),
            (BackupError::BackupKeyDoesNotOpenDatabase, ErrorCode::BackupKeyMismatch),
            (BackupError::RestoreRolledBack, ErrorCode::RestoreRolledBack),
            (BackupError::RestoreFailedAndRollbackFailed, ErrorCode::RestoreFailed),
            (BackupError::IntegrityCheckFailed, ErrorCode::BackupIo),
            (BackupError::Io(std::io::ErrorKind::NotFound), ErrorCode::BackupIo),
            (BackupError::Db(DbError::WrongKeyOrCorrupt), ErrorCode::Internal),
            (BackupError::Keystore(KeystoreError::Poisoned), ErrorCode::Internal),
            (BackupError::Random, ErrorCode::Internal),
        ];
        for (err, code) in cases {
            assert_eq!(code_of_backup(&err), code, "{err:?}");
        }
        assert_eq!(
            ErrorCode::WrongPassphrase.message(),
            "La frase de paso no es correcta o el archivo está dañado."
        );
        assert!(ErrorCode::RestoreFailed.message().contains("pocket-ledger.db.pre-restore-"));
    }

    #[test]
    fn serializes_as_code_and_message_only() {
        let v = serde_json::to_value(CommandError::new(ErrorCode::Busy)).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 2);
        assert_eq!(v["code"], "busy");
    }

    #[test]
    fn eb_errors_map_by_variant() {
        assert_eq!(code_of_eb(&EbError::KeyNotConfigured), ErrorCode::KeyMissing);
        assert_eq!(code_of_eb(&EbError::KeyFileNotFound), ErrorCode::KeyMissing);
        assert_eq!(code_of_eb(&EbError::KeyInvalid), ErrorCode::KeyMissing);
        assert_eq!(code_of_eb(&EbError::Network { kind: NetworkKind::Timeout }), ErrorCode::BankError);
        assert_eq!(
            code_of_eb(&EbError::Api { status: 429, code: ApiErrorCode::AspspRateLimitExceeded }),
            ErrorCode::RateLimited
        );
        assert_eq!(
            code_of_eb(&EbError::Api { status: 401, code: ApiErrorCode::ExpiredSession }),
            ErrorCode::ReauthRequired
        );
    }

    #[test]
    fn auth_errors_map_by_variant() {
        assert_eq!(code_of_auth(&AuthError::PortInUse), ErrorCode::PortInUse);
        assert_eq!(code_of_auth(&AuthError::Timeout), ErrorCode::Timeout);
        assert_eq!(code_of_auth(&AuthError::StateMismatch), ErrorCode::StateMismatch);
        assert_eq!(code_of_auth(&AuthError::Denied { code: "x".into() }), ErrorCode::Denied);
        assert_eq!(code_of_auth(&AuthError::Api(EbError::KeyInvalid)), ErrorCode::KeyMissing);
    }

    #[test]
    fn sync_errors_map_by_variant_and_hide_codes() {
        assert_eq!(code_of_sync(&SyncError::NotConnected), ErrorCode::NotConnected);
        assert_eq!(code_of_sync(&SyncBlocked::AlreadyRunning.into()), ErrorCode::Busy);
        assert_eq!(code_of_sync(&SyncBlocked::AutoTooSoon.into()), ErrorCode::BlockedTooSoon);
        assert_eq!(code_of_sync(&SyncBlocked::AutoDailyLimit.into()), ErrorCode::BlockedTooSoon);
        assert_eq!(code_of_sync(&SyncBlocked::ManualTooSoon.into()), ErrorCode::BlockedTooSoon);
        assert_eq!(code_of_sync(&SyncError::ReauthRequired), ErrorCode::ReauthRequired);
        assert_eq!(code_of_sync(&SyncError::RateLimited), ErrorCode::RateLimited);
        let e: CommandError = SyncError::Bank { code: "SECRET-LOOKING".into() }.into();
        assert_eq!(e.code, "bank_error");
        assert!(!e.message.contains("SECRET"));
        assert_eq!(code_of_sync(&SyncError::Poisoned), ErrorCode::Internal);
    }

    #[test]
    fn ledger_conflict_maps_to_the_conflict_code() {
        use ledger_core::ledger::LedgerError;
        let e: CommandError = LedgerError::Conflict("duplicate").into();
        assert_eq!(e.code, "conflict");
        assert!(!e.message.contains("duplicate"));
    }
}
