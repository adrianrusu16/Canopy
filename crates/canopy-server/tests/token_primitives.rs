use canopy_server::identity::{
    AccessTokenConfig, Ed25519AccessTokenIssuer, OpaqueToken, TokenDigest,
};

#[test]
fn access_token_round_trips_required_claims() {
    let issuer = Ed25519AccessTokenIssuer::generate(AccessTokenConfig {
        issuer: "canopy.test".into(),
        audience: "pandawave".into(),
        key_id: "test-key-1".into(),
        ttl_seconds: 900,
    });

    let issued = issuer.issue("account-1", "session-1", 1_000).unwrap();
    let claims = issuer.verify(&issued.token, 1_100).unwrap();
    assert_eq!(issued.expires_at_epoch_ms, 1_900_000);
    assert_eq!(
        issued.expires_at_epoch_ms,
        claims.expires_at_epoch_seconds * 1_000
    );

    assert_eq!(claims.subject, "account-1");
    assert_eq!(claims.session_id, "session-1");
    assert_eq!(claims.issuer, "canopy.test");
    assert_eq!(claims.audience, "pandawave");
    assert_eq!(claims.key_id, "test-key-1");
    assert_eq!(claims.issued_at_epoch_seconds, 1_000);
    assert_eq!(claims.expires_at_epoch_seconds, 1_900);
    assert!(!claims.jwt_id.is_empty());
}

#[test]
fn access_token_rejects_wrong_audience() {
    let issuer = Ed25519AccessTokenIssuer::generate(AccessTokenConfig {
        issuer: "canopy.test".into(),
        audience: "pandawave".into(),
        key_id: "test-key-1".into(),
        ttl_seconds: 900,
    });
    let verifier = issuer.verifier(AccessTokenConfig {
        issuer: "canopy.test".into(),
        audience: "other-client".into(),
        key_id: "test-key-1".into(),
        ttl_seconds: 900,
    });

    let issued = issuer.issue("account-1", "session-1", 1_000).unwrap();

    assert!(verifier.verify(&issued.token, 1_100).is_err());
}

#[test]
fn access_token_rejects_expired_tokens() {
    let issuer = Ed25519AccessTokenIssuer::generate(AccessTokenConfig {
        issuer: "canopy.test".into(),
        audience: "pandawave".into(),
        key_id: "test-key-1".into(),
        ttl_seconds: 900,
    });

    let issued = issuer.issue("account-1", "session-1", 1_000).unwrap();

    assert!(issuer.verify(&issued.token, 1_901).is_err());
}

#[test]
fn access_token_issuer_uses_configured_signing_key() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    let config = AccessTokenConfig {
        issuer: "canopy.test".into(),
        audience: "pandawave".into(),
        key_id: "configured-key".into(),
        ttl_seconds: 900,
    };
    let signing_key = STANDARD.encode([11_u8; 32]);
    let issuer =
        Ed25519AccessTokenIssuer::from_signing_key_base64(config.clone(), &signing_key).unwrap();
    let verifier = Ed25519AccessTokenIssuer::from_signing_key_base64(config, &signing_key)
        .unwrap()
        .verifier(AccessTokenConfig {
            issuer: "canopy.test".into(),
            audience: "pandawave".into(),
            key_id: "configured-key".into(),
            ttl_seconds: 900,
        });

    let issued = issuer.issue("account-1", "session-1", 1_000).unwrap();

    assert_eq!(
        verifier.verify(&issued.token, 1_100).unwrap().subject,
        "account-1"
    );
}
#[test]
fn opaque_tokens_are_random_digestible_and_redacted() {
    let token = OpaqueToken::generate();
    let second = OpaqueToken::generate();
    let digest = TokenDigest::from_token(&token);

    assert_ne!(token.as_str(), second.as_str());
    assert_eq!(digest.as_bytes().len(), 32);
    assert_eq!(digest, TokenDigest::from_token(&token));
    assert_eq!(format!("{token:?}"), "[REDACTED]");
}
