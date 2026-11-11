//! What the panel's Documentation page is built from: the integration contract as this build
//! ships it, and what the server Observer runs does of it.

use observer_domain::docs::{BridgeExport, ExampleFile, IntegrationDocs, IntegrationServer};
use observer_domain::permissions::Permissions;
use observer_fxserver::supervisor::State;
use observer_fxserver::{bridge, contract};

use crate::{Caller, Error, Servers, Supervision};

/// The contract with its example, what the bridge raises and exports, and this server's side of
/// it. The contract holds no secret, so any account reads it. What the server does now goes to
/// those who see its state elsewhere in the panel, the holders of the dashboard's permission.
pub async fn integration_docs(
    caller: &Caller,
    supervision: &Supervision,
    servers: &Servers,
) -> Result<IntegrationDocs, Error> {
    let server = if caller.permissions.contains(Permissions::DASHBOARD_VIEW) {
        Some(server_side(supervision, servers).await?)
    } else {
        None
    };
    Ok(IntegrationDocs {
        contract: contract::TEXT.to_owned(),
        contract_version: bridge::CONTRACT_VERSION,
        events: contract::events(),
        exports: contract::exports()
            .into_iter()
            .map(|export| BridgeExport {
                name: export.name.to_owned(),
                parameters: export.parameters.into_iter().map(str::to_owned).collect(),
            })
            .collect(),
        example: contract::EXAMPLE
            .into_iter()
            .map(|(name, text)| ExampleFile {
                name: name.to_owned(),
                text: text.to_owned(),
            })
            .collect(),
        server,
    })
}

async fn server_side(supervision: &Supervision, servers: &Servers) -> Result<IntegrationServer, Error> {
    let next = servers
        .launch_settings(Servers::DEFAULT)
        .await?
        .map(|stored| stored.settings.txadmin_events);
    let status = supervision.supervisor().status();
    let status = status.borrow();
    Ok(IntegrationServer {
        bridge: status.bridge.clone(),
        starting: status.state == State::Starting,
        txadmin_events: status.txadmin_events,
        txadmin_events_next: next,
    })
}
