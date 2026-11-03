//! What the Settings page reads of Observer's own configuration, which the panel cannot change.

use axum::Json;
use axum::extract::State;
use observer_app::Metrics;
use observer_domain::settings::ObserverSettings;

use crate::AppState;

pub(crate) async fn observer(State(state): State<AppState>) -> Json<ObserverSettings> {
    Json(ObserverSettings {
        config_file: state.bootstrap.config_file.display().to_string(),
        listen: state.bootstrap.listen.to_string(),
        hosts: state.hosts.to_vec(),
        trusted_proxies: state.proxies.iter().map(ToString::to_string).collect(),
        statistics: Metrics::kept(),
    })
}
