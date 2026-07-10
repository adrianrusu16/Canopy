use std::collections::BTreeMap;
use std::sync::Arc;

use canopy_core::{CanopyError, CanopyResult, UserProfile};
use canopy_proto::profile_service_server::ProfileService;
use canopy_proto::{
    DeleteProfileRequest, GetPreferencesRequest, GetProfileRequest, Preferences, Profile,
    UpdatePreferencesRequest, UpdateProfileRequest, UpsertProfileRequest,
};
use prost_types::{FieldMask, Struct, Value, value::Kind};
use tonic::{Request, Response, Status};

use super::{GrpcServices, extract_durable_principal};
use crate::api::to_status;

pub struct ProfileGrpc(pub Arc<GrpcServices>);

#[tonic::async_trait]
impl ProfileService for ProfileGrpc {
    async fn upsert_profile(
        &self,
        request: Request<UpsertProfileRequest>,
    ) -> Result<Response<Profile>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let profile = self
            .0
            .profile
            .upsert_from_contract(&identity, request.display_name.as_deref())
            .await
            .map_err(to_status)?;
        Ok(Response::new(to_proto_profile(profile)))
    }

    async fn get_profile(
        &self,
        request: Request<GetProfileRequest>,
    ) -> Result<Response<Profile>, Status> {
        let identity = extract_durable_principal(request.metadata(), &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let profile = self
            .0
            .profile
            .get_profile(&identity)
            .await
            .map_err(to_status)?;
        Ok(Response::new(to_proto_profile(profile)))
    }

    async fn update_profile(
        &self,
        request: Request<UpdateProfileRequest>,
    ) -> Result<Response<Profile>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let profile = request
            .profile
            .ok_or_else(|| Status::invalid_argument("profile is required"))?;
        let mask = request
            .update_mask
            .ok_or_else(|| Status::invalid_argument("update_mask is required"))?;
        let display_name = masked_display_name(&profile, &mask).map_err(to_status)?;
        let profile = self
            .0
            .profile
            .upsert_from_contract(&identity, display_name)
            .await
            .map_err(to_status)?;
        Ok(Response::new(to_proto_profile(profile)))
    }

    async fn delete_profile(
        &self,
        request: Request<DeleteProfileRequest>,
    ) -> Result<Response<()>, Status> {
        let identity = extract_durable_principal(request.metadata(), &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        self.0
            .profile
            .delete_profile(&identity)
            .await
            .map_err(to_status)?;
        Ok(Response::new(()))
    }

    async fn get_preferences(
        &self,
        request: Request<GetPreferencesRequest>,
    ) -> Result<Response<Preferences>, Status> {
        let identity = extract_durable_principal(request.metadata(), &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let preferences = self
            .0
            .preferences
            .get_preferences(&identity)
            .await
            .map_err(to_status)?;
        Ok(Response::new(Preferences {
            values: Some(json_to_struct(&preferences.values_json).map_err(to_status)?),
        }))
    }

    async fn update_preferences(
        &self,
        request: Request<UpdatePreferencesRequest>,
    ) -> Result<Response<Preferences>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let values = request
            .preferences
            .and_then(|preferences| preferences.values)
            .ok_or_else(|| Status::invalid_argument("preferences.values is required"))?;
        let values_json = struct_to_json(&values).map_err(to_status)?;
        let preferences = self
            .0
            .preferences
            .update_preferences(&identity, &values_json)
            .await
            .map_err(to_status)?;
        Ok(Response::new(Preferences {
            values: Some(json_to_struct(&preferences.values_json).map_err(to_status)?),
        }))
    }
}

fn to_proto_profile(profile: UserProfile) -> Profile {
    Profile {
        id: profile.id,
        external_user_id: profile.external_user_id,
        display_name: profile.display_name,
        created_at: None,
        updated_at: None,
    }
}

fn masked_display_name<'a>(
    profile: &'a Profile,
    mask: &FieldMask,
) -> CanopyResult<Option<&'a str>> {
    if mask.paths.as_slice() != ["display_name"] {
        return Err(CanopyError::InvalidArgument(
            "update_mask must contain only display_name".into(),
        ));
    }
    Ok(profile.display_name.as_deref())
}

fn struct_to_json(value: &Struct) -> CanopyResult<String> {
    let object = value
        .fields
        .iter()
        .map(|(key, value)| Ok((key.clone(), prost_value_to_json(value)?)))
        .collect::<CanopyResult<serde_json::Map<String, serde_json::Value>>>()?;
    serde_json::to_string(&serde_json::Value::Object(object))
        .map_err(|error| CanopyError::InvalidArgument(format!("invalid preferences: {error}")))
}

fn prost_value_to_json(value: &Value) -> CanopyResult<serde_json::Value> {
    match value.kind.as_ref() {
        None | Some(Kind::NullValue(_)) => Ok(serde_json::Value::Null),
        Some(Kind::NumberValue(number)) => serde_json::Number::from_f64(*number)
            .map(serde_json::Value::Number)
            .ok_or_else(|| CanopyError::InvalidArgument("preference number must be finite".into())),
        Some(Kind::StringValue(value)) => Ok(serde_json::Value::String(value.clone())),
        Some(Kind::BoolValue(value)) => Ok(serde_json::Value::Bool(*value)),
        Some(Kind::StructValue(value)) => {
            let json = struct_to_json(value)?;
            serde_json::from_str(&json).map_err(|error| {
                CanopyError::InvalidArgument(format!("invalid nested preferences: {error}"))
            })
        }
        Some(Kind::ListValue(value)) => value
            .values
            .iter()
            .map(prost_value_to_json)
            .collect::<CanopyResult<Vec<_>>>()
            .map(serde_json::Value::Array),
    }
}

fn json_to_struct(json: &str) -> CanopyResult<Struct> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| {
        CanopyError::Internal(format!("stored preferences are invalid: {error}"))
    })?;
    let serde_json::Value::Object(object) = value else {
        return Err(CanopyError::Internal(
            "stored preferences must be a JSON object".into(),
        ));
    };
    let fields = object
        .into_iter()
        .map(|(key, value)| Ok((key, json_value_to_prost(value)?)))
        .collect::<CanopyResult<BTreeMap<_, _>>>()?;
    Ok(Struct { fields })
}

fn json_value_to_prost(value: serde_json::Value) -> CanopyResult<Value> {
    let kind = match value {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(value) => Kind::BoolValue(value),
        serde_json::Value::Number(value) => Kind::NumberValue(value.as_f64().ok_or_else(|| {
            CanopyError::Internal("stored preference number is outside protobuf range".into())
        })?),
        serde_json::Value::String(value) => Kind::StringValue(value),
        serde_json::Value::Array(values) => Kind::ListValue(prost_types::ListValue {
            values: values
                .into_iter()
                .map(json_value_to_prost)
                .collect::<CanopyResult<Vec<_>>>()?,
        }),
        serde_json::Value::Object(values) => Kind::StructValue(Struct {
            fields: values
                .into_iter()
                .map(|(key, value)| Ok((key, json_value_to_prost(value)?)))
                .collect::<CanopyResult<BTreeMap<_, _>>>()?,
        }),
    };
    Ok(Value { kind: Some(kind) })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use prost_types::{FieldMask, Struct, Value, value::Kind};

    use super::*;

    #[test]
    fn profile_update_requires_explicit_display_name_mask() {
        let profile = Profile {
            display_name: Some("Ada".into()),
            ..Profile::default()
        };
        let mask = FieldMask {
            paths: vec!["display_name".into()],
        };

        assert_eq!(masked_display_name(&profile, &mask).unwrap(), Some("Ada"));
    }

    #[test]
    fn preferences_struct_round_trips_through_canonical_json() {
        let preferences = Struct {
            fields: BTreeMap::from([(
                "history_ui".into(),
                Value {
                    kind: Some(Kind::BoolValue(true)),
                },
            )]),
        };

        let json = struct_to_json(&preferences).unwrap();
        let decoded = json_to_struct(&json).unwrap();

        assert_eq!(decoded, preferences);
    }
}
