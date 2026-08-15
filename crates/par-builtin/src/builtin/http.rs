//package: basic
use std::{
    io,
    net::SocketAddr,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    time::Duration,
};

use arcstr::literal;
use bytes::Bytes;
use futures::{
    SinkExt, StreamExt,
    channel::{mpsc, oneshot},
};
use http_body::Frame;
use http_body_util::{self as body_util, BodyExt, Full, StreamBody};
use hyper::{
    Request, Response,
    body::Incoming,
    http::{HeaderName, HeaderValue, StatusCode, header::HOST},
    service::service_fn,
    upgrade::{OnUpgrade, Upgraded},
};
use hyper_util::rt::TokioIo;
use num_bigint::BigUint;
use tokio::{net::TcpListener, signal, sync::Notify};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Error as WebSocketError, error::ProtocolError, handshake::server::create_response,
        protocol::Role,
    },
};
use url::Url as ParsedUrl;

use crate::builtin::{list::readback_list, url::provide_url_value, websocket::provide_connection};
use par_runtime::readback::Handle;
use par_runtime::{external_def, primitive::ParString};

external_def! {
    @basic/Http.{
        Fetch => http_fetch,
        Listen => http_listen,
    }
}

// ----------

async fn http_fetch(mut handle: Handle) {
    let mut request = handle.receive();

    let method = request.receive().string().await;

    let mut url_handle = request.receive();
    url_handle.signal(literal!("full"));
    let url = url_handle.string().await;

    let header_pairs = readback_list(request.receive(), |mut handle| async move {
        let name = handle.receive().string().await;
        let value = handle.string().await;
        (name, value)
    })
    .await;

    let body_reader = request;

    let (tx, rx) = mpsc::unbounded::<Result<bytes::Bytes, std::io::Error>>();
    let (body_done_tx, body_done_rx) = oneshot::channel::<Result<(), ParString>>();

    body_reader.concurrently(move |handle| async move {
        consume_http_reader(handle, tx, body_done_tx).await;
    });

    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            handle.signal(literal!("err"));
            return handle.provide_string(ParString::from(err.to_string()));
        }
    };

    let method = reqwest::Method::from_bytes(&method.as_bytes()).unwrap_or(reqwest::Method::GET);

    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in header_pairs.iter() {
        if let (Ok(hn), Ok(hv)) = (
            reqwest::header::HeaderName::from_bytes(&name.as_bytes()),
            reqwest::header::HeaderValue::from_bytes(&value.as_bytes()),
        ) {
            headers.append(hn, hv);
        }
    }

    let request = client.request(method, url.as_str());
    let request = request
        .headers(headers)
        .body(reqwest::Body::wrap_stream(rx));

    let response_result = request.send().await;
    let body_result = body_done_rx.await.unwrap_or(Ok(()));

    let response = match response_result {
        Ok(response) => {
            if let Err(body_err) = body_result {
                handle.signal(literal!("err"));
                return handle.provide_string(ParString::from(body_err));
            }
            response
        }
        Err(err) => {
            handle.signal(literal!("err"));
            if let Err(body_err) = body_result {
                return handle.provide_string(ParString::from(body_err));
            }
            return handle.provide_string(ParString::from(err.to_string()));
        }
    };

    handle.signal(literal!("ok"));
    handle
        .send()
        .provide_nat(BigUint::from(response.status().as_u16()));
    provide_headers_list(handle.send(), response.headers());
    provide_body_reader(handle, response).await;
}

async fn provide_body_reader(mut handle: Handle, response: reqwest::Response) {
    let mut stream = response.bytes_stream();
    loop {
        match handle.case().await.as_str() {
            "close" | "#close" => {
                handle.signal(literal!("ok"));
                return handle.break_();
            }
            "read" => match stream.next().await {
                Some(Ok(bytes)) => {
                    handle.signal(literal!("ok"));
                    handle.signal(literal!("chunk"));
                    handle.send().provide_bytes(bytes);
                    continue;
                }
                Some(Err(err)) => {
                    handle.signal(literal!("err"));
                    return handle.provide_string(ParString::from(err.to_string()));
                }
                None => {
                    handle.signal(literal!("ok"));
                    handle.signal(literal!("end"));
                    return handle.break_();
                }
            },
            _ => unreachable!(),
        }
    }
}

fn provide_headers_list(mut handle: Handle, headers: &reqwest::header::HeaderMap) {
    for (name, value) in headers {
        handle.signal(literal!("item"));
        let (name, value) = (
            ParString::copy_from_slice(name.as_str()),
            Bytes::copy_from_slice(value.as_bytes()),
        );
        handle.send().concurrently(|mut handle| async {
            handle.send().provide_string(name);
            handle.provide_bytes(value);
        });
    }
    handle.signal(literal!("end"));
    handle.break_();
}

async fn consume_http_reader(
    mut handle: Handle,
    mut tx: mpsc::UnboundedSender<Result<bytes::Bytes, std::io::Error>>,
    done: oneshot::Sender<Result<(), ParString>>,
) {
    let mut done = Some(done);

    loop {
        handle.signal(literal!("read"));
        match handle.case().await.as_str() {
            "ok" => match handle.case().await.as_str() {
                "chunk" => {
                    let chunk = handle.receive().bytes().await;
                    if chunk.is_empty() {
                        continue;
                    }
                    if tx.unbounded_send(Ok(chunk)).is_err() {
                        let result = close_reader(handle).await;
                        if let Some(done) = done.take() {
                            let _ = done.send(result);
                        }
                        return;
                    }
                }
                "end" => {
                    handle.continue_();
                    tx.disconnect();
                    if let Some(done) = done.take() {
                        let _ = done.send(Ok(()));
                    }
                    return;
                }
                _ => unreachable!(),
            },
            "err" => {
                let err = handle.string().await;
                let io_err = io::Error::new(io::ErrorKind::Other, err.as_str().to_string());
                let _ = tx.unbounded_send(Err(io_err));
                if let Some(done) = done.take() {
                    let _ = done.send(Err(err));
                }
                return;
            }
            _ => unreachable!(),
        }
    }
}

async fn close_reader(mut handle: Handle) -> Result<(), ParString> {
    handle.signal(literal!("close"));
    match handle.case().await.as_str() {
        "ok" => {
            handle.continue_();
            Ok(())
        }
        "err" => Err(handle.string().await),
        _ => unreachable!(),
    }
}

// ----------

async fn http_listen(mut handle: Handle) {
    let address = handle.receive().string().await;
    match start_listener(address.as_str().to_string()).await {
        Ok(state) => provide_listener_value(handle, state).await,
        Err(err) => {
            handle.signal(literal!("shutdown"));
            handle.signal(literal!("err"));
            handle.provide_string(err);
        }
    }
}

type ResponseBody = body_util::combinators::BoxBody<Bytes, BodyError>;

struct ListenerState {
    events: mpsc::UnboundedReceiver<ListenerEvent>,
}

#[derive(Clone)]
struct ListenerControl {
    sender: mpsc::UnboundedSender<ListenerEvent>,
    notify: Arc<Notify>,
    shutdown: Arc<AtomicBool>,
    bind_host: String,
}

enum ListenerEvent {
    Incoming(Box<IncomingRequest>),
    Shutdown(Result<(), String>),
}

struct IncomingRequest {
    method: String,
    url: ParsedUrl,
    headers: Vec<(String, String)>,
    body: Incoming,
    respond: IncomingRespond,
}

struct IncomingRespond {
    response: oneshot::Sender<Result<Response<ResponseBody>, BodyError>>,
    upgrade_request: Request<()>,
    on_upgrade: OnUpgrade,
}

#[derive(Debug, Clone)]
struct BodyError(ParString);

impl std::fmt::Display for BodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.as_str())
    }
}

impl std::error::Error for BodyError {}

impl ListenerControl {
    fn new(sender: mpsc::UnboundedSender<ListenerEvent>, bind_host: String) -> Self {
        Self {
            sender,
            notify: Arc::new(Notify::new()),
            shutdown: Arc::new(AtomicBool::new(false)),
            bind_host,
        }
    }

    async fn trigger_shutdown(&mut self, result: Result<(), String>) {
        if !self.shutdown.swap(true, AtomicOrdering::SeqCst) {
            let _ = self.sender.send(ListenerEvent::Shutdown(result)).await;
            self.notify.notify_waiters();
        }
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.load(AtomicOrdering::SeqCst)
    }
}

impl ListenerState {
    fn new(receiver: mpsc::UnboundedReceiver<ListenerEvent>) -> Self {
        Self { events: receiver }
    }

    async fn next_event(&mut self) -> ListenerEvent {
        match self.events.next().await {
            Some(event) => event,
            None => ListenerEvent::Shutdown(Ok(())),
        }
    }
}

async fn start_listener(address: String) -> Result<ListenerState, ParString> {
    let socket_addr: SocketAddr = address
        .parse()
        .map_err(|err: std::net::AddrParseError| err.to_string())?;

    let listener = TcpListener::bind(socket_addr)
        .await
        .map_err(|err| err.to_string())?;

    let (event_tx, event_rx) = mpsc::unbounded();
    let mut control = ListenerControl::new(event_tx, address);

    let mut accept_control = control.clone();
    tokio::spawn(async move {
        if let Err(err) = run_accept_loop(listener, accept_control.clone()).await {
            accept_control.trigger_shutdown(Err(err)).await;
        }
    });

    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => control.trigger_shutdown(Ok(())).await,
            Err(err) => control.trigger_shutdown(Err(err.to_string())).await,
        }
    });

    Ok(ListenerState::new(event_rx))
}

async fn run_accept_loop(
    listener: TcpListener,
    mut control: ListenerControl,
) -> Result<(), String> {
    loop {
        tokio::select! {
            _ = control.notify.notified() => {
                break Ok(());
            }
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((stream, _)) => {
                        let control = control.clone();
                        tokio::spawn(async move {
                            handle_connection(stream, control).await;
                        });
                    }
                    Err(err) => {
                        let message = err.to_string();
                        control.trigger_shutdown(Err(message.clone())).await;
                        break Err(message);
                    }
                }
            }
        }
    }
}

async fn handle_connection(stream: tokio::net::TcpStream, control: ListenerControl) {
    let io = TokioIo::new(stream);
    let service = service_fn(move |req| handle_request(req, control.clone()));

    if hyper::server::conn::http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades()
        .await
        .is_err()
    {
        // Ignore individual connection errors; they are reported per-request.
    }
}

async fn handle_request(
    mut req: Request<Incoming>,
    control: ListenerControl,
) -> Result<Response<ResponseBody>, hyper::Error> {
    if control.is_shutdown() {
        return Ok(simple_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "server shutting down",
        ));
    }

    let on_upgrade = hyper::upgrade::on(&mut req);
    let (parts, body) = req.into_parts();
    let method = parts.method.as_str().to_string();

    let mut upgrade_request = Request::new(());
    *upgrade_request.method_mut() = parts.method.clone();
    *upgrade_request.uri_mut() = parts.uri.clone();
    *upgrade_request.version_mut() = parts.version;
    *upgrade_request.headers_mut() = parts.headers.clone();

    let headers_vec = parts
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect::<Vec<_>>();

    let host = parts
        .headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| control.bind_host.clone());

    let path = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let scheme = parts.uri.scheme_str().unwrap_or("http");
    let full_url = format!("{}://{}{}", scheme, host, path);

    let parsed_url = match ParsedUrl::parse(&full_url) {
        Ok(url) => url,
        Err(_) => {
            return Ok(simple_response(
                StatusCode::BAD_REQUEST,
                "invalid request url",
            ));
        }
    };

    let (resp_tx, resp_rx) = oneshot::channel();
    let incoming = IncomingRequest {
        method,
        url: parsed_url,
        headers: headers_vec,
        body,
        respond: IncomingRespond {
            response: resp_tx,
            upgrade_request,
            on_upgrade,
        },
    };

    if control
        .sender
        .clone()
        .send(ListenerEvent::Incoming(Box::new(incoming)))
        .await
        .is_err()
    {
        return Ok(simple_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "listener dropped",
        ));
    }

    match resp_rx.await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(err)) => Ok(simple_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            err.to_string(),
        )),
        Err(_) => Ok(simple_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "response cancelled",
        )),
    }
}

async fn provide_listener_value(mut handle: Handle, mut state: ListenerState) {
    match state.next_event().await {
        ListenerEvent::Incoming(request) => {
            handle.signal(literal!("incoming"));
            let IncomingRequest {
                method,
                url,
                headers,
                body,
                respond,
            } = *request;

            handle.send().concurrently(|handle| {
                provide_http_request_value(handle, method, url, headers, body)
            });
            handle
                .send()
                .concurrently(|handle| provide_respond_value(handle, respond));
            Box::pin(provide_listener_value(handle, state)).await;
        }

        ListenerEvent::Shutdown(result) => {
            handle.signal(literal!("shutdown"));
            match result {
                Ok(()) => {
                    handle.signal(literal!("ok"));
                    handle.break_();
                }
                Err(err) => {
                    handle.signal(literal!("err"));
                    handle.provide_string(ParString::from(err));
                }
            }
        }
    }
}

async fn provide_http_request_value(
    mut handle: Handle,
    method: String,
    url: ParsedUrl,
    headers: Vec<(String, String)>,
    body: Incoming,
) {
    handle.send().provide_string(ParString::from(method));
    provide_url_value(handle.send(), url);
    provide_header_list_value(handle.send(), headers);
    provide_request_body_reader(handle, body).await;
}

fn provide_header_list_value(mut handle: Handle, headers: Vec<(String, String)>) {
    for (name, value) in headers {
        handle.signal(literal!("item"));
        let mut pair = handle.send();
        pair.send().provide_string(ParString::from(name));
        pair.provide_bytes(Bytes::from(value));
    }
    handle.signal(literal!("end"));
    handle.break_();
}

async fn provide_request_body_reader(mut handle: Handle, mut body: Incoming) {
    loop {
        match handle.case().await.as_str() {
            "close" | "#close" => {
                handle.signal(literal!("ok"));
                return handle.break_();
            }
            "read" => match body.frame().await {
                Some(Ok(frame)) => {
                    match frame.into_data() {
                        Ok(chunk) => {
                            if chunk.is_empty() {
                                continue;
                            }
                            handle.signal(literal!("ok"));
                            handle.signal(literal!("chunk"));
                            handle.send().provide_bytes(chunk);
                        }
                        Err(_) => {
                            // Skip non-data frames such as trailers.
                            continue;
                        }
                    }
                }
                Some(Err(err)) => {
                    handle.signal(literal!("err"));
                    handle.provide_string(ParString::from(err.to_string()));
                    return;
                }
                None => {
                    handle.signal(literal!("ok"));
                    handle.signal(literal!("end"));
                    return handle.break_();
                }
            },
            _ => unreachable!(),
        }
    }
}

async fn provide_respond_value(mut handle: Handle, respond: IncomingRespond) {
    match handle.case().await.as_str() {
        "http" => match build_response(handle.receive()).await {
            Ok(response) => {
                let _ = respond.response.send(Ok(response));
                handle.signal(literal!("ok"));
                handle.break_();
            }
            Err(err) => {
                let _ = respond.response.send(Err(BodyError(err.clone())));
                handle.signal(literal!("err"));
                handle.provide_string(err);
            }
        },
        "webSocket" => provide_websocket_upgrade(handle, respond).await,
        _ => unreachable!(),
    }
}

async fn provide_websocket_upgrade(mut handle: Handle, respond: IncomingRespond) {
    match accept_websocket(respond).await {
        Ok(socket) => {
            handle.signal(literal!("ok"));
            provide_connection(handle, socket).await;
        }
        Err(error) => {
            handle.signal(literal!("err"));
            handle.provide_string(ParString::from(error));
        }
    }
}

async fn accept_websocket(
    respond: IncomingRespond,
) -> Result<WebSocketStream<TokioIo<Upgraded>>, String> {
    let IncomingRespond {
        response,
        upgrade_request,
        on_upgrade,
    } = respond;

    let handshake_response = match create_response(&upgrade_request) {
        Ok(response) => response.map(|()| empty_response_body()),
        Err(error) => {
            let unsupported_version = matches!(
                &error,
                WebSocketError::Protocol(ProtocolError::MissingSecWebSocketVersionHeader)
            );
            let error = error.to_string();
            let mut rejection = websocket_rejection(&upgrade_request, &error);
            if unsupported_version {
                rejection
                    .headers_mut()
                    .insert("Sec-WebSocket-Version", HeaderValue::from_static("13"));
            }
            let _ = response.send(Ok(rejection));
            return Err(error);
        }
    };

    if response.send(Ok(handshake_response)).is_err() {
        return Err(
            "HTTP request ended before the WebSocket upgrade response was sent".to_string(),
        );
    }

    let upgraded = on_upgrade.await.map_err(|error| error.to_string())?;
    Ok(server_websocket(upgraded).await)
}

async fn server_websocket(upgraded: Upgraded) -> WebSocketStream<TokioIo<Upgraded>> {
    WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await
}

fn websocket_rejection(request: &Request<()>, error: &str) -> Response<ResponseBody> {
    let status = if has_websocket_upgrade_intent(request) {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::UPGRADE_REQUIRED
    };
    let mut response = simple_response(status, error);
    response
        .headers_mut()
        .insert("Upgrade", HeaderValue::from_static("websocket"));
    response
}

fn has_websocket_upgrade_intent(request: &Request<()>) -> bool {
    request
        .headers()
        .get("Upgrade")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        || request
            .headers()
            .get("Connection")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split([' ', ','])
                    .any(|token| token.eq_ignore_ascii_case("upgrade"))
            })
}

fn empty_response_body() -> ResponseBody {
    Full::new(Bytes::new())
        .map_err(|infallible| match infallible {})
        .boxed()
}

async fn build_response(mut handle: Handle) -> Result<Response<ResponseBody>, ParString> {
    use num_traits::ToPrimitive;

    let status = handle
        .receive()
        .nat()
        .await
        .to_u16()
        .map(StatusCode::from_u16)
        .and_then(Result::ok)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let headers = readback_list(handle.receive(), |mut handle| async {
        let key = handle.receive().string().await;
        let val = handle.bytes().await;
        (key, val)
    })
    .await;

    let mut response = Response::builder()
        .status(status)
        .body(BodyExt::boxed(reader_to_body(handle)))
        .map_err(|err| err.to_string())?;

    for (name, value) in headers {
        let Ok(header_name) = HeaderName::from_str(name.as_str()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::from_bytes(&value) else {
            continue;
        };
        response.headers_mut().append(header_name, header_value);
    }

    Ok(response)
}

fn reader_to_body(reader: Handle) -> StreamBody<mpsc::Receiver<Result<Frame<Bytes>, BodyError>>> {
    let (mut tx, rx) = mpsc::channel(1);

    reader.concurrently(|mut reader| async move {
        loop {
            reader.signal(literal!("read"));
            match reader.case().await.as_str() {
                "ok" => match reader.case().await.as_str() {
                    "chunk" => {
                        let bytes = reader.receive().bytes().await;
                        if tx.send(Ok(Frame::data(bytes))).await.is_err() {
                            reader.signal(literal!("close"));
                            match reader.case().await.as_str() {
                                "ok" => reader.continue_(),
                                "err" => {
                                    let _ = reader.string().await;
                                }
                                _ => unreachable!(),
                            }
                            return;
                        }
                        continue;
                    }
                    "end" => {
                        reader.continue_();
                        return;
                    }
                    _ => unreachable!(),
                },
                "err" => {
                    let err = reader.string().await;
                    let _ = tx.send(Err(BodyError(err))).await;
                    return;
                }
                _ => unreachable!(),
            }
        }
    });

    StreamBody::new(rx)
}

fn simple_response(status: StatusCode, message: impl Into<String>) -> Response<ResponseBody> {
    let text = message.into();
    let body = Full::new(Bytes::from(text))
        .map_err(|infallible| match infallible {})
        .boxed();
    Response::builder()
        .status(status)
        .body(body)
        .expect("valid response")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::{SinkExt, StreamExt, channel::mpsc};
    use hyper::{Request, body::Incoming, service::service_fn};
    use hyper_util::rt::TokioIo;
    use tokio::time::timeout;
    use tokio_tungstenite::tungstenite::Message;

    use super::{
        IncomingRequest, ListenerControl, ListenerEvent, accept_websocket, handle_request,
    };

    #[tokio::test]
    async fn http_upgrade_produces_a_working_websocket() {
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let (event_tx, mut event_rx) = mpsc::unbounded();
        let control = ListenerControl::new(event_tx, "localhost".to_string());

        let server = tokio::spawn(async move {
            let service = service_fn(move |request: Request<Incoming>| {
                handle_request(request, control.clone())
            });
            hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(server_io), service)
                .with_upgrades()
                .await
                .unwrap();
        });

        let client = tokio::spawn(async move {
            tokio_tungstenite::client_async("ws://localhost/socket", client_io)
                .await
                .unwrap()
                .0
        });

        let incoming = timeout(Duration::from_secs(1), event_rx.next())
            .await
            .expect("HTTP listener did not receive the upgrade request")
            .expect("HTTP listener event stream ended");
        let ListenerEvent::Incoming(incoming) = incoming else {
            panic!("listener shut down during upgrade");
        };
        let IncomingRequest { respond, .. } = *incoming;

        let mut server_socket = accept_websocket(respond).await.unwrap();
        let mut client_socket = client.await.unwrap();

        client_socket
            .send(Message::Text("hello".into()))
            .await
            .unwrap();
        let message = timeout(Duration::from_secs(1), server_socket.next())
            .await
            .expect("server did not receive WebSocket message")
            .expect("server WebSocket ended")
            .expect("server WebSocket read failed");
        assert_eq!(message, Message::Text("hello".into()));

        server_socket
            .send(Message::Binary(bytes::Bytes::from_static(b"world")))
            .await
            .unwrap();
        let message = timeout(Duration::from_secs(1), client_socket.next())
            .await
            .expect("client did not receive WebSocket message")
            .expect("client WebSocket ended")
            .expect("client WebSocket read failed");
        assert_eq!(
            message,
            Message::Binary(bytes::Bytes::from_static(b"world"))
        );

        client_socket.send(Message::Close(None)).await.unwrap();
        let _ = timeout(Duration::from_secs(1), server_socket.next()).await;
        server.await.unwrap();
    }

    #[test]
    fn non_upgrade_requests_are_rejected_with_upgrade_required() {
        let request = Request::builder().method("GET").uri("/").body(()).unwrap();
        let response = super::websocket_rejection(&request, "not an upgrade");

        assert_eq!(response.status(), hyper::StatusCode::UPGRADE_REQUIRED);
        assert_eq!(response.headers()["Upgrade"], "websocket");
    }

    #[test]
    fn upgrade_intent_with_an_invalid_handshake_is_bad_request() {
        let request = Request::builder()
            .method("GET")
            .uri("/")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .body(())
            .unwrap();
        let response = super::websocket_rejection(&request, "invalid handshake");

        assert_eq!(response.status(), hyper::StatusCode::BAD_REQUEST);
    }
}
