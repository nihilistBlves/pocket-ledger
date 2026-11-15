//! Bank connectivity (open banking): the [`enable_banking`] client, the loopback
//! [`callback`] listener, the authorization flow ([`connect`]) and the incremental
//! [`sync`].
//!
//! Every operation is split into an async network part that returns plain data and a
//! synchronous database part taking `&mut Connection`, so a connection is never held
//! across an `.await`.

pub mod callback;
pub mod connect;
pub mod enable_banking;
pub mod overview;
pub mod sync;

#[cfg(test)]
mod test_support;

/// Timestamp format of the `*_at` columns (UTC, millisecond precision, same shape as
/// SQLite's `strftime('%Y-%m-%dT%H:%M:%fZ')`), so text comparison orders correctly.
pub(crate) fn db_timestamp(t: chrono::DateTime<chrono::Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}
