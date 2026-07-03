use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use canopy_proto::system_service_server::SystemService;
use canopy_proto::{DependencyStatus, GetStatusRequest, GetStatusResponse};
use prost_types::Timestamp;
use tonic::{Request, Response, Status};

use super::GrpcServices;

pub struct SystemGrpc(pub Arc<GrpcServices>);

#[tonic::async_trait]
impl SystemService for SystemGrpc {
    async fn get_status(
        &self,
        _: Request<GetStatusRequest>,
    ) -> Result<Response<GetStatusResponse>, Status> {
        let status = self.0.health.check().await;
        let checked_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                Status::internal(format!("system clock before Unix epoch: {error}"))
            })?;
        let seconds = i64::try_from(checked_at.as_secs())
            .map_err(|_| Status::internal("status timestamp overflow"))?;
        let nanos = i32::try_from(checked_at.subsec_nanos())
            .map_err(|_| Status::internal("status nanoseconds overflow"))?;

        Ok(Response::new(GetStatusResponse {
            healthy: status.healthy,
            version: status.version,
            status: status.status.as_str().into(),
            dependencies: status
                .dependencies
                .into_iter()
                .map(|dependency| DependencyStatus {
                    name: dependency.name,
                    status: dependency.status.as_str().into(),
                    message: dependency.message,
                })
                .collect(),
            checked_at: Some(Timestamp { seconds, nanos }),
        }))
    }
}
