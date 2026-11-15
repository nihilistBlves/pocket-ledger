//! Loopback listener that receives the authorization redirect (see ADR 0004).
//!
//! The registered HTTPS bridge page forwards the browser to
//! `http://127.0.0.1:8080/callback?code=...&state=...`. This module binds that
//! address (IPv4 loopback only, never `0.0.0.0` or `localhost`), accepts a single
//! `/callback` request, checks `state` in constant time and hands the `code` back.
//!
//! Security notes: the query string, `code` and `state` are never logged, printed or
//! put in an error. Everything but `GET /callback` gets a 404 and the listener keeps
//! waiting (browsers also request `/favicon.ico`). A `state` mismatch aborts the flow.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::connect::AuthError;
use super::enable_banking::{AuthCode, State};

/// Fixed port: the bridge page hard-codes it.
pub const DEFAULT_PORT: u16 = 8080;
/// Overall time the user has to complete the authorization at the bank.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

const MAX_HEAD_BYTES: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_ERROR_CODE_LEN: usize = 64;

const OK_PAGE: &str = "<!doctype html><html lang=\"es\"><head><meta charset=\"utf-8\">\
<title>Pocket Ledger</title><style>body{font-family:sans-serif;margin:3rem auto;max-width:32rem;\
text-align:center}</style></head><body><p>Puedes cerrar esta pesta\u{f1}a y volver a Pocket Ledger.</p>\
</body></html>";

const ERROR_PAGE: &str = "<!doctype html><html lang=\"es\"><head><meta charset=\"utf-8\">\
<title>Pocket Ledger</title><style>body{font-family:sans-serif;margin:3rem auto;max-width:32rem;\
text-align:center}</style></head><body><p>No se ha podido completar la autorizaci\u{f3}n. \
Cierra esta pesta\u{f1}a y vuelve a intentarlo desde Pocket Ledger.</p></body></html>";

const NOT_FOUND_PAGE: &str = "<!doctype html><html lang=\"es\"><head><meta charset=\"utf-8\">\
<title>Pocket Ledger</title></head><body><p>No encontrado.</p></body></html>";

/// A bound loopback listener. Binding happens before the browser is opened so that a
/// busy port fails fast.
#[derive(Debug)]
pub struct CallbackListener {
    listener: TcpListener,
}

impl CallbackListener {
    /// Binds `127.0.0.1:port`. The port is a parameter only so tests can use 0.
    pub async fn bind(port: u16) -> Result<Self, AuthError> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        match TcpListener::bind(addr).await {
            Ok(listener) => Ok(Self { listener }),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                Err(AuthError::PortInUse)
            }
            Err(_) => Err(AuthError::Bind),
        }
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.local_addr().ok()
    }

    pub fn port(&self) -> u16 {
        self.local_addr().map_or(0, |a| a.port())
    }

    /// Waits for the first `/callback` request and returns its authorization code.
    /// Consumes the listener: it is single use and closes on return.
    pub async fn wait_for_code(
        self,
        expected: &State,
        timeout: Duration,
    ) -> Result<AuthCode, AuthError> {
        match tokio::time::timeout(timeout, self.serve(expected)).await {
            Ok(result) => result,
            Err(_) => Err(AuthError::Timeout),
        }
    }

    async fn serve(self, expected: &State) -> Result<AuthCode, AuthError> {
        let (tx, mut rx) = mpsc::channel::<Result<AuthCode, AuthError>>(1);
        let claimed = Arc::new(AtomicBool::new(false));
        let expected = Arc::new(expected.clone());
        loop {
            tokio::select! {
                accepted = self.listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        tokio::spawn(handle(
                            stream,
                            Arc::clone(&expected),
                            Arc::clone(&claimed),
                            tx.clone(),
                        ));
                    }
                    // Transient accept failure (e.g. out of handles): back off, keep waiting.
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                },
                result = rx.recv() => {
                    return result.unwrap_or(Err(AuthError::Timeout));
                }
            }
        }
    }
}

enum Head {
    Complete(Vec<u8>),
    TooLarge,
    /// Timed out, connection closed or I/O error: nothing to answer.
    Abandoned,
}

enum Parsed {
    BadRequest,
    NotFound,
    /// Raw query string of `GET /callback`.
    Callback(String),
}

async fn handle(
    mut stream: TcpStream,
    expected: Arc<State>,
    claimed: Arc<AtomicBool>,
    tx: mpsc::Sender<Result<AuthCode, AuthError>>,
) {
    let head = match read_head(&mut stream).await {
        Head::Complete(head) => head,
        Head::TooLarge => return respond(&mut stream, 400, "Bad Request", ERROR_PAGE).await,
        Head::Abandoned => return,
    };
    match parse_request(&head) {
        Parsed::BadRequest => respond(&mut stream, 400, "Bad Request", ERROR_PAGE).await,
        Parsed::NotFound => respond(&mut stream, 404, "Not Found", NOT_FOUND_PAGE).await,
        Parsed::Callback(query) => {
            // Single use: only the first /callback is evaluated.
            if claimed.swap(true, Ordering::SeqCst) {
                return respond(&mut stream, 404, "Not Found", NOT_FOUND_PAGE).await;
            }
            let outcome = evaluate(&query, &expected);
            match &outcome {
                Ok(_) => respond(&mut stream, 200, "OK", OK_PAGE).await,
                Err(AuthError::Denied { .. }) => respond(&mut stream, 200, "OK", ERROR_PAGE).await,
                Err(_) => respond(&mut stream, 400, "Bad Request", ERROR_PAGE).await,
            }
            // The response is on the wire before the flow learns the outcome.
            let _ = tx.send(outcome).await;
        }
    }
}

/// Reads up to the end of the request head (`\r\n\r\n`), at most 8 KiB, within 5 s.
async fn read_head(stream: &mut TcpStream) -> Head {
    let read = async {
        let mut buf: Vec<u8> = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return Head::Abandoned,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
            let window = &buf[..buf.len().min(MAX_HEAD_BYTES + 3)];
            if let Some(pos) = window.windows(4).position(|w| w == b"\r\n\r\n") {
                if pos + 4 <= MAX_HEAD_BYTES {
                    return Head::Complete(buf);
                }
                return Head::TooLarge;
            }
            if buf.len() >= MAX_HEAD_BYTES {
                return Head::TooLarge;
            }
        }
    };
    tokio::time::timeout(READ_TIMEOUT, read).await.unwrap_or(Head::Abandoned)
}

fn parse_request(head: &[u8]) -> Parsed {
    let line_end = head.windows(2).position(|w| w == b"\r\n").unwrap_or(head.len());
    let Ok(line) = std::str::from_utf8(&head[..line_end]) else {
        return Parsed::BadRequest;
    };
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Parsed::BadRequest;
    };
    if !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return Parsed::BadRequest;
    }
    if method != "GET" {
        return Parsed::NotFound;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path == "/callback" {
        Parsed::Callback(query.to_owned())
    } else {
        Parsed::NotFound
    }
}

/// Interprets the query of the single accepted `/callback` request.
fn evaluate(query: &str, expected: &State) -> Result<AuthCode, AuthError> {
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1/?{query}")) else {
        return Err(AuthError::StateMismatch);
    };
    let (mut error, mut state, mut code) = (None, None, None);
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "error" => &mut error,
            "state" => &mut state,
            "code" => &mut code,
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(value.into_owned());
        }
    }
    if let Some(raw) = error {
        return Err(AuthError::Denied { code: sanitize_error_code(&raw) });
    }
    match state {
        Some(s) if ct_eq(s.as_bytes(), expected.expose().as_bytes()) => {}
        _ => return Err(AuthError::StateMismatch),
    }
    match code {
        Some(c) if !c.is_empty() => Ok(AuthCode::new(c)),
        _ => Err(AuthError::MissingCode),
    }
}

/// Keeps an `error` value only if it looks like an identifier (`[A-Za-z0-9_]`, at most
/// 64 chars); anything else could carry arbitrary text.
fn sanitize_error_code(raw: &str) -> String {
    let ok = !raw.is_empty()
        && raw.len() <= MAX_ERROR_CODE_LEN
        && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if ok {
        raw.to_owned()
    } else {
        "unknown".to_owned()
    }
}

/// Constant-time equality (the length is not secret: the state has a fixed size).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = u8::from(a.len() != b.len());
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    std::hint::black_box(diff) == 0
}

async fn respond(stream: &mut TcpStream, status: u16, reason: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let write = async {
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(body.as_bytes()).await?;
        stream.shutdown().await
    };
    let _ = tokio::time::timeout(WRITE_TIMEOUT, write).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = "0123456789abcdef0123456789abcdef";
    const CODE: &str = "fake-code-4242";

    struct Running {
        port: u16,
        handle: tokio::task::JoinHandle<Result<AuthCode, AuthError>>,
    }

    async fn start(timeout: Duration) -> Running {
        let listener = CallbackListener::bind(0).await.unwrap();
        let port = listener.port();
        let handle = tokio::spawn(async move {
            listener.wait_for_code(&State::new(STATE), timeout).await
        });
        Running { port, handle }
    }

    /// Sends raw bytes and returns the whole response; `None` if the connection fails.
    async fn raw(port: u16, request: &[u8]) -> Option<String> {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.ok()?;
        s.write_all(request).await.ok()?;
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(3), s.read_to_end(&mut out)).await;
        Some(String::from_utf8_lossy(&out).into_owned())
    }

    async fn get(port: u16, target: &str) -> Option<String> {
        raw(port, format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes()).await
    }

    fn long() -> Duration {
        Duration::from_secs(30)
    }

    #[tokio::test]
    async fn valid_callback_returns_the_code() {
        let r = start(long()).await;
        let resp = get(r.port, &format!("/callback?code={CODE}&state={STATE}")).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"));
        assert!(resp.contains("Puedes cerrar esta pesta\u{f1}a"));
        assert!(!resp.contains(CODE) && !resp.contains(STATE));
        assert_eq!(r.handle.await.unwrap().unwrap().expose(), CODE);
    }

    #[tokio::test]
    async fn response_carries_the_security_headers() {
        let r = start(long()).await;
        let resp = get(r.port, &format!("/callback?code={CODE}&state={STATE}")).await.unwrap();
        let head = resp.split("\r\n\r\n").next().unwrap().to_ascii_lowercase();
        for h in [
            "content-type: text/html; charset=utf-8",
            "cache-control: no-store",
            "referrer-policy: no-referrer",
            "content-security-policy: default-src 'none'; style-src 'unsafe-inline'",
            "connection: close",
        ] {
            assert!(head.contains(h), "missing header {h}");
        }
        assert!(!resp.contains("http://") && !resp.contains("https://"));
        let _ = r.handle.await;
    }

    #[tokio::test]
    async fn wrong_state_aborts_the_flow() {
        let r = start(long()).await;
        let resp = get(r.port, &format!("/callback?code={CODE}&state=ffff")).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 400"));
        assert!(matches!(r.handle.await.unwrap(), Err(AuthError::StateMismatch)));
    }

    #[tokio::test]
    async fn missing_state_aborts_the_flow() {
        let r = start(long()).await;
        let resp = get(r.port, &format!("/callback?code={CODE}")).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 400"));
        assert!(matches!(r.handle.await.unwrap(), Err(AuthError::StateMismatch)));
    }

    #[tokio::test]
    async fn missing_code_is_reported() {
        let r = start(long()).await;
        let resp = get(r.port, &format!("/callback?state={STATE}")).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 400"));
        assert!(matches!(r.handle.await.unwrap(), Err(AuthError::MissingCode)));
    }

    #[tokio::test]
    async fn other_paths_and_methods_get_404_and_listener_keeps_waiting() {
        let r = start(long()).await;
        let resp = get(r.port, "/favicon.ico").await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 404"));
        let resp = raw(r.port, b"POST /callback?code=x HTTP/1.1\r\n\r\n").await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 404"));
        assert!(!r.handle.is_finished());
        get(r.port, &format!("/callback?code={CODE}&state={STATE}")).await.unwrap();
        assert_eq!(r.handle.await.unwrap().unwrap().expose(), CODE);
    }

    #[tokio::test]
    async fn only_the_first_callback_is_accepted() {
        let r = start(long()).await;
        get(r.port, &format!("/callback?code={CODE}&state={STATE}")).await.unwrap();
        assert_eq!(r.handle.await.unwrap().unwrap().expose(), CODE);
        // The listener is gone: a second callback cannot succeed.
        let second = get(r.port, &format!("/callback?code=other&state={STATE}")).await;
        assert!(second.is_none_or(|resp| !resp.starts_with("HTTP/1.1 200")));
    }

    #[tokio::test]
    async fn a_second_concurrent_callback_is_not_evaluated() {
        let claimed = Arc::new(AtomicBool::new(true));
        let (tx, mut rx) = mpsc::channel(1);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = tokio::spawn(async move {
            get(port, &format!("/callback?code={CODE}&state={STATE}")).await
        });
        let (stream, _) = listener.accept().await.unwrap();
        handle(stream, Arc::new(State::new(STATE)), claimed, tx).await;
        assert!(client.await.unwrap().unwrap().starts_with("HTTP/1.1 404"));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn access_denied_is_reported_with_a_sanitized_code() {
        let r = start(long()).await;
        let resp = get(r.port, &format!("/callback?error=access_denied&state={STATE}")).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"));
        match r.handle.await.unwrap() {
            Err(AuthError::Denied { code }) => assert_eq!(code, "access_denied"),
            other => panic!("unexpected {other:?}"),
        }

        let r = start(long()).await;
        get(r.port, "/callback?error=bad%20%3Cscript%3E").await.unwrap();
        match r.handle.await.unwrap() {
            Err(AuthError::Denied { code }) => assert_eq!(code, "unknown"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn times_out() {
        let r = start(Duration::from_millis(100)).await;
        assert!(matches!(r.handle.await.unwrap(), Err(AuthError::Timeout)));
    }

    #[tokio::test]
    async fn busy_port_is_reported() {
        let first = CallbackListener::bind(0).await.unwrap();
        let again = CallbackListener::bind(first.port()).await;
        assert!(matches!(again, Err(AuthError::PortInUse)));
    }

    #[tokio::test]
    async fn binds_ipv4_loopback_only() {
        let l = CallbackListener::bind(0).await.unwrap();
        let addr = l.local_addr().unwrap();
        assert_eq!(addr.ip(), std::net::IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[tokio::test]
    async fn oversized_request_gets_400_and_listener_keeps_waiting() {
        let r = start(long()).await;
        let mut big = b"GET /callback?x=".to_vec();
        big.extend(std::iter::repeat_n(b'a', 9000));
        let resp = raw(r.port, &big).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 400"));
        assert!(!r.handle.is_finished());
        get(r.port, &format!("/callback?code={CODE}&state={STATE}")).await.unwrap();
        assert!(r.handle.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn garbled_request_gets_400() {
        let r = start(long()).await;
        let resp = raw(r.port, b"\x16\x03\x01garbage\r\n\r\n").await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 400"));
        assert!(!r.handle.is_finished());
        r.handle.abort();
    }

    #[test]
    fn constant_time_compare() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(!ct_eq(b"", b"a"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn generated_states_are_random_hex() {
        let a = State::generate().unwrap();
        let b = State::generate().unwrap();
        assert_eq!(a.expose().len(), 32);
        assert!(a.expose().bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a.expose(), b.expose());
    }
}
