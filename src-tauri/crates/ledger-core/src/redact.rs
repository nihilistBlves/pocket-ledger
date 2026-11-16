//! Last-line redaction for log output (ADR 0003).
//!
//! Log calls must only carry endpoint templates, statuses, durations and codes. This
//! is the safety net behind that rule: the app's logger passes every formatted line
//! through [`redact`], which masks anything that looks like an IBAN, a JWT, a bearer
//! token, a UUID (Enable Banking session ids and account uids) or a long digit run
//! (card or account numbers). Over-redaction is acceptable; leaks are not.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

/// Pattern and replacement, applied in order (JWT before UUID before IBAN, so a
/// token is masked as a whole rather than piecemeal).
static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        // "Bearer <anything>" in a header dump.
        (r"(?i)\bbearer\s+[^\s,;]+", "Bearer [redacted]"),
        // Three base64url segments starting with a JSON header ("eyJ" = `{"`).
        (r"\beyJ[A-Za-z0-9_-]{2,}\.[A-Za-z0-9_-]{2,}\.[A-Za-z0-9_-]*", "[jwt]"),
        (r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b", "[uuid]"),
        // Two letters, two digits, 11-30 alphanumerics: compact, or printed in groups
        // of 4 separated by single spaces (a short last group may swallow a word of
        // up to 3 characters right after the IBAN; acceptable over-redaction).
        (
            r"(?i)\b[a-z]{2}[0-9]{2}(?:[a-z0-9]{11,30}|(?: [a-z0-9]{4}){3,7}(?: [a-z0-9]{1,3})?)\b",
            "[iban]",
        ),
        (r"\b[0-9]{13,}\b", "[number]"),
    ]
    .into_iter()
    .map(|(pattern, replacement)| (Regex::new(pattern).expect("valid redaction regex"), replacement))
    .collect()
});

/// `line` with every sensitive-looking value masked. Borrows when nothing matched.
pub fn redact(line: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(line);
    for (re, replacement) in RULES.iter() {
        if let Cow::Owned(changed) = re.replace_all(&out, *replacement) {
            out = Cow::Owned(changed);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_log_lines_pass_unchanged() {
        for line in [
            "GET /accounts/{uid}/transactions -> 200 (attempt 1, 412 ms)",
            "POST /sessions -> network error timeout (attempt 2, 30000 ms)",
            "automatic bank sync failed: session_expired",
            "stored bank session is unreadable; treating it as not connected",
            "sync finished: 3 accounts, 120 inserted, 4 duplicates",
        ] {
            assert!(matches!(redact(line), Cow::Borrowed(_)), "{line}");
        }
    }

    #[test]
    fn ibans_are_masked_with_or_without_spaces() {
        let fake = "ES00 0000 0000 0000 0000 0001";
        assert_eq!(redact(&format!("account {fake} synced")), "account [iban] synced");
        assert_eq!(redact("iban=es0000000000000000000001;"), "iban=[iban];");
        assert_eq!(redact("DE99123456789012345678"), "[iban]");
        assert_eq!(redact("DE99 1234 5678 9012 3456 78 ok"), "[iban] ok");
        assert_eq!(redact(&format!("{fake}.")), "[iban].");
    }

    #[test]
    fn jwts_and_bearer_tokens_are_masked() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJpc3MiOiJlbmFibGViYW5raW5nLmNvbSJ9.c2lnbmF0dXJl";
        assert_eq!(redact(&format!("token {jwt} end")), "token [jwt] end");
        assert_eq!(
            redact(&format!("authorization: Bearer {jwt}")),
            "authorization: Bearer [redacted]"
        );
        assert_eq!(redact("Authorization: bearer abc.def"), "Authorization: Bearer [redacted]");
    }

    #[test]
    fn uuid_session_ids_are_masked() {
        assert_eq!(
            redact("DELETE /sessions/0f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b failed"),
            "DELETE /sessions/[uuid] failed"
        );
        assert_eq!(redact("uid=0F1E2D3C-4B5A-6978-8A9B-0C1D2E3F4A5B"), "uid=[uuid]");
    }

    #[test]
    fn long_digit_runs_are_masked() {
        assert_eq!(redact("card 4000000000000002 used"), "card [number] used");
        assert_eq!(redact("took 30000 ms"), "took 30000 ms");
    }
}
