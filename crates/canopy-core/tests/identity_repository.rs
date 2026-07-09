use canopy_core::IdentityRepository;

#[test]
fn identity_repository_is_a_send_sync_domain_port() {
    fn accepts_port<T: IdentityRepository + ?Sized>() {}

    accepts_port::<dyn IdentityRepository>();
}
