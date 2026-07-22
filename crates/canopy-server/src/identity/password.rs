use argon2::password_hash::{
    Error as PasswordHashError, PasswordHash, PasswordHasher as Argon2PasswordHasherTrait,
    PasswordVerifier, SaltString,
};
use argon2::{Algorithm, Argon2, Params, Version};
use canopy_core::{CanopyError, CanopyResult};
use rand_core::OsRng;

const ARGON2_MEMORY_COST_KIB: u32 = 19_456;
const ARGON2_TIME_COST: u32 = 2;
const ARGON2_PARALLELISM: u32 = 1;

pub const PASSWORD_MIN_CHARS: usize = 8;
pub const PASSWORD_MAX_CHARS: usize = 64;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PasswordPolicy {
    min_chars: usize,
    max_chars: usize,
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            min_chars: PASSWORD_MIN_CHARS,
            max_chars: PASSWORD_MAX_CHARS,
        }
    }
}

impl PasswordPolicy {
    pub fn validate(&self, password: &str) -> CanopyResult<()> {
        let char_count = password.chars().count();
        if char_count < self.min_chars {
            return Err(CanopyError::InvalidArgument(format!(
                "password must be at least {PASSWORD_MIN_CHARS} characters"
            )));
        }
        if char_count > self.max_chars {
            return Err(CanopyError::InvalidArgument(format!(
                "password must be no more than {PASSWORD_MAX_CHARS} characters"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PasswordVerification {
    pub valid: bool,
    pub needs_rehash: bool,
}

pub trait PasswordHasher: Send + Sync {
    fn hash(&self, password: &str) -> CanopyResult<String>;

    fn verify(&self, password: &str, password_hash_phc: &str)
    -> CanopyResult<PasswordVerification>;
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct Argon2PasswordHasher {
    policy: PasswordPolicy,
}

impl Argon2PasswordHasher {
    fn argon2(&self) -> CanopyResult<Argon2<'static>> {
        let params = Params::new(
            ARGON2_MEMORY_COST_KIB,
            ARGON2_TIME_COST,
            ARGON2_PARALLELISM,
            None,
        )
        .map_err(|error| CanopyError::Internal(format!("invalid Argon2 parameters: {error}")))?;

        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }

    fn uses_current_policy(password_hash_phc: &str) -> bool {
        password_hash_phc.starts_with("$argon2id$v=19$")
            && password_hash_phc.contains("m=19456,t=2,p=1")
    }
}

impl PasswordHasher for Argon2PasswordHasher {
    fn hash(&self, password: &str) -> CanopyResult<String> {
        self.policy.validate(password)?;

        let salt = SaltString::generate(&mut OsRng);
        let password_hash = self
            .argon2()?
            .hash_password(password.as_bytes(), &salt)
            .map_err(|error| CanopyError::Internal(format!("failed to hash password: {error}")))?;

        Ok(password_hash.to_string())
    }

    fn verify(
        &self,
        password: &str,
        password_hash_phc: &str,
    ) -> CanopyResult<PasswordVerification> {
        let parsed_hash = PasswordHash::new(password_hash_phc).map_err(|error| {
            CanopyError::Unauthenticated(format!("invalid password hash: {error}"))
        })?;

        match self
            .argon2()?
            .verify_password(password.as_bytes(), &parsed_hash)
        {
            Ok(()) => Ok(PasswordVerification {
                valid: true,
                needs_rehash: !Self::uses_current_policy(password_hash_phc),
            }),
            Err(PasswordHashError::Password) => Ok(PasswordVerification {
                valid: false,
                needs_rehash: false,
            }),
            Err(error) => Err(CanopyError::Unauthenticated(format!(
                "password verification failed: {error}"
            ))),
        }
    }
}
