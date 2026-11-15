//! Manual checks against the Enable Banking SANDBOX. Ignored by default; they need
//! `ENABLE_BANKING_KEY_PATH` (sandbox key) and `ENABLE_BANKING_APP_ID` (sandbox app).
//! They print only public data (bank names), never tokens or account data.
//!
//! Run: `cargo test -p ledger-core --test sandbox -- --ignored --nocapture`

use ledger_core::bank::enable_banking::{EbClient, EbConfig, PsuType};

fn client() -> EbClient {
    let app_id = std::env::var("ENABLE_BANKING_APP_ID").expect("ENABLE_BANKING_APP_ID not set");
    EbClient::new(EbConfig::from_env(app_id).expect("key path not configured")).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn sandbox_lists_spanish_aspsps() {
    let aspsps = client().list_aspsps("ES", PsuType::Personal).await.expect("GET /aspsps failed");
    println!("{} ASPSPs for ES:", aspsps.len());
    for a in &aspsps {
        println!(
            "  {} ({}), max consent {:?} s",
            a.name, a.country, a.maximum_consent_validity
        );
    }
    assert!(!aspsps.is_empty());
}
