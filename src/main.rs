#[macro_use]
extern crate macro_attr;

#[macro_use]
extern crate newtype_derive;

#[macro_use]
extern crate log;

#[macro_use]
extern crate eyre;

mod api;
mod core;
mod db;
mod integrations;
mod types;
mod utils;

use crate::core::expr::Expr;
// use db::{actions::find_floorplans, establish_connection};
use crate::core::{
    devices::Devices, groups::Groups, integrations::Integrations, message::handle_message,
    rules::Rules, scenes::Scenes, state::AppState,
};
use crate::types::event::mk_event_channel;
use api::init_api;
use color_eyre::Result;
use db::init_db;
use eyre::eyre;
use std::{error::Error, sync::Arc};
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    color_eyre::install()?;
    pretty_env_logger::init();

    // Attempt connecting to Postgres
    init_db().await;

    let (config, opaque_integrations_configs) = core::config::read_config()?;

    trace!("Using config:\n    {:#?}", config);

    let (event_tx, mut event_rx) = mk_event_channel();

    let mut integrations = Integrations::new(event_tx.clone());
    let groups = Groups::new(config.groups.unwrap_or_default());
    let mut scenes = Scenes::new(config.scenes.unwrap_or_default());
    scenes.refresh_db_scenes().await;
    let devices = Devices::new(event_tx.clone());
    let expr = Expr::new();
    let rules = Rules::new(config.routines.unwrap_or_default(), event_tx.clone());

    for (id, integration_config) in &config.integrations.unwrap_or_default() {
        let opaque_integration_config: &config::Value = opaque_integrations_configs
            .get(id)
            .ok_or_else(|| eyre!("Expected to find config for integration with id {}", id))?;

        integrations
            .load_integration(&integration_config.plugin, id, opaque_integration_config)
            .await?;
    }

    integrations.run_register_pass().await?;
    integrations.run_start_pass().await?;

    let state = AppState {
        integrations,
        groups,
        scenes,
        devices,
        rules,
        event_tx,
        expr,
        ws: Default::default(),
    };

    let state = Arc::new(RwLock::new(state));

    init_api(&state)?;

    loop {
        let msg = event_rx
            .recv()
            .await
            .expect("Expected sender end of channel to never be dropped");

        // trace!("Received message: {:.100}", format!("{:?}", msg));

        let state = Arc::clone(&state);

        tokio::spawn(async move {
            let mut state = state.write().await;
            let result = handle_message(&mut state, &msg).await;

            if let Err(err) = result {
                error!(
                    "Error while handling message:\n    Msg:\n    {:#?}\n\n    Err:\n    {:#?}",
                    msg, err
                );
            }
        });
    }
}
