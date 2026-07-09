use canopy_proto::auth_service_client::AuthServiceClient;
use canopy_proto::{AccountSummary, SessionEnvelope};

#[test]
fn auth_contract_exports_service_and_session_resources() {
    fn accepts_client<T>(_client: Option<AuthServiceClient<T>>) {}

    accepts_client::<tonic::transport::Channel>(None);
    assert_eq!(AccountSummary::default().id, "");
    assert_eq!(SessionEnvelope::default().refresh_token, "");
}
