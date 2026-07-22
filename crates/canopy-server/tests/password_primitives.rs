use canopy_server::identity::{Argon2PasswordHasher, PasswordHasher, PasswordPolicy};

const PASSPHRASE: &str = "correct horse battery staple";

#[test]
fn password_policy_accepts_long_passphrases_without_composition_rules() {
    assert!(PasswordPolicy::default().validate(PASSPHRASE).is_ok());
}

#[test]
fn password_policy_accepts_eight_through_sixty_four_unicode_characters() {
    let policy = PasswordPolicy::default();

    assert!(policy.validate("12345678").is_ok());
    assert!(policy.validate(&"a".repeat(64)).is_ok());
    assert!(policy.validate(&"\u{1f43c}".repeat(8)).is_ok());
}

#[test]
fn password_policy_rejects_outside_creation_boundaries() {
    let policy = PasswordPolicy::default();

    assert!(policy.validate("1234567").is_err());
    assert!(policy.validate(&"a".repeat(65)).is_err());
}

#[test]
fn verification_does_not_apply_new_password_length_policy() {
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHasher as _, SaltString};
    use rand_core::OsRng;

    let legacy_password = "a".repeat(65);
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(legacy_password.as_bytes(), &salt)
        .unwrap()
        .to_string();

    assert!(
        Argon2PasswordHasher::default()
            .verify(&legacy_password, &hash)
            .unwrap()
            .valid
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
