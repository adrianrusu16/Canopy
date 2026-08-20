//! Track visibility policy shared by every backend surface.

/// Catalog partition a request is allowed to observe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackAccessScope {
    /// Release-safe, ready tracks only.
    Public,
    /// Public tracks plus ready personal tracks owned by this profile.
    Owner { profile_id: String },
}

impl TrackAccessScope {
    /// Returns the owner binding used by persistence adapters, when present.
    pub fn owner_profile_id(&self) -> Option<&str> {
        match self {
            Self::Public => None,
            Self::Owner { profile_id } => Some(profile_id),
        }
    }
}
