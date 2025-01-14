use color_eyre::Result;

use crate::types::{
    action::Action,
    dim::DimDescriptor,
    event::*,
    integration::CustomActionDescriptor,
    rule::ForceTriggerRoutineDescriptor,
    scene::{CycleScenesDescriptor, SceneDescriptor},
};

use crate::db::actions::{db_delete_scene, db_edit_scene, db_store_scene};

use super::{expr::eval_action_expr, state::AppState};

pub async fn handle_message(state: &mut AppState, msg: &Message) -> Result<()> {
    match msg {
        Message::RecvDeviceState { device } => {
            state
                .devices
                .handle_recv_device_state(device, &state.scenes)
                .await
        }
        Message::InternalStateUpdate {
            old_state,
            new_state,
            old,
            new,
        } => {
            let invalidated_device = new;
            debug!("invalidating {name}", name = invalidated_device.name);

            let _groups_invalidated = state
                .groups
                .invalidate(old_state, new_state, &state.devices);

            let _invalidated_scenes = state.scenes.invalidate(
                old_state,
                new_state,
                invalidated_device,
                &state.devices,
                &state.groups,
                state.expr.get_context(),
            );

            // TODO: only invalidate changed devices/groups/scenes in expr context
            state
                .expr
                .invalidate(new_state, &state.groups, &state.scenes);

            state
                .rules
                .handle_internal_state_update(
                    old_state,
                    new_state,
                    old,
                    &state.devices,
                    &state.groups,
                    &state.expr,
                )
                .await;

            state.event_tx.send(Message::WsBroadcastState);

            Ok(())
        }
        Message::SetExpectedState {
            device,
            set_scene,
            skip_send,
        } => {
            state
                .devices
                .set_device_state(device, &state.scenes, *set_scene, false, *skip_send)
                .await;

            Ok(())
        }
        Message::SendDeviceState { device } => {
            state
                .integrations
                .set_integration_device_state(device)
                .await
        }
        Message::WsBroadcastState => {
            state.send_state_ws(None).await;

            Ok(())
        }
        Message::DbStoreScene { scene_id, config } => {
            db_store_scene(scene_id, config).await.ok();
            state.scenes.refresh_db_scenes().await;
            state.send_state_ws(None).await;

            Ok(())
        }
        Message::DbDeleteScene { scene_id } => {
            db_delete_scene(scene_id).await.ok();
            state.scenes.refresh_db_scenes().await;
            state.send_state_ws(None).await;

            Ok(())
        }
        Message::DbEditScene { scene_id, name } => {
            db_edit_scene(scene_id, name).await.ok();
            state.scenes.refresh_db_scenes().await;
            state.send_state_ws(None).await;

            Ok(())
        }
        Message::Action(Action::ActivateScene(SceneDescriptor {
            scene_id,
            device_keys,
            group_keys,
        })) => {
            let eval_context = state.expr.get_context();
            state
                .devices
                .activate_scene(
                    scene_id,
                    device_keys,
                    group_keys,
                    &state.groups,
                    &state.scenes,
                    eval_context,
                )
                .await;

            Ok(())
        }
        Message::Action(Action::CycleScenes(CycleScenesDescriptor { scenes, nowrap })) => {
            let eval_context = state.expr.get_context();
            state
                .devices
                .cycle_scenes(
                    scenes,
                    nowrap.unwrap_or(false),
                    &state.groups,
                    &state.scenes,
                    eval_context,
                )
                .await;

            Ok(())
        }
        Message::Action(Action::Dim(DimDescriptor {
            device_keys,
            group_keys,
            step,
        })) => {
            state
                .devices
                .dim(device_keys, group_keys, step, &state.scenes)
                .await;

            Ok(())
        }
        Message::Action(Action::Custom(CustomActionDescriptor {
            integration_id,
            payload,
        })) => {
            state
                .integrations
                .run_integration_action(integration_id, payload)
                .await
        }
        Message::Action(Action::ForceTriggerRoutine(ForceTriggerRoutineDescriptor {
            routine_id,
        })) => state.rules.force_trigger_routine(routine_id),
        Message::Action(Action::SetDeviceState(device)) => {
            state
                .devices
                .set_device_state(device, &state.scenes, false, false, false)
                .await;

            Ok(())
        }
        Message::Action(Action::EvalExpr(expr)) => {
            let eval_context = state.expr.get_context();
            eval_action_expr(
                expr,
                eval_context,
                state.devices.get_state(),
                &state.event_tx,
            )?;

            Ok(())
        }
    }
}
