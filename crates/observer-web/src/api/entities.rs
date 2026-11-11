//! The server's entities: a page of the list, the entities the map draws, a deletion, and how
//! often the server lists them. The state of the list comes over the websocket, and following it
//! there is what keeps the server listing.

use axum::extract::State;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::entities::{
    DEFAULT_CELL, EntitiesSettings, EntityDeleteRequest, EntityDeletion, EntityFilterQuery, EntityFilters,
    EntityMap, EntityPage, EntitySort,
};
use observer_domain::error::ErrorCode;
use observer_domain::map::MapBounds;
use serde::Deserialize;

use crate::AppState;
use crate::error::Failure;
use crate::extract::{JsonBody, QueryParams};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PageQuery {
    kinds: Option<String>,
    class: Option<String>,
    resource: Option<String>,
    owner: Option<String>,
    bucket: Option<i32>,
    search: Option<String>,
    #[serde(default)]
    sort: EntitySort,
    #[serde(default)]
    descending: bool,
    #[serde(default)]
    page: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MapQuery {
    kinds: Option<String>,
    class: Option<String>,
    resource: Option<String>,
    owner: Option<String>,
    bucket: Option<i32>,
    search: Option<String>,
    /// The side of a density cell, should the entities be too many to draw each.
    cell: Option<u32>,
    /// The part of the map the panel shows, in game units: all four, or none for the whole map.
    west: Option<f64>,
    east: Option<f64>,
    south: Option<f64>,
    north: Option<f64>,
}

/// The filters a query names; one that names nothing is `invalid_request`.
fn filters(query: &EntityFilterQuery) -> Result<EntityFilters, Failure> {
    EntityFilters::parse(query).ok_or_else(|| ErrorCode::InvalidRequest.into())
}

pub(crate) async fn page(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<PageQuery>,
) -> Result<Json<EntityPage>, Failure> {
    let kept = filters(&EntityFilterQuery {
        kinds: query.kinds,
        class: query.class,
        resource: query.resource,
        owner: query.owner,
        bucket: query.bucket,
        search: query.search,
    })?;
    let page = state
        .entities()?
        .page(&caller, &kept, query.sort, query.descending, query.page)
        .await?;
    Ok(Json(page))
}

pub(crate) async fn map(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<MapQuery>,
) -> Result<Json<EntityMap>, Failure> {
    let kept = filters(&EntityFilterQuery {
        kinds: query.kinds,
        class: query.class,
        resource: query.resource,
        owner: query.owner,
        bucket: query.bucket,
        search: query.search,
    })?;
    let bounds = match (query.west, query.east, query.south, query.north) {
        (None, None, None, None) => None,
        (Some(west), Some(east), Some(south), Some(north)) => Some(MapBounds {
            west,
            east,
            south,
            north,
        }),
        _ => return Err(ErrorCode::InvalidRequest.into()),
    };
    let map = state
        .entities()?
        .map(&caller, &kept, query.cell.unwrap_or(DEFAULT_CELL), bounds)
        .await?;
    Ok(Json(map))
}

pub(crate) async fn delete(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<EntityDeleteRequest>,
) -> Result<Json<EntityDeletion>, Failure> {
    Ok(Json(state.entities()?.delete(&caller, request).await?))
}

pub(crate) async fn settings(State(state): State<AppState>) -> Result<Json<EntitiesSettings>, Failure> {
    Ok(Json(state.entities()?.settings().await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(settings): JsonBody<EntitiesSettings>,
) -> Result<Json<EntitiesSettings>, Failure> {
    let entities = state.entities()?;
    entities.change_settings(&caller, settings).await?;
    Ok(Json(entities.settings().await?))
}
