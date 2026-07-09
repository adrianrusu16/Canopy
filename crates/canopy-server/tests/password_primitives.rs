use canopy_server::identity::{Argon2PasswordHasher, PasswordHasher, PasswordPolicy};

const PASSPHRASE: &str = "correct horse battery staple";

#[test]
fn password_policy_accepts_long_passphrases_without_composition_rules() {
    assert!(PasswordPolicy::default().validate(PASSPHRASE).is_ok());
}

#[test]
fn password_policy_rejects_fewer_than_fifteen_characters() {
    assert!(
        PasswordPolicy::default()
            .validate("short password")
            .is_err()
    );
}

#[test]
fn argon2id_hash_verifies_and_current_policy_needs_no_rehash() {
    let hasher = Argon2PasswordHasher::default();
    let hash = hasher.hash(PASSPHRASE).unwrap();
    let verification = hasher.verify(PASSPHRASE, &hash).unwrap();

    assert!(verification.valid);
    assert!(!verification.needs_rehash);
    assert!(hash.starts_with("$argon2id$"));
}
