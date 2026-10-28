//! axum's extractors, with their rejections turned into API error codes like every other failure.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::header;
use axum::http::request::Parts;
use observer_app::Client;
use observer_domain::error::ErrorCode;
use serde::de::DeserializeOwned;

use crate::error::Failure;

/// A JSON request body. A body that does not parse is `invalid_request`.
pub(crate) struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Failure;

    async fn from_request(request: Request, state: &S) -> Result<Self, Failure> {
        axum::Json::<T>::from_request(request, state)
            .await
            .map(|axum::Json(value)| Self(value))
            .map_err(|_| ErrorCode::InvalidRequest.into())
    }
}

/// A query string. One that does not parse is `invalid_request`.
pub(crate) struct QueryParams<T>(pub T);

impl<S, T> FromRequestParts<S> for QueryParams<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Failure> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|_| ErrorCode::InvalidRequest.into())
    }
}

/// The `{id}` of a path. An id that is not a number names nothing: `not_found`.
pub(crate) struct Id(pub i64);

impl<S: Send + Sync> FromRequestParts<S> for Id {
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Failure> {
        Path::<i64>::from_request_parts(parts, state)
            .await
            .map(|Path(id)| Self(id))
            .map_err(|_| ErrorCode::NotFound.into())
    }
}

/// Where the request came from: the peer address, since no proxy is trusted yet, and the
/// user agent.
pub(crate) struct RemoteClient(pub Client);

impl<S: Send + Sync> FromRequestParts<S> for RemoteClient {
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Failure> {
        let ConnectInfo(address) = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .map_err(|_| ErrorCode::Internal)?;
        let user_agent = parts
            .headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok());
        Ok(Self(Client::new(address.ip(), user_agent)))
    }
}
