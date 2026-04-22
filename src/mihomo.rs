#![allow(dead_code)]
use std::{collections::HashMap, sync::Arc, time::Duration};

use futures_util::StreamExt;
use http::{
    HeaderMap, HeaderValue, Request,
    header::{AUTHORIZATION, CONNECTION, CONTENT_TYPE, HOST, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE},
};
use reqwest::{Method, RequestBuilder};
use serde_json::json;
use tokio_tungstenite::{
    client_async, connect_async,
    tungstenite::{
        Message, client::IntoClientRequest, handshake::client::generate_key, protocol::CloseFrame as ProtocolCloseFrame,
    },
};

use crate::{
    DOWNLOAD_FILE_TIMEOUT, Error, Result,
    models::{
        BaseConfig, CloseFrame, ConnectionManager, Connections, CoreUpdaterChannel, ErrorResponse, Groups, LogLevel,
        MihomoVersion, NetworkContext, NetworkStatus, Protocol, Proxies, Proxy, ProxyDelay, ProxyProvider,
        ProxyProviders, PutResponse, RuleProviders, Rules, WebSocketConnectionId, WebSocketMessage, WebSocketWriter,
    },
    ret_failed_resp,
};

/// Mihomo REST/WebSocket client.
///
/// `protocol`, `socket_path`, `request_timeout`, and `client` are private
/// because they are bound together: `client` caches a `reqwest::Client` whose
/// connector is built from the other three. Mutate them only through
/// `Mihomo::new` and the `update_*` methods so the cached client stays in
/// sync; do not construct via struct literal.
pub struct Mihomo {
    protocol: Protocol,
    pub external_host: Option<String>,
    pub external_port: Option<u16>,
    pub secret: Option<String>,
    socket_path: Option<String>,
    request_timeout: Duration,
    pub connection_manager: Arc<ConnectionManager>,
    client: reqwest::Client,
}

impl Mihomo {
    /// Build a `reqwest::Client` whose connector matches `protocol`. The
    /// returned client carries a connection pool, so callers should cache and
    /// reuse it across requests rather than rebuild on every call.
    fn build_client(
        protocol: Protocol,
        socket_path: Option<&str>,
        request_timeout: Duration,
    ) -> Result<reqwest::Client> {
        let mut builder = reqwest::ClientBuilder::new().timeout(request_timeout);
        if matches!(protocol, Protocol::LocalSocket) {
            let socket_path = socket_path.ok_or_else(|| {
                log::error!("missing socket path parameter");
                Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "missing socket path".to_string(),
                ))
            })?;
            #[cfg(windows)]
            {
                builder = builder.windows_named_pipe(socket_path);
            }
            #[cfg(unix)]
            {
                builder = builder.unix_socket(socket_path);
            }
        }
        Ok(builder.build()?)
    }

    pub fn new(
        protocol: Protocol,
        external_host: Option<String>,
        external_port: Option<u16>,
        secret: Option<String>,
        socket_path: Option<String>,
        request_timeout: Duration,
    ) -> Result<Self> {
        let client = Self::build_client(protocol, socket_path.as_deref(), request_timeout)?;
        Ok(Self {
            protocol,
            external_host,
            external_port,
            secret,
            socket_path,
            request_timeout,
            connection_manager: Default::default(),
            client,
        })
    }

    /// Switch the underlying transport. On error the previous protocol and
    /// cached client are kept intact (transactional update).
    pub fn update_protocol(&mut self, protocol: Protocol) -> Result<()> {
        let client = Self::build_client(protocol, self.socket_path.as_deref(), self.request_timeout)?;
        self.protocol = protocol;
        self.client = client;
        Ok(())
    }

    #[inline]
    pub fn update_external_host(&mut self, host: Option<String>) {
        self.external_host = host;
    }

    pub fn update_external_port(&mut self, port: Option<u16>) {
        self.external_port = port;
    }

    #[inline]
    pub fn update_secret(&mut self, secret: Option<String>) {
        self.secret = secret;
    }

    /// Update the local-socket path. Rebuilds the cached client transactionally
    /// (on error the previous path and client are kept).
    pub fn update_socket_path(&mut self, socket_path: Option<String>) -> Result<()> {
        let client = Self::build_client(self.protocol, socket_path.as_deref(), self.request_timeout)?;
        self.socket_path = socket_path;
        self.client = client;
        Ok(())
    }

    /// Update the request timeout. Rebuilds the cached client transactionally
    /// (on error the previous timeout and client are kept).
    pub fn update_request_timeout(&mut self, request_timeout: Duration) -> Result<()> {
        let client = Self::build_client(self.protocol, self.socket_path.as_deref(), request_timeout)?;
        self.request_timeout = request_timeout;
        self.client = client;
        Ok(())
    }

    pub fn start_ws_connections_watcher(&self) {
        let manager = Arc::clone(&self.connection_manager);
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(1000));
            loop {
                interval.tick().await;
                let ids_map = manager.0.read().await;
                let ids: Vec<&u32> = ids_map.keys().collect();
                log::trace!("manager websocket connection ids: {ids:?}",);
            }
        });
    }

    #[inline]
    fn get_req_url(&self, suffix_url: &str) -> Result<String> {
        let suffix_url = suffix_url.trim_start_matches("/");
        match self.protocol {
            Protocol::Http => {
                if let Some(host) = self.external_host.as_ref() {
                    let port = self.external_port.unwrap_or(9090);
                    Ok(format!("http://{host}:{port}/{suffix_url}"))
                } else {
                    log::error!("missing external host parameter");
                    Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "missing external host".to_string(),
                    )))
                }
            }
            Protocol::LocalSocket => Ok(format!("http://localhost/{suffix_url}")),
        }
    }

    #[inline]
    fn get_req_headers(&self) -> Result<HeaderMap<HeaderValue>> {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_str("localhost")?);
        headers.insert(CONTENT_TYPE, HeaderValue::from_str("application/json")?);
        if matches!(self.protocol, Protocol::Http)
            && let Some(secret) = &self.secret
        {
            let auth_value = HeaderValue::from_str(&format!("Bearer {secret}"))?;
            headers.insert(AUTHORIZATION, auth_value);
        }
        Ok(headers)
    }

    #[inline]
    fn build_request(&self, method: Method, suffix_url: &str) -> Result<RequestBuilder> {
        let url = self.get_req_url(suffix_url)?;
        let headers = self.get_req_headers()?;
        let client = &self.client;

        match method {
            Method::POST => Ok(client.post(url).headers(headers)),
            Method::GET => Ok(client.get(url).headers(headers)),
            Method::PUT => Ok(client.put(url).headers(headers)),
            Method::PATCH => Ok(client.patch(url).headers(headers)),
            Method::DELETE => Ok(client.delete(url).headers(headers)),
            _ => {
                let method_str = method.as_str().to_string();
                log::error!("method not supported: {method_str}");
                Err(Error::MethodNotSupported(method_str))
            }
        }
    }

    #[inline]
    fn get_websocket_url(&self, suffix_url: &str) -> Result<String> {
        let suffix_url = suffix_url.trim_start_matches("/");
        match self.protocol {
            Protocol::Http => {
                if let Some(host) = self.external_host.as_ref() {
                    let port = self.external_port.unwrap_or(9090);
                    let secret = self.secret.as_deref().unwrap_or_default();
                    Ok(format!("ws://{host}:{port}/{suffix_url}?token={secret}"))
                } else {
                    log::error!("missing external host parameter");
                    Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "missing external host".to_string(),
                    )))
                }
            }
            Protocol::LocalSocket => Ok(format!("ws://localhost/{suffix_url}")),
        }
    }

    /// 连接 WebSocket
    async fn connect<F>(&self, url: String, on_message: F) -> Result<WebSocketConnectionId>
    where
        F: Fn(serde_json::Value) + Send + 'static,
    {
        let id = rand::random();
        log::info!("connecting to websocket: {url}, id: {id}");
        let manager = Arc::clone(&self.connection_manager);
        let handle_message = |message| {
            let serialize_with_fallback = |ws_message: WebSocketMessage| {
                serde_json::to_value(ws_message).unwrap_or_else(|err| {
                    log::error!("Failed to serialize WebSocket message: {err}");
                    serde_json::Value::Null
                })
            };

            match message {
                Ok(Message::Text(t)) => serialize_with_fallback(WebSocketMessage::Text(t.to_string())),
                Ok(Message::Binary(t)) => serialize_with_fallback(WebSocketMessage::Binary(t.to_vec())),
                Ok(Message::Ping(t)) => serialize_with_fallback(WebSocketMessage::Ping(t.to_vec())),
                Ok(Message::Pong(t)) => serialize_with_fallback(WebSocketMessage::Pong(t.to_vec())),
                Ok(Message::Close(t)) => serialize_with_fallback(WebSocketMessage::Close(t.map(|v| CloseFrame {
                    code: v.code.into(),
                    reason: v.reason.to_string(),
                }))),
                Ok(Message::Frame(_)) => serde_json::Value::Null,
                Err(e) => {
                    log::error!("websocket error: {e}");
                    serialize_with_fallback(WebSocketMessage::Text(Error::from(e).to_string()))
                }
            }
        };

        match self.protocol {
            Protocol::Http => {
                log::debug!("starting connect to websocket by using http");
                let request = url.into_client_request()?;
                let (ws_stream, _) = connect_async(request).await?;
                let (writer, mut reader) = ws_stream.split();

                manager
                    .0
                    .write()
                    .await
                    .insert(id, WebSocketWriter::TcpStreamWriter(writer));

                tokio::spawn(async move {
                    let manager_ = Arc::clone(&manager);
                    loop {
                        if !manager_.0.read().await.keys().any(|key| key == &id) {
                            log::debug!("connection [{id}] is removed from manager");
                            break;
                        }
                        if let Some(message) = reader.next().await {
                            if let Ok(Message::Close(_)) = message {
                                log::debug!("connection [{id}] is closed");
                                manager_.0.write().await.remove(&id);
                            }
                            let response = handle_message(message);
                            on_message(response);
                        }
                    }
                });

                Ok(id)
            }
            Protocol::LocalSocket => {
                if let Some(socket_path) = self.socket_path.as_ref() {
                    log::debug!("starting connect to websocket by using local socket: {socket_path}");
                    let stream = crate::wrap_stream::connect_to_socket(socket_path).await?;

                    let request = Request::builder()
                        .uri(url)
                        .header(HOST, "clash-verge")
                        .header(SEC_WEBSOCKET_KEY, generate_key())
                        .header(CONNECTION, "Upgrade")
                        .header(UPGRADE, "websocket")
                        .header(SEC_WEBSOCKET_VERSION, "13")
                        .body(())?;
                    let (ws_stream, _) = client_async(request, stream).await?;
                    let (writer, mut reader) = ws_stream.split();

                    manager
                        .0
                        .write()
                        .await
                        .insert(id, WebSocketWriter::SocketStreamWriter(writer));

                    tokio::spawn(async move {
                        let manager_ = Arc::clone(&manager);
                        loop {
                            if !manager_.0.read().await.keys().any(|key| key == &id) {
                                log::debug!("connection [{id}] is removed from manager");
                                break;
                            }
                            if let Some(message) = reader.next().await {
                                if let Ok(Message::Close(_)) = message {
                                    log::debug!("connection [{id}] closed");
                                    manager_.0.write().await.remove(&id);
                                }
                                let response = handle_message(message);
                                on_message(response);
                            }
                        }
                    });
                    Ok(id)
                } else {
                    log::error!("missing socket path parameter");
                    Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "missing socket path".to_string(),
                    )))
                }
            }
        }
    }

    /// 向指定 WebSocket 连接发送消息 (暂无使用该方法的地方)
    async fn send(&self, id: WebSocketConnectionId, message: WebSocketMessage) -> Result<()> {
        let manager = Arc::clone(&self.connection_manager);
        let mut manager = manager.0.write().await;
        if let Some(writer) = manager.get_mut(&id) {
            let data = match message {
                WebSocketMessage::Text(t) => Message::Text(t.into()),
                WebSocketMessage::Binary(t) => Message::Binary(t.into()),
                WebSocketMessage::Ping(t) => Message::Ping(t.into()),
                WebSocketMessage::Pong(t) => Message::Pong(t.into()),
                WebSocketMessage::Close(t) => Message::Close(t.map(|v| ProtocolCloseFrame {
                    code: v.code.into(),
                    reason: v.reason.into(),
                })),
            };
            writer.send(data).await?;
            Ok(())
        } else {
            log::error!("connection not found: {id}");
            Err(Error::WebSocketConnectionNotFound(id))
        }
    }

    /// 取消 WebSocket 连接
    pub async fn disconnect(&self, id: WebSocketConnectionId, force_timeout: Option<u64>) -> Result<()> {
        log::debug!("disconnecting connection: {id}");
        let mut manager = self.connection_manager.0.write().await;
        if let Some(writer) = manager.get_mut(&id) {
            let close_message = Message::Close(Some(ProtocolCloseFrame {
                code: 1000.into(),
                reason: "Disconnected by client".into(),
            }));
            // ignore send error
            let _ = writer.send(close_message).await;
            if let Some(timeout) = force_timeout {
                let manager_ = Arc::clone(&self.connection_manager);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(timeout)).await;
                    log::debug!("force close websocket connection");
                    manager_.0.write().await.remove(&id);
                });
            }
            Ok(())
        } else {
            log::error!("connection not found: {id}");
            Err(Error::WebSocketConnectionNotFound(id))
        }
    }

    pub async fn clear_all_ws_connections(&self) -> Result<()> {
        log::debug!("start to clear all websocket connections");
        let mut manager = self.connection_manager.0.write().await;
        log::debug!("manage_ids: {:?}", manager.keys());
        manager.clear();
        log::debug!("clear all done, manager_ids: {:?}", manager.keys());
        drop(manager);
        Ok(())
    }

    // ------------------------------------------------------
    // |                     Mihomo API                     |
    // ------------------------------------------------------
    /// WebSocket: Mihomo 流量数据
    pub async fn ws_traffic<F>(&self, on_message: F) -> Result<WebSocketConnectionId>
    where
        F: Fn(serde_json::Value) + Send + 'static,
    {
        let ws_url = self.get_websocket_url("/traffic")?;
        let websocket_id = self.connect(ws_url, on_message).await?;
        Ok(websocket_id)
    }

    /// WebSocket: Mihomo 内存使用数据
    pub async fn ws_memory<F>(&self, on_message: F) -> Result<WebSocketConnectionId>
    where
        F: Fn(serde_json::Value) + Send + 'static,
    {
        let ws_url = self.get_websocket_url("/memory")?;
        let websocket_id = self.connect(ws_url, on_message).await?;
        Ok(websocket_id)
    }

    /// WebSocket: Mihomo 连接信息数据
    pub async fn ws_connections<F>(&self, on_message: F) -> Result<WebSocketConnectionId>
    where
        F: Fn(serde_json::Value) + Send + 'static,
    {
        let ws_url = self.get_websocket_url("/connections")?;
        let websocket_id = self.connect(ws_url, on_message).await?;
        Ok(websocket_id)
    }

    /// WebSocket: Mihomo 日志数据
    pub async fn ws_logs<F>(&self, level: LogLevel, on_message: F) -> Result<WebSocketConnectionId>
    where
        F: Fn(serde_json::Value) + Send + 'static,
    {
        let ws_url = self.get_websocket_url("/logs")?;
        let ws_url = match self.protocol {
            // url 后面添加 format=structured 参数的日志格式如下：
            // {"time":"11:49:58","level":"debug","message":"[DNS] hijack udp:192.168.2.1:53 from 198.18.0.1:42761","fields":[]}
            Protocol::Http => format!("{ws_url}&level={level}"),
            Protocol::LocalSocket => format!("{ws_url}?level={level}"),
        };
        let websocket_id = self.connect(ws_url, on_message).await?;
        Ok(websocket_id)
    }

    // clash api
    /// 获取 Mihomo 版本信息
    pub async fn get_version(&self) -> Result<MihomoVersion> {
        let response = self.build_request(Method::GET, "/version")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("get mihomo version failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<MihomoVersion>().await?)
    }

    /// 清理 FakeIP 缓存
    pub async fn flush_fakeip(&self) -> Result<()> {
        let response = self.build_request(Method::POST, "/cache/fakeip/flush")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("flush fakeip cache failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 清理 DNS 缓存
    pub async fn flush_dns(&self) -> Result<()> {
        let response = self.build_request(Method::POST, "/cache/dns/flush")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response
                .json::<ErrorResponse>()
                .await
                .map_or_else(|e| format!("flush dns cache failed, {}", e), |err_res| err_res.message);
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 获取全部连接信息
    pub async fn get_connections(&self) -> Result<Connections> {
        let response = self.build_request(Method::GET, "/connections")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("get all connections failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<Connections>().await?)
    }

    /// 关闭全部连接
    pub async fn close_all_connections(&self) -> Result<()> {
        let response = self.build_request(Method::DELETE, "/connections")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("close all connections failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 关闭指定 ID 的连接
    pub async fn close_connection(&self, connection_id: &str) -> Result<()> {
        let response = self
            .build_request(Method::DELETE, &format!("/connections/{connection_id}"))?
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response
                .json::<ErrorResponse>()
                .await
                .map_or_else(|e| format!("close connection failed, {}", e), |err_res| err_res.message);
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 获取所有的代理组
    pub async fn get_groups(&self) -> Result<Groups> {
        let response = self.build_request(Method::GET, "/group")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response
                .json::<ErrorResponse>()
                .await
                .map_or_else(|e| format!("get all groups failed, {}", e), |err_res| err_res.message);
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<Groups>().await?)
    }

    /// 获取指定名称的代理组
    pub async fn get_group_by_name(&self, group_name: &str) -> Result<Proxy> {
        let group_name_encode = urlencoding::encode(group_name);
        let response = self
            .build_request(Method::GET, &format!("/group/{group_name_encode}"))?
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("get group[{}] failed, {}", group_name, e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<Proxy>().await?)
    }

    /// 对指定代理组进行延迟测试, 同时清理代理组已固定的节点
    pub async fn delay_group(&self, group_name: &str, test_url: &str, timeout: u32) -> Result<HashMap<String, u32>> {
        let group_name_encode = urlencoding::encode(group_name);
        let test_url = urlencoding::encode(test_url);
        let suffix_url = format!("/group/{group_name_encode}/delay?url={test_url}&timeout={timeout}");
        let req_timeout = Duration::from_millis(timeout as u64) + self.request_timeout;
        let response = self
            .build_request(Method::GET, &suffix_url)?
            .timeout(req_timeout)
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("delay group[{}] failed, {}", group_name, e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<HashMap<String, u32>>().await?)
    }

    /// 获取代理提供者信息
    pub async fn get_proxy_providers(&self) -> Result<ProxyProviders> {
        let response = self.build_request(Method::GET, "/providers/proxies")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("get all proxy providers failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<ProxyProviders>().await?)
    }

    /// 获取指定代理提供者信息
    pub async fn get_proxy_provider_by_name(&self, provider_name: &str) -> Result<ProxyProvider> {
        let provider_name_encode = urlencoding::encode(provider_name);
        let response = self
            .build_request(Method::GET, &format!("/providers/proxies/{provider_name_encode}"))?
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("get proxy provider[{}] failed, {}", provider_name, e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<ProxyProvider>().await?)
    }

    /// 更新指定代理提供者信息
    pub async fn update_proxy_provider(&self, provider_name: &str) -> Result<()> {
        let provider_name_encode = urlencoding::encode(provider_name);
        let response = self
            .build_request(Method::PUT, &format!("/providers/proxies/{provider_name_encode}"))?
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("update proxy provider[{}] failed, {}", provider_name, e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 对指定代理提供者进行健康检查
    pub async fn healthcheck_proxy_provider(&self, provider_name: &str) -> Result<()> {
        let provider_name_encode = urlencoding::encode(provider_name);
        let response = self
            .build_request(
                Method::GET,
                &format!("/providers/proxies/{provider_name_encode}/healthcheck"),
            )?
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("healthcheck proxy provider[{}] failed, {}", provider_name, e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 对指定代理提供者下的指定节点（非代理组）进行健康检查, 并返回新的延迟信息
    pub async fn healthcheck_node_in_provider(
        &self,
        provider_name: &str,
        proxy_name: &str,
        test_url: &str,
        timeout: u32,
    ) -> Result<ProxyDelay> {
        let provider_name_encode = urlencoding::encode(provider_name);
        let proxy_name_encode = urlencoding::encode(proxy_name);
        let req_timeout = Duration::from_millis(timeout as u64) + self.request_timeout;
        let response = self
            .build_request(
                Method::GET,
                &format!("/providers/proxies/{provider_name_encode}/{proxy_name_encode}/healthcheck"),
            )?
            .query(&[("url", test_url), ("timeout", &timeout.to_string())])
            .timeout(req_timeout)
            .send()
            .await?;
        if !response.status().is_success() {
            // maybe proxy delay is timeout response, try parse it.
            match response.json::<ErrorResponse>().await {
                Ok(err_res) => {
                    log::debug!("healthcheck node[{}] error: {}", proxy_name, err_res.message);
                    return Ok(ProxyDelay { delay: 0 });
                }
                Err(e) => {
                    ret_failed_resp!("healthcheck node[{}] failed, {}", proxy_name, e);
                }
            }
        }
        Ok(response.json::<ProxyDelay>().await?)
    }

    /// 获取所有代理信息
    pub async fn get_proxies(&self) -> Result<Proxies> {
        let response = self.build_request(Method::GET, "/proxies")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response
                .json::<ErrorResponse>()
                .await
                .map_or_else(|e| format!("get all proxies failed, {}", e), |err_res| err_res.message);
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<Proxies>().await?)
    }

    /// 获取指定代理信息
    pub async fn get_proxy_by_name(&self, proxy_name: &str) -> Result<Proxy> {
        let proxy_name_encode = urlencoding::encode(proxy_name);
        let response = self
            .build_request(Method::GET, &format!("/proxies/{proxy_name_encode}"))?
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("get proxy[{}] failed, {}", proxy_name, e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<Proxy>().await?)
    }

    /// 为指定代理选择节点
    ///
    /// 一般为指定代理组下使用指定的代理节点 【代理组/节点】
    pub async fn select_node_for_group(&self, group_name: &str, node: &str) -> Result<()> {
        let group_name_encode = urlencoding::encode(group_name);
        let response = self
            .build_request(Method::PUT, &format!("/proxies/{group_name_encode}"))?
            .json(&json!({ "name": node }))
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("select node[{}] for group[{}] failed, {}", node, group_name, e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 指定代理组下不再使用固定的代理节点
    ///
    /// 一般用于自动选择的代理组（例如：URLTest 类型的代理组）下的节点
    pub async fn unfixed_proxy(&self, group_name: &str) -> Result<()> {
        let group_name_encode = urlencoding::encode(group_name);
        let response = self
            .build_request(Method::DELETE, &format!("/proxies/{group_name_encode}"))?
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("unfixed group[{}] failed, {}", group_name, e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 对指定代理进行延迟测试
    ///
    /// 一般用于代理节点的延迟测试，也可传代理组名称（只会测试代理组下选中的代理节点）
    pub async fn delay_proxy_by_name(&self, proxy_name: &str, test_url: &str, timeout: u32) -> Result<ProxyDelay> {
        let proxy_name_encode = urlencoding::encode(proxy_name);
        let req_timeout = Duration::from_millis(timeout as u64) + self.request_timeout;
        let response = self
            .build_request(Method::GET, &format!("/proxies/{proxy_name_encode}/delay"))?
            .query(&[("timeout", &timeout.to_string()), ("url", &test_url.to_string())])
            .timeout(req_timeout)
            .send()
            .await?;
        if !response.status().is_success() {
            match response.json::<ErrorResponse>().await {
                Ok(err_res) => {
                    log::debug!(
                        "delay proxy[{}], mark it timeout, response error message: {}",
                        proxy_name,
                        err_res.message
                    );
                    return Ok(ProxyDelay { delay: 0 });
                }
                Err(e) => {
                    ret_failed_resp!("delay proxy[{}] failed, {}", proxy_name, e);
                }
            }
        }
        Ok(response.json::<ProxyDelay>().await?)
    }

    /// 获取所有规则信息
    pub async fn get_rules(&self) -> Result<Rules> {
        let response = self.build_request(Method::GET, "/rules")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response
                .json::<ErrorResponse>()
                .await
                .map_or_else(|e| format!("get all rules failed, {}", e), |err_res| err_res.message);
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<Rules>().await?)
    }

    /// 获取所有规则提供者信息
    pub async fn get_rule_providers(&self) -> Result<RuleProviders> {
        let response = self.build_request(Method::GET, "/providers/rules")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("get all rule providers failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<RuleProviders>().await?)
    }

    /// 更新规则提供者信息
    pub async fn update_rule_provider(&self, provider_name: &str) -> Result<()> {
        let provider_name_encode = urlencoding::encode(provider_name);
        let response = self
            .build_request(Method::PUT, &format!("/providers/rules/{provider_name_encode}"))?
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("update rule provider[{}] failed, {}", provider_name, e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 获取基础配置
    pub async fn get_base_config(&self) -> Result<BaseConfig> {
        let response = self.build_request(Method::GET, "/configs")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response
                .json::<ErrorResponse>()
                .await
                .map_or_else(|e| format!("get base config failed, {}", e), |err_res| err_res.message);
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<BaseConfig>().await?)
    }

    /// 重新加载配置
    ///
    /// 如果配置文件中包含了很多 provider，则需等待 provider 下载完成 (如果网络不好则导致此方法耗时)
    pub async fn reload_config(&self, force: bool, config_path: &str) -> Result<()> {
        let response = self
            .build_request(Method::PUT, "/configs")?
            .query(&[("force", force)])
            .json(&json!({ "path": config_path }))
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("reload base config failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 更新基础配置
    pub async fn patch_base_config<D: serde::Serialize + Clone + Sync>(&self, data: &D) -> Result<()> {
        let response = self.build_request(Method::PATCH, "/configs")?.json(data).send().await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("patch base config failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 更新 Geo, 同 [`upgrade_geo`](crate::mihomo::Mihomo::upgrade_geo)
    pub async fn update_geo(&self) -> Result<()> {
        let response = self
            .build_request(Method::POST, "/configs/geo")?
            .timeout(DOWNLOAD_FILE_TIMEOUT)
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("update geo database failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 重启核心
    pub async fn restart(&self) -> Result<()> {
        let response = self.build_request(Method::POST, "/restart")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response
                .json::<ErrorResponse>()
                .await
                .map_or_else(|e| format!("restart core failed, {}", e), |err_res| err_res.message);
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 升级核心
    pub async fn upgrade_core(&self, channel: CoreUpdaterChannel, force: bool) -> Result<()> {
        let response = self
            .build_request(Method::POST, "/upgrade")?
            .timeout(DOWNLOAD_FILE_TIMEOUT)
            .query(&[("channel", &channel.to_string()), ("force", &force.to_string())])
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("upgrade core failed, {}", e),
                |err_res| {
                    let msg = err_res.message;
                    if msg.to_lowercase().contains("already using latest version") {
                        "already using latest version".to_string()
                    } else {
                        msg
                    }
                },
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 更新 UI
    pub async fn upgrade_ui(&self) -> Result<()> {
        let response = self
            .build_request(Method::POST, "/upgrade/ui")?
            .timeout(DOWNLOAD_FILE_TIMEOUT)
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response
                .json::<ErrorResponse>()
                .await
                .map_or_else(|e| format!("upgrade ui failed, {}", e), |err_res| err_res.message);
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 更新 Geo
    pub async fn upgrade_geo(&self) -> Result<()> {
        let response = self
            .build_request(Method::POST, "/upgrade/geo")?
            .timeout(DOWNLOAD_FILE_TIMEOUT)
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("upgrade geo database failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    // network policy

    /// 推送网络上下文，触发 network-policy 状态机
    pub async fn put_network_context(&self, ctx: &NetworkContext) -> Result<PutResponse> {
        let response = self
            .build_request(Method::PUT, "/network/context")?
            .json(ctx)
            .send()
            .await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("put network context failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<PutResponse>().await?)
    }

    /// 清除网络上下文
    pub async fn delete_network_context(&self) -> Result<()> {
        let response = self.build_request(Method::DELETE, "/network/context")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("delete network context failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(())
    }

    /// 获取网络上下文与各组应用状态
    pub async fn get_network_context(&self) -> Result<NetworkStatus> {
        let response = self.build_request(Method::GET, "/network/context")?.send().await?;
        if !response.status().is_success() {
            let err_msg = response.json::<ErrorResponse>().await.map_or_else(
                |e| format!("get network context failed, {}", e),
                |err_res| err_res.message,
            );
            ret_failed_resp!("{}", err_msg);
        }
        Ok(response.json::<NetworkStatus>().await?)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn http_mihomo() -> Mihomo {
        Mihomo::new(
            Protocol::Http,
            Some("127.0.0.1".into()),
            Some(9090),
            None,
            None,
            Duration::from_secs(1),
        )
        .expect("http mihomo should construct")
    }

    fn local_socket_path() -> String {
        if cfg!(windows) {
            r"\\.\pipe\verge-mihomo".into()
        } else {
            "/tmp/verge-mihomo.sock".into()
        }
    }

    #[test]
    fn local_socket_requires_socket_path() {
        let result = Mihomo::new(
            Protocol::LocalSocket,
            None,
            None,
            None,
            None,
            Duration::from_secs(1),
        );
        assert!(result.is_err(), "LocalSocket without socket_path must fail");
    }

    #[test]
    fn update_protocol_to_local_socket_without_path_rolls_back() {
        // Transactional update: when build_client fails, the previous
        // protocol must remain in place so the client and protocol stay
        // consistent.
        let mut m = http_mihomo();
        let result = m.update_protocol(Protocol::LocalSocket);
        assert!(result.is_err(), "switching to LocalSocket without socket_path must fail");
        assert_eq!(m.protocol, Protocol::Http, "protocol must remain Http on rebuild error");
    }

    #[test]
    fn update_socket_path_persists_new_path() {
        let mut m = http_mihomo();
        m.update_socket_path(Some(local_socket_path()))
            .expect("update_socket_path should succeed");
        assert_eq!(m.socket_path.as_deref(), Some(local_socket_path().as_str()));
    }

    #[test]
    fn update_protocol_to_local_socket_with_path_succeeds() {
        // Stage a socket_path under Http first, then switch protocol.
        let mut m = http_mihomo();
        m.update_socket_path(Some(local_socket_path())).expect("stage socket path");
        m.update_protocol(Protocol::LocalSocket)
            .expect("switch to LocalSocket with socket_path should succeed");
        assert_eq!(m.protocol, Protocol::LocalSocket);
        assert_eq!(m.socket_path.as_deref(), Some(local_socket_path().as_str()));
    }

    #[test]
    fn clearing_socket_path_under_local_socket_rolls_back() {
        // Removing socket_path while still on LocalSocket must fail and keep
        // the previous path/client intact.
        let mut m = Mihomo::new(
            Protocol::LocalSocket,
            None,
            None,
            None,
            Some(local_socket_path()),
            Duration::from_secs(1),
        )
        .expect("local socket mihomo should construct");
        let result = m.update_socket_path(None);
        assert!(result.is_err(), "clearing socket_path under LocalSocket must fail");
        assert_eq!(m.protocol, Protocol::LocalSocket);
        assert_eq!(m.socket_path.as_deref(), Some(local_socket_path().as_str()));
    }

    #[test]
    fn update_request_timeout_persists() {
        let mut m = http_mihomo();
        m.update_request_timeout(Duration::from_secs(2))
            .expect("timeout change should succeed");
        assert_eq!(m.request_timeout, Duration::from_secs(2));
    }

    // ---------------- network policy ----------------

    use crate::models::{AppliedGroup, GroupStatus, InterfaceContext};

    fn sample_iface_full() -> InterfaceContext {
        InterfaceContext {
            name: "wlan0".into(),
            iface_type: Some("wifi".into()),
            ssid: Some("office-5g".into()),
            bssid: Some("aa:bb:cc:dd:ee:00".into()),
            gateway_ip: Some("10.0.0.1".into()),
            gateway_mac: Some("11:22:33:44:55:66".into()),
            subnets: Some(vec!["10.0.0.0/24".into()]),
            metered: Some(false),
        }
    }

    #[test]
    fn network_context_round_trip_full() {
        // Full context with one full-field iface and one minimal iface.
        let ctx = NetworkContext {
            version: 1,
            interfaces: vec![
                sample_iface_full(),
                InterfaceContext {
                    name: "wg0".into(),
                    iface_type: Some("vpn".into()),
                    ssid: None,
                    bssid: None,
                    gateway_ip: None,
                    gateway_mac: None,
                    subnets: None,
                    metered: None,
                },
            ],
            dns_suffix: Some(vec!["corp.example.com".into()]),
            ttl: Some(1800),
        };
        let v = serde_json::to_value(&ctx).expect("serialize");
        assert_eq!(v["version"], 1);
        assert_eq!(v["interfaces"][0]["name"], "wlan0");
        assert_eq!(v["interfaces"][0]["iface_type"], "wifi");
        assert_eq!(v["interfaces"][0]["gateway_ip"], "10.0.0.1");
        assert_eq!(v["interfaces"][0]["gateway_mac"], "11:22:33:44:55:66");
        assert_eq!(v["dns_suffix"], serde_json::json!(["corp.example.com"]));
        assert_eq!(v["ttl"], 1800);
        // Minimal iface drops skip_serializing_if fields entirely.
        let iface1 = v["interfaces"][1].as_object().expect("iface1 object");
        assert!(iface1.contains_key("name"));
        assert!(iface1.contains_key("iface_type"));
        for k in &["ssid", "bssid", "gateway_ip", "gateway_mac", "subnets", "metered"] {
            assert!(!iface1.contains_key(*k), "iface1 unexpectedly has {}", k);
        }
        // Round-trip equality.
        let back: NetworkContext = serde_json::from_value(v).expect("deserialize");
        assert_eq!(back, ctx);
    }

    #[test]
    fn network_context_serializes_empty_interfaces_array() {
        // `interfaces` is wire-required: empty Vec must serialize as [], not be skipped.
        let ctx = NetworkContext {
            version: 1,
            interfaces: vec![],
            dns_suffix: None,
            ttl: None,
        };
        let v = serde_json::to_value(&ctx).expect("serialize");
        let obj = v.as_object().expect("object");
        assert!(obj.contains_key("interfaces"), "interfaces key must be present");
        assert_eq!(v["interfaces"], serde_json::json!([]));
        assert!(!obj.contains_key("dns_suffix"), "dns_suffix=None must be skipped");
        assert!(!obj.contains_key("ttl"), "ttl=None must be skipped");
    }

    #[test]
    fn dns_suffix_must_be_array_not_scalar() {
        // Scalar-form dns_suffix must fail-fast on deserialize; mihomo
        // rejects scalars wire-side, and the plugin's type refuses them at
        // the Rust boundary to catch bugs earlier.
        let result: serde_json::Result<NetworkContext> = serde_json::from_value(serde_json::json!({
            "version": 1,
            "interfaces": [],
            "dns_suffix": "corp.example.com",
        }));
        assert!(result.is_err(), "scalar dns_suffix must not deserialize");
    }

    #[test]
    fn camelcase_keys_are_silently_ignored_on_deserialize() {
        // Locking this down prevents anyone from quietly re-introducing
        // `alias = "ifaceType"` etc. Go's json.Unmarshal does not collapse
        // case across underscores, so camelCase keys on the wire would be
        // silently dropped by mihomo; the plugin mirrors that by ignoring
        // them on deserialize (only snake_case is recognized).
        let ctx: NetworkContext = serde_json::from_value(serde_json::json!({
            "version": 1,
            "interfaces": [{
                "name": "wlan0",
                "ifaceType": "wifi",
                "gatewayIp": "10.0.0.1",
            }],
        }))
        .expect("deserialize ignores unknown camelCase keys");
        assert_eq!(ctx.interfaces.len(), 1);
        assert_eq!(ctx.interfaces[0].name, "wlan0");
        assert!(
            ctx.interfaces[0].iface_type.is_none(),
            "ifaceType must NOT be recognized"
        );
        assert!(
            ctx.interfaces[0].gateway_ip.is_none(),
            "gatewayIp must NOT be recognized"
        );
    }

    #[test]
    fn unknown_top_level_fields_are_ignored_forward_compat() {
        // Forward-compat: mihomo may add additive wire fields later.
        let ctx: NetworkContext = serde_json::from_value(serde_json::json!({
            "version": 1,
            "interfaces": [{"name": "en0"}],
            "future_field": 42,
            "another_unknown": {"nested": true},
        }))
        .expect("unknown top-level fields are ignored");
        assert_eq!(ctx.version, 1);
        assert_eq!(ctx.interfaces[0].name, "en0");
    }

    #[test]
    fn interface_metered_tristate_round_trip() {
        let cases = [(Some(true), Some(true)), (Some(false), Some(false)), (None, None)];
        for (input, expected) in cases {
            let iface = InterfaceContext {
                name: "en0".into(),
                iface_type: None,
                ssid: None,
                bssid: None,
                gateway_ip: None,
                gateway_mac: None,
                subnets: None,
                metered: input,
            };
            let v = serde_json::to_value(&iface).expect("serialize");
            match input {
                Some(true) => assert_eq!(v["metered"], true),
                Some(false) => assert_eq!(v["metered"], false),
                None => assert!(
                    !v.as_object().expect("object").contains_key("metered"),
                    "metered=None must be skipped",
                ),
            }
            let back: InterfaceContext = serde_json::from_value(v).expect("deserialize");
            assert_eq!(back.metered, expected);
        }
    }

    #[test]
    fn interface_subnets_three_states() {
        let with_items = InterfaceContext {
            name: "en0".into(),
            iface_type: None,
            ssid: None,
            bssid: None,
            gateway_ip: None,
            gateway_mac: None,
            subnets: Some(vec!["10.0.0.0/24".into()]),
            metered: None,
        };
        let empty_vec = InterfaceContext {
            name: "en0".into(),
            iface_type: None,
            ssid: None,
            bssid: None,
            gateway_ip: None,
            gateway_mac: None,
            subnets: Some(vec![]),
            metered: None,
        };
        let none = InterfaceContext {
            name: "en0".into(),
            iface_type: None,
            ssid: None,
            bssid: None,
            gateway_ip: None,
            gateway_mac: None,
            subnets: None,
            metered: None,
        };
        let v1 = serde_json::to_value(&with_items).expect("serialize");
        assert_eq!(v1["subnets"], serde_json::json!(["10.0.0.0/24"]));
        let v2 = serde_json::to_value(&empty_vec).expect("serialize");
        // Option<Vec>::Some(empty) is NOT skipped by skip_serializing_if (it
        // only checks Option::is_none), so the empty array must appear.
        assert_eq!(v2["subnets"], serde_json::json!([]));
        let v3 = serde_json::to_value(&none).expect("serialize");
        assert!(
            !v3.as_object().expect("object").contains_key("subnets"),
            "subnets=None must be skipped",
        );
    }

    #[test]
    fn put_response_deserializes_snake_serializes_camel() {
        let resp: PutResponse = serde_json::from_value(serde_json::json!({
            "matched_network": "office",
            "applied": [{
                "group": "Smart",
                "target_proxy": "hk",
                "applied_proxy": "hk",
                "changed": true,
                "selection_source": "auto",
                "reason": "matched",
            }],
            "expires_at": 1_700_000_000_i64,
        }))
        .expect("deserialize from mihomo snake_case");
        assert_eq!(resp.matched_network.as_deref(), Some("office"));
        assert_eq!(resp.applied.len(), 1);
        assert_eq!(resp.applied[0].target_proxy.as_deref(), Some("hk"));
        assert_eq!(resp.applied[0].applied_proxy, "hk");
        assert_eq!(resp.expires_at, Some(1_700_000_000));

        let v = serde_json::to_value(&resp).expect("serialize");
        assert!(v.get("matchedNetwork").is_some(), "serialize must be camelCase");
        assert!(v.get("matched_network").is_none(), "snake_case keys must NOT appear");
        assert!(v.get("expiresAt").is_some());
        let applied0 = &v["applied"][0];
        assert!(applied0.get("targetProxy").is_some());
        assert!(applied0.get("appliedProxy").is_some());
        assert!(applied0.get("selectionSource").is_some());
    }

    #[test]
    fn network_status_round_trip_with_nested_context_snake_case() {
        // mihomo serves snake_case for the outer NetworkStatus and for the
        // embedded `context` (NetworkContext). The plugin preserves the
        // outer/inner asymmetry when it serializes back for the frontend:
        // outer fields become camelCase (host↔TS boundary), but the nested
        // `context` stays snake_case so fields keep their wire names.
        // Note: the GET response's context does NOT echo `ttl` (normalized
        // snapshot per the mihomo /network/context contract), so the fixture
        // below omits it; the outer `expiresAt` carries the expiry.
        let status: NetworkStatus = serde_json::from_value(serde_json::json!({
            "context": {
                "version": 1,
                "interfaces": [{
                    "name": "wlan0",
                    "iface_type": "wifi",
                    "ssid": "office-5g",
                }],
                "dns_suffix": ["corp.example.com"],
            },
            "matched_network": "office",
            "groups": [{
                "group": "auto",
                "current_proxy": "hk",
                "selection_source": "auto",
                "last_matched_network": "office",
            }],
            "expires_at": 1_700_000_000_i64,
            "age_seconds": 42_i64,
        }))
        .expect("deserialize NetworkStatus");
        let ctx = status.context.as_ref().expect("context present");
        assert_eq!(ctx.version, 1);
        assert_eq!(ctx.interfaces[0].iface_type.as_deref(), Some("wifi"));

        let v = serde_json::to_value(&status).expect("serialize");
        // Outer: camelCase.
        assert!(v.get("matchedNetwork").is_some());
        assert!(v.get("expiresAt").is_some());
        assert!(v.get("ageSeconds").is_some());
        // Inner context: snake_case preserved (rename_all is NOT recursive).
        let inner = &v["context"];
        assert!(inner.get("interfaces").is_some());
        let iface0 = &inner["interfaces"][0];
        assert_eq!(iface0["iface_type"], "wifi");
        assert!(iface0.get("ifaceType").is_none(), "inner must NOT become camelCase");
        assert!(inner.get("dns_suffix").is_some());
        assert!(inner.get("dnsSuffix").is_none());
        // Inner group: camelCase (GroupStatus is a response type, not a
        // double-direction type like NetworkContext).
        let g0 = &v["groups"][0];
        assert!(g0.get("currentProxy").is_some());
        assert!(g0.get("selectionSource").is_some());
        assert!(g0.get("lastMatchedNetwork").is_some());
    }

    #[test]
    fn group_status_last_matched_network_two_wire_states() {
        // On the wire `last_matched_network` is either a concrete name or
        // JSON null. The internal sentinel `<none>` is never emitted as a
        // literal string by mihomo (it's encoded as null). `selection_source`
        // partially disambiguates null (`unknown` = never evaluated, `auto` =
        // evaluated-no-match); `manual + null` stays ambiguous by design.
        for last in [Some("office".to_string()), None] {
            let g = GroupStatus {
                group: "auto".into(),
                current_proxy: "hk".into(),
                selection_source: "auto".into(),
                last_matched_network: last.clone(),
            };
            let v = serde_json::to_value(&g).expect("serialize");
            match &last {
                Some(s) => assert_eq!(v["lastMatchedNetwork"], serde_json::Value::String(s.clone())),
                None => assert_eq!(v["lastMatchedNetwork"], serde_json::Value::Null),
            }
            let wire = serde_json::json!({
                "group": "auto",
                "current_proxy": "hk",
                "selection_source": "auto",
                "last_matched_network": last,
            });
            let back: GroupStatus = serde_json::from_value(wire).expect("deserialize");
            assert_eq!(back.last_matched_network, last);
        }
    }

    #[test]
    fn applied_group_reason_seven_literals_round_trip() {
        // All 7 mihomo reason values must round-trip as String without a
        // plugin-side enum (keeps the plugin independent of kernel reason
        // additions).
        let reasons = [
            "matched",
            "already_selected",
            "default",
            "no_change_no_default",
            "unchanged_network",
            "manual_locked",
            "missing_target",
        ];
        for reason in reasons {
            let ag: AppliedGroup = serde_json::from_value(serde_json::json!({
                "group": "auto",
                "target_proxy": null,
                "applied_proxy": "hk",
                "changed": false,
                "selection_source": "auto",
                "reason": reason,
            }))
            .expect("deserialize");
            assert_eq!(ag.reason, reason);
            let v = serde_json::to_value(&ag).expect("serialize");
            assert_eq!(v["reason"], reason);
        }
    }

    #[test]
    fn put_response_expires_at_null_and_concrete() {
        let null_case: PutResponse = serde_json::from_value(serde_json::json!({
            "matched_network": null,
            "applied": [],
            "expires_at": null,
        }))
        .expect("deserialize null expires_at");
        assert!(null_case.expires_at.is_none());
        let v_null = serde_json::to_value(&null_case).expect("serialize");
        // expires_at has no skip_serializing_if — null must be explicitly
        // present so the frontend gets the "sticky" signal.
        assert_eq!(v_null["expiresAt"], serde_json::Value::Null);

        let concrete: PutResponse = serde_json::from_value(serde_json::json!({
            "matched_network": "office",
            "applied": [],
            "expires_at": 1_713_401_800_i64,
        }))
        .expect("deserialize concrete expires_at");
        assert_eq!(concrete.expires_at, Some(1_713_401_800));
    }

    #[test]
    fn network_status_no_context_round_trip() {
        // mihomo returns null context / null matched_network / null
        // expires_at / null age_seconds when no ctx is present; groups[]
        // still carries one entry per network-policy group.
        let status: NetworkStatus = serde_json::from_value(serde_json::json!({
            "context": null,
            "matched_network": null,
            "groups": [{
                "group": "auto",
                "current_proxy": "hk",
                "selection_source": "unknown",
                "last_matched_network": null,
            }],
            "expires_at": null,
            "age_seconds": null,
        }))
        .expect("deserialize no-ctx status");
        assert!(status.context.is_none());
        assert!(status.matched_network.is_none());
        assert_eq!(status.groups.len(), 1);
        assert!(status.expires_at.is_none());
        assert!(status.age_seconds.is_none());

        let v = serde_json::to_value(&status).expect("serialize");
        assert_eq!(v["context"], serde_json::Value::Null);
        assert_eq!(v["matchedNetwork"], serde_json::Value::Null);
        assert_eq!(v["expiresAt"], serde_json::Value::Null);
        assert_eq!(v["ageSeconds"], serde_json::Value::Null);
    }

    #[test]
    fn interface_gateway_combo_caller_responsibility() {
        // Plugin is pass-through: it does NOT enforce "gateway_mac requires
        // gateway_ip" — mihomo server-side returns invalid_gateway_combo on
        // such input. Document that plugin emits exactly what caller gives
        // it, including the illegal combination.
        let illegal = InterfaceContext {
            name: "en0".into(),
            iface_type: Some("ethernet".into()),
            ssid: None,
            bssid: None,
            gateway_ip: None,
            gateway_mac: Some("11:22:33:44:55:66".into()),
            subnets: None,
            metered: None,
        };
        let v = serde_json::to_value(&illegal).expect("serialize");
        let obj = v.as_object().expect("object");
        assert!(!obj.contains_key("gateway_ip"), "gateway_ip=None must be skipped");
        assert!(
            obj.contains_key("gateway_mac"),
            "plugin must forward gateway_mac without gateway_ip; rejection is mihomo's job",
        );
        assert_eq!(obj["gateway_mac"], "11:22:33:44:55:66");
    }
}
