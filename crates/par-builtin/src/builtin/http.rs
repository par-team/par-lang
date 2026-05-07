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
};
use hyper_util::rt::TokioIo;
use num_bigint::BigUint;
use tokio::{net::TcpListener, signal, sync::Notify};
use url::Url as ParsedUrl;

use crate::builtin::{list::readback_list, url::provide_url_value};
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};
use par_runtime::{atom::sym, primitive::ParString};

macro_rules! basic_http_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::BASIC,
                path: &[],
                module: "Http",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

basic_http_external!("Fetch", http_fetch);
basic_http_external!("Listen", http_listen);

// ----------

async fn http_fetch(mut handle: Handle) {
    let mut request = handle.receive();

    let method = request.receive().string().await;

    let mut url_handle = request.receive();
    url_handle.signal(sym::full);
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
            handle.signal(sym::err);
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
                handle.signal(sym::err);
                return handle.provide_string(ParString::from(body_err));
            }
            response
        }
        Err(err) => {
            handle.signal(sym::err);
            if let Err(body_err) = body_result {
                return handle.provide_string(ParString::from(body_err));
            }
            return handle.provide_string(ParString::from(err.to_string()));
        }
    };

    handle.signal(sym::ok);
    handle
        .send()
        .provide_nat(BigUint::from(response.status().as_u16()));
    provide_headers_list(handle.send(), response.headers());
    provide_body_reader(handle, response).await;
}

async fn provide_body_reader(mut handle: Handle, response: reqwest::Response) {
    let mut stream = response.bytes_stream();
    loop {
        match handle.case().await {
            sym::close => {
                handle.signal(sym::ok);
                return handle.break_();
            }
            sym::read => match stream.next().await {
                Some(Ok(bytes)) => {
                    handle.signal(sym::ok);
                    handle.signal(sym::chunk);
                    handle.send().provide_bytes(bytes);
                    continue;
                }
                Some(Err(err)) => {
                    handle.signal(sym::err);
                    return handle.provide_string(ParString::from(err.to_string()));
                }
                None => {
                    handle.signal(sym::ok);
                    handle.signal(sym::end);
                    return handle.break_();
                }
            },
            _ => unreachable!(),
        }
    }
}

fn provide_headers_list(mut handle: Handle, headers: &reqwest::header::HeaderMap) {
    for (name, value) in headers {
        handle.signal(sym::item);
        let (name, value) = (
            ParString::copy_from_slice(name.as_str()),
            Bytes::copy_from_slice(value.as_bytes()),
        );
        handle.send().concurrently(|mut handle| async {
            handle.send().provide_string(name);
            handle.provide_bytes(value);
        });
    }
    handle.signal(sym::end);
    handle.break_();
}

async fn consume_http_reader(
    mut handle: Handle,
    mut tx: mpsc::UnboundedSender<Result<bytes::Bytes, std::io::Error>>,
    done: oneshot::Sender<Result<(), ParString>>,
) {
    let mut done = Some(done);

    loop {
        handle.signal(sym::read);
        match handle.case().await {
            sym::ok => match handle.case().await {
                sym::chunk => {
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
                sym::end => {
                    handle.continue_();
                    tx.disconnect();
                    if let Some(done) = done.take() {
                        let _ = done.send(Ok(()));
                    }
                    return;
                }
                _ => unreachable!(),
            },
            sym::err => {
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
    handle.signal(sym::close);
    match handle.case().await {
        sym::ok => {
            handle.continue_();
            Ok(())
        }
        sym::err => Err(handle.string().await),
        _ => unreachable!(),
    }
}

// ----------

async fn http_listen(mut handle: Handle) {
    let address = handle.receive().string().await;
    match start_listener(address.as_str().to_string()).await {
        Ok(state) => provide_listener_value(handle, state).await,
        Err(err) => {
            handle.signal(sym::shutdown);
            handle.signal(sym::err);
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
    Incoming(IncomingRequest),
    Shutdown(Result<(), String>),
}

struct IncomingRequest {
    method: String,
    url: ParsedUrl,
    headers: Vec<(String, String)>,
    body: Incoming,
    responder: oneshot::Sender<Result<Response<ResponseBody>, BodyError>>,
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
    req: Request<Incoming>,
    control: ListenerControl,
) -> Result<Response<ResponseBody>, hyper::Error> {
    if control.is_shutdown() {
        return Ok(simple_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "server shutting down",
        ));
    }

    let (parts, body) = req.into_parts();
    let method = parts.method.as_str().to_string();

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
        responder: resp_tx,
    };

    if control
        .sender
        .clone()
        .send(ListenerEvent::Incoming(incoming))
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
            handle.signal(sym::incoming);
            let IncomingRequest {
                method,
                url,
                headers,
                body,
                responder,
            } = request;

            handle.send().concurrently(|handle| {
                provide_http_request_value(handle, method, url, headers, body)
            });
            handle
                .send()
                .concurrently(|handle| provide_responder_function(handle, responder));
            Box::pin(provide_listener_value(handle, state)).await;
        }

        ListenerEvent::Shutdown(result) => {
            handle.signal(sym::shutdown);
            match result {
                Ok(()) => {
                    handle.signal(sym::ok);
                    handle.break_();
                }
                Err(err) => {
                    handle.signal(sym::err);
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
        handle.signal(sym::item);
        let mut pair = handle.send();
        pair.send().provide_string(ParString::from(name));
        pair.provide_bytes(Bytes::from(value));
    }
    handle.signal(sym::end);
    handle.break_();
}

async fn provide_request_body_reader(mut handle: Handle, mut body: Incoming) {
    loop {
        match handle.case().await {
            sym::close => {
                handle.signal(sym::ok);
                return handle.break_();
            }
            sym::read => match body.frame().await {
                Some(Ok(frame)) => {
                    match frame.into_data() {
                        Ok(chunk) => {
                            if chunk.is_empty() {
                                continue;
                            }
                            handle.signal(sym::ok);
                            handle.signal(sym::chunk);
                            handle.send().provide_bytes(chunk);
                        }
                        Err(_) => {
                            // Skip non-data frames such as trailers.
                            continue;
                        }
                    }
                }
                Some(Err(err)) => {
                    handle.signal(sym::err);
                    handle.provide_string(ParString::from(err.to_string()));
                    return;
                }
                None => {
                    handle.signal(sym::ok);
                    handle.signal(sym::end);
                    return handle.break_();
                }
            },
            _ => unreachable!(),
        }
    }
}

async fn provide_responder_function(
    mut handle: Handle,
    responder: oneshot::Sender<Result<Response<ResponseBody>, BodyError>>,
) {
    match build_response(handle.receive()).await {
        Ok(response) => {
            let _ = responder.send(Ok(response));
            handle.signal(sym::ok);
            handle.break_();
        }
        Err(err) => {
            let _ = responder.send(Err(BodyError(err.clone())));
            handle.signal(sym::err);
            handle.provide_string(err);
        }
    }
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
            reader.signal(sym::read);
            match reader.case().await {
                sym::ok => match reader.case().await {
                    sym::chunk => {
                        let bytes = reader.receive().bytes().await;
                        if tx.send(Ok(Frame::data(bytes))).await.is_err() {
                            reader.signal(sym::close);
                            match reader.case().await {
                                sym::ok => reader.continue_(),
                                sym::err => {
                                    let _ = reader.string();
                                }
                                _ => unreachable!(),
                            }
                            return;
                        }
                        continue;
                    }
                    sym::end => {
                        reader.continue_();
                        return;
                    }
                    _ => unreachable!(),
                },
                sym::err => {
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
