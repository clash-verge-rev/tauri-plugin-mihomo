use std::collections::HashMap;

use tauri::{
    State, command,
    ipc::{Channel, InvokeResponseBody},
};

use crate::{Result, mihomo::Mihomo, models::*};

#[command]
pub(crate) fn update_controller(state: State<'_, Mihomo>, host: Option<&str>, port: Option<u16>) -> Result<()> {
    state.update_external_host(host);
    state.update_external_port(port);
    Ok(())
}

#[command]
pub(crate) fn update_secret(state: State<'_, Mihomo>, secret: Option<&str>) -> Result<()> {
    state.update_secret(secret);
    Ok(())
}

#[command]
pub(crate) async fn get_version(state: State<'_, Mihomo>) -> Result<MihomoVersion> {
    state.get_version().await
}

#[command]
pub(crate) async fn flush_fakeip(state: State<'_, Mihomo>) -> Result<()> {
    state.flush_fakeip().await
}

#[command]
pub(crate) async fn flush_dns(state: State<'_, Mihomo>) -> Result<()> {
    state.flush_dns().await
}

// connections
#[command]
pub(crate) async fn get_connections(state: State<'_, Mihomo>) -> Result<Connections> {
    state.get_connections().await
}

#[command]
pub(crate) async fn close_all_connections(state: State<'_, Mihomo>) -> Result<()> {
    state.close_all_connections().await
}

#[command]
pub(crate) async fn close_connection(state: State<'_, Mihomo>, connection_id: String) -> Result<()> {
    state.close_connection(&connection_id).await
}

// groups
#[command]
pub(crate) async fn get_groups(state: State<'_, Mihomo>) -> Result<Groups> {
    state.get_groups().await
}

#[command]
pub(crate) async fn get_group_by_name(state: State<'_, Mihomo>, group_name: String) -> Result<Proxy> {
    state.get_group_by_name(&group_name).await
}

#[command]
pub(crate) async fn delay_group(
    state: State<'_, Mihomo>,
    group_name: String,
    test_url: String,
    timeout: u32,
    keep_fixed: bool,
) -> Result<HashMap<String, u32>> {
    let fixed = if keep_fixed {
        state.get_group_by_name(&group_name).await?.fixed
    } else {
        None
    };
    log::debug!("delay group, fixed: {fixed:?}");
    let res = state.delay_group(&group_name, &test_url, timeout).await?;
    if keep_fixed
        && let Some(fixed) = fixed
        && !fixed.is_empty()
    {
        state.select_node_for_group(&group_name, &fixed).await?;
    }
    Ok(res)
}

// providers
#[command]
pub(crate) async fn get_proxy_providers(state: State<'_, Mihomo>) -> Result<ProxyProviders> {
    state.get_proxy_providers().await
}

#[command]
pub(crate) async fn get_proxy_provider_by_name(
    state: State<'_, Mihomo>,
    provider_name: String,
) -> Result<ProxyProvider> {
    state.get_proxy_provider_by_name(&provider_name).await
}

#[command]
pub(crate) async fn update_proxy_provider(state: State<'_, Mihomo>, provider_name: String) -> Result<()> {
    state.update_proxy_provider(&provider_name).await
}

#[command]
pub(crate) async fn healthcheck_proxy_provider(state: State<'_, Mihomo>, provider_name: String) -> Result<()> {
    state.healthcheck_proxy_provider(&provider_name).await
}

#[command]
pub(crate) async fn healthcheck_node_in_provider(
    state: State<'_, Mihomo>,
    provider_name: String,
    proxy_name: String,
    test_url: String,
    timeout: u32,
) -> Result<ProxyDelay> {
    state
        .healthcheck_node_in_provider(&provider_name, &proxy_name, &test_url, timeout)
        .await
}

// proxies
#[command]
pub(crate) async fn get_proxies(state: State<'_, Mihomo>) -> Result<Proxies> {
    state.get_proxies().await
}

#[command]
pub(crate) async fn get_proxy_by_name(state: State<'_, Mihomo>, proxy_name: String) -> Result<Proxy> {
    state.get_proxy_by_name(&proxy_name).await
}

#[command]
pub(crate) async fn select_node_for_group(state: State<'_, Mihomo>, group_name: String, node: String) -> Result<()> {
    state.select_node_for_group(&group_name, &node).await
}

#[command]
pub(crate) async fn unfixed_proxy(state: State<'_, Mihomo>, group_name: String) -> Result<()> {
    state.unfixed_proxy(&group_name).await
}

#[command]
pub(crate) async fn delay_proxy_by_name(
    state: State<'_, Mihomo>,
    proxy_name: String,
    test_url: String,
    timeout: u32,
) -> Result<ProxyDelay> {
    state.delay_proxy_by_name(&proxy_name, &test_url, timeout).await
}

// rules
#[command]
pub(crate) async fn get_rules(state: State<'_, Mihomo>) -> Result<Rules> {
    state.get_rules().await
}

#[command]
pub(crate) async fn get_rule_providers(state: State<'_, Mihomo>) -> Result<RuleProviders> {
    state.get_rule_providers().await
}

#[command]
pub(crate) async fn update_rule_provider(state: State<'_, Mihomo>, provider_name: String) -> Result<()> {
    state.update_rule_provider(&provider_name).await
}

// runtime config
#[command]
pub(crate) async fn get_base_config(state: State<'_, Mihomo>) -> Result<BaseConfig> {
    state.get_base_config().await
}

#[command]
pub(crate) async fn reload_config(state: State<'_, Mihomo>, force: bool, config_path: String) -> Result<()> {
    state.reload_config(force, &config_path).await
}

#[command]
pub(crate) async fn patch_base_config(state: State<'_, Mihomo>, data: serde_json::Value) -> Result<()> {
    state.patch_base_config(&data).await
}

#[command]
pub(crate) async fn update_geo(state: State<'_, Mihomo>) -> Result<()> {
    state.update_geo().await
}

#[command]
pub(crate) async fn restart(state: State<'_, Mihomo>) -> Result<()> {
    state.restart().await
}

// upgrade
#[command]
pub(crate) async fn upgrade_core(state: State<'_, Mihomo>, channel: CoreUpdaterChannel, force: bool) -> Result<()> {
    state.upgrade_core(channel, force).await
}

#[command]
pub(crate) async fn upgrade_ui(state: State<'_, Mihomo>) -> Result<()> {
    state.upgrade_ui().await
}

#[command]
pub(crate) async fn upgrade_geo(state: State<'_, Mihomo>) -> Result<()> {
    state.upgrade_geo().await
}

// mihomo websocket
#[command]
pub(crate) async fn ws_traffic(
    state: State<'_, Mihomo>,
    on_message: Channel<InvokeResponseBody>,
) -> Result<WsConnectionId> {
    state
        .ws_traffic_checked(move |data| on_message.send(data).is_ok())
        .await
}

#[command]
pub(crate) async fn ws_memory(
    state: State<'_, Mihomo>,
    on_message: Channel<InvokeResponseBody>,
) -> Result<WsConnectionId> {
    state.ws_memory_checked(move |data| on_message.send(data).is_ok()).await
}

#[command]
pub(crate) async fn ws_connections(
    state: State<'_, Mihomo>,
    on_message: Channel<InvokeResponseBody>,
) -> Result<WsConnectionId> {
    state
        .ws_connections_checked(move |data| on_message.send(data).is_ok())
        .await
}

#[command]
pub(crate) async fn ws_logs(
    state: State<'_, Mihomo>,
    level: LogLevel,
    on_message: Channel<InvokeResponseBody>,
) -> Result<WsConnectionId> {
    state
        .ws_logs_checked(level, move |data| on_message.send(data).is_ok())
        .await
}

#[command]
pub(crate) async fn ws_disconnect(
    state: State<'_, Mihomo>,
    id: WsConnectionId,
    force_timeout: Option<u64>,
) -> Result<()> {
    state.disconnect(id, force_timeout).await
}

#[command]
pub(crate) fn clear_all_ws_connections(state: State<'_, Mihomo>) -> Result<()> {
    state.clear_all_ws_connections()
}
