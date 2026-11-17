//! Live Bitwarden/Vaultwarden integration test (ADR-0016, P2-9/10/11/17).
//!
//! This is the wire-compat validation the in-crate unit tests deliberately can't
//! do: it talks to a **real** server. It self-registers a fresh account, then
//! runs login → import → sync → lookup end-to-end through the crate's *public*
//! API - the same surface a frontend uses.
//!
//! It is `#[ignore]`d, gated on `VAULT_TEST_URL`, and behind the `live-tests`
//! feature (which pulls the RSA dep used only for registration), so `just verify`
//! never compiles or runs it. To run it against a local Vaultwarden:
//!
//! ```sh
//! VAULT_TEST_URL=http://<server> \
//!   cargo test -p browser-core --features live-tests --test vault_live -- --ignored --nocapture
//! ```
//!
//! Reusable on the security treadmill: re-run after every CEF / crypto-dep bump.
#![cfg(feature = "live-tests")]

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use browser_core::vault::auth::{Credentials, Device, Grant, ServerConfig, VaultClient};
use browser_core::vault::crypto::{Kdf, MasterKey, SymmetricKey};
use browser_core::vault::http::{HttpRequest, HttpResponse, HttpTransport, UreqTransport};
use browser_core::vault::import::ImportRecord;
use browser_core::vault::lookup::matching_logins;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
use rsa::{RsaPrivateKey, RsaPublicKey};

/// A KDF cheap enough for a test, strong enough to be a real value.
const KDF: Kdf = Kdf::Pbkdf2 {
    iterations: 100_000,
};
const PASSWORD: &str = "correct horse battery staple";

fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::getrandom(&mut bytes).expect("os rng");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn json_headers() -> Vec<(String, String)> {
    vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "application/json".to_string()),
    ]
}

fn post_json(url: String, body: serde_json::Value) -> HttpResponse {
    let request = HttpRequest::post(url, json_headers(), serde_json::to_vec(&body).unwrap());
    UreqTransport::new().send(request).expect("transport")
}

/// Register a fresh account via Bitwarden's two-step flow (as Vaultwarden 1.36+
/// implements it). Prints the server's response on failure so wire mismatches
/// are visible - this is the wire-compat validation.
fn register(config: &ServerConfig, email: &str) {
    // Step 1: request a verification token. With no SMTP configured, Vaultwarden
    // returns the JWT directly in the 200 body instead of emailing it.
    let resp = post_json(
        format!(
            "{}/accounts/register/send-verification-email",
            config.identity_url
        ),
        serde_json::json!({ "email": email, "name": "browser vault test" }),
    );
    assert!(
        (200..300).contains(&resp.status),
        "send-verification-email failed: HTTP {} - {}",
        resp.status,
        String::from_utf8_lossy(&resp.body)
    );
    let token: String = serde_json::from_slice(&resp.body).unwrap_or_default();

    // Derive the account crypto (this is exactly what login will reverse).
    let master = MasterKey::derive(PASSWORD, email, KDF).expect("derive");
    let password_hash = master.password_hash(PASSWORD);
    let stretched = master.stretch().expect("stretch");

    let mut sym = [0u8; 64];
    getrandom::getrandom(&mut sym).expect("os rng");
    let user_key = SymmetricKey::from_bytes(&sym).expect("sym key");
    let protected_key = stretched.encrypt(&sym).expect("wrap user key").to_string();

    // A real RSA account keypair; the private key is stored encrypted under the
    // user key (Bitwarden's account model).
    let mut rng = rand::thread_rng();
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen");
    let public = RsaPublicKey::from(&private);
    let public_key_b64 = B64.encode(public.to_public_key_der().unwrap().as_bytes());
    let encrypted_private_key = user_key
        .encrypt(private.to_pkcs8_der().unwrap().as_bytes())
        .expect("wrap private key")
        .to_string();

    let (kdf_type, iterations) = match KDF {
        Kdf::Pbkdf2 { iterations } => (0u32, iterations),
        Kdf::Argon2id { iterations, .. } => (1u32, iterations),
    };

    // Step 2: finish registration with the token + crypto.
    let resp = post_json(
        format!("{}/accounts/register/finish", config.identity_url),
        serde_json::json!({
            "email": email,
            "emailVerificationToken": token,
            "masterPasswordHash": password_hash,
            "masterPasswordHint": serde_json::Value::Null,
            "userSymmetricKey": protected_key,
            "userAsymmetricKeys": {
                "publicKey": public_key_b64,
                "encryptedPrivateKey": encrypted_private_key,
            },
            "kdf": kdf_type,
            "kdfIterations": iterations,
        }),
    );
    assert!(
        (200..300).contains(&resp.status),
        "register/finish failed: HTTP {} - {}",
        resp.status,
        String::from_utf8_lossy(&resp.body)
    );
    println!("[live] registered {email} (HTTP {})", resp.status);
}

#[test]
#[ignore = "requires a live server via VAULT_TEST_URL"]
fn live_register_login_import_sync_lookup() {
    let Ok(url) = std::env::var("VAULT_TEST_URL") else {
        println!("[live] VAULT_TEST_URL unset - skipping");
        return;
    };
    let config = ServerConfig::self_hosted(&url);
    let email = format!("browser-test-{}@example.com", random_hex(6));
    println!("[live] server={url}");

    // 1. Register.
    register(&config, &email);

    // 2. Log in - the full prelogin → token → decrypt-vault-key chain.
    let client = VaultClient::new(UreqTransport::new(), config.clone());
    let creds = Credentials {
        email: email.clone(),
        master_password: PASSWORD.to_string(),
        grant: Grant::Password,
    };
    let device = Device::generate("browser-live-test");
    let session = client
        .login(&creds, &device, None)
        .unwrap_or_else(|e| panic!("login failed: {e}"));
    println!(
        "[live] login ok - access token {} chars",
        session.tokens.access_token.len()
    );

    // 3. Import two credentials.
    let records = vec![
        ImportRecord {
            name: "Example".to_string(),
            username: Some("alice@example.com".to_string()),
            password: Some("s3cret-p@ss,word".to_string()), // comma on purpose
            uri: Some("https://example.com".to_string()),
            totp: None,
            notes: Some("imported by the live test".to_string()),
        },
        ImportRecord {
            name: "GitHub".to_string(),
            username: Some("octocat".to_string()),
            password: Some("hunter2".to_string()),
            uri: Some("https://github.com/login".to_string()),
            totp: None,
            notes: None,
        },
    ];
    client
        .import(&session, &records)
        .unwrap_or_else(|e| panic!("import failed: {e}"));
    println!("[live] imported {} logins", records.len());

    // 4. Sync and confirm the imported logins came back, decrypted.
    let vault = client
        .sync(&session)
        .unwrap_or_else(|e| panic!("sync failed: {e}"));
    println!("[live] sync ok - {} logins", vault.logins.len());
    assert!(
        vault.logins.len() >= 2,
        "expected the 2 imported logins, got {}",
        vault.logins.len()
    );

    let example = vault
        .logins
        .iter()
        .find(|l| l.name == "Example")
        .expect("imported 'Example' login present after sync");
    assert_eq!(example.username.as_deref(), Some("alice@example.com"));
    assert_eq!(
        example.password.as_deref(),
        Some("s3cret-p@ss,word"),
        "comma survived the round-trip"
    );

    // 5. Lookup by host matches the right login.
    let hits = matching_logins(&vault, "example.com");
    assert!(
        hits.iter().any(|l| l.name == "Example"),
        "vault_lookup found example.com"
    );
    let gh = matching_logins(&vault, "github.com");
    assert!(
        gh.iter().any(|l| l.name == "GitHub"),
        "domain match reached github.com login"
    );
    // A site with no stored login matches nothing.
    assert!(matching_logins(&vault, "no-such-site.example.org").is_empty());

    println!("[live] ✅ register → login → import → sync → lookup all passed against {url}");
}
