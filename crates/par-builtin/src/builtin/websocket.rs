//package: basic
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use arcstr::literal;
use bytes::Bytes;
use futures::{
    SinkExt, StreamExt,
    channel::{mpsc, oneshot},
    stream::{SplitSink, SplitStream},
};
use par_runtime::{external_def, primitive::ParString, readback::Handle};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{
    WebSocketStream, connect_async,
    tungstenite::{
        Error as WebSocketError, Message,
        error::ProtocolError,
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

external_def! {
    @basic/WebSocket.{
        Connect => websocket_connect,
    }
}

const INCOMING_BUFFER_SIZE: usize = 16;

#[derive(Debug)]
enum DataMessage {
    Text(String),
    Binary(Bytes),
}

#[derive(Debug)]
enum ReaderEvent {
    Message(DataMessage),
    End,
    Error(String),
}

enum WriterCommand {
    Send(DataMessage, oneshot::Sender<Result<(), String>>),
    Close(oneshot::Sender<Result<(), String>>),
}

struct ConnectionState {
    closed: AtomicBool,
    local_close_sent: AtomicBool,
}

impl ConnectionState {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            closed: AtomicBool::new(false),
            local_close_sent: AtomicBool::new(false),
        })
    }
}

async fn websocket_connect(mut handle: Handle) {
    let mut url = handle.receive();
    url.signal(literal!("full"));
    let url = url.string().await;

    match connect_async(url.as_str()).await {
        Ok((socket, _response)) => {
            handle.signal(literal!("ok"));
            provide_connection(handle, socket).await;
        }
        Err(error) => {
            handle.signal(literal!("err"));
            handle.provide_string(ParString::from(error.to_string()));
        }
    }
}

pub(super) async fn provide_connection<S>(mut handle: Handle, socket: WebSocketStream<S>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader_events_rx, writer_commands_tx) = start_connection_pumps(socket);

    handle
        .send()
        .concurrently(|reader| provide_reader(reader, reader_events_rx));
    provide_writer(handle, writer_commands_tx).await;
}

fn start_connection_pumps<S>(
    socket: WebSocketStream<S>,
) -> (mpsc::Receiver<ReaderEvent>, mpsc::Sender<WriterCommand>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (socket_writer, socket_reader) = socket.split();
    let (reader_events_tx, reader_events_rx) = mpsc::channel(INCOMING_BUFFER_SIZE);
    let (writer_commands_tx, writer_commands_rx) = mpsc::channel(1);
    let state = ConnectionState::new();

    tokio::spawn(pump_incoming(
        socket_reader,
        reader_events_tx.clone(),
        state.clone(),
    ));
    tokio::spawn(pump_outgoing(
        socket_writer,
        writer_commands_rx,
        reader_events_tx,
        state,
    ));

    (reader_events_rx, writer_commands_tx)
}

async fn pump_incoming<S>(
    mut socket: SplitStream<WebSocketStream<S>>,
    mut events: mpsc::Sender<ReaderEvent>,
    state: Arc<ConnectionState>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut deliver_messages = true;

    while let Some(result) = socket.next().await {
        match result {
            Ok(Message::Text(text)) => {
                if deliver_messages
                    && events
                        .send(ReaderEvent::Message(DataMessage::Text(text.to_string())))
                        .await
                        .is_err()
                {
                    deliver_messages = false;
                }
            }
            Ok(Message::Binary(bytes)) => {
                if deliver_messages
                    && events
                        .send(ReaderEvent::Message(DataMessage::Binary(bytes)))
                        .await
                        .is_err()
                {
                    deliver_messages = false;
                }
            }
            Ok(Message::Close(_)) => {
                state.closed.store(true, Ordering::SeqCst);

                // Tungstenite queues the close reply while reading the close frame. Keep
                // polling this half below so that reply is flushed, but make `.end` available
                // immediately and independently of the Par Writer.
                let mut terminal_events = events.clone();
                tokio::spawn(async move {
                    let _ = terminal_events.send(ReaderEvent::End).await;
                });

                while socket.next().await.is_some() {}
                return;
            }
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {
                // Tungstenite queues automatic pong replies. Polling again drives the reply.
            }
            Err(error) => {
                state.closed.store(true, Ordering::SeqCst);
                if deliver_messages {
                    let _ = events.send(ReaderEvent::Error(error.to_string())).await;
                }
                return;
            }
        }
    }

    state.closed.store(true, Ordering::SeqCst);
    if deliver_messages {
        let _ = events.send(ReaderEvent::End).await;
    }
}

async fn pump_outgoing<S>(
    mut socket: SplitSink<WebSocketStream<S>, Message>,
    mut commands: mpsc::Receiver<WriterCommand>,
    mut reader_events: mpsc::Sender<ReaderEvent>,
    state: Arc<ConnectionState>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    while let Some(command) = commands.next().await {
        match command {
            WriterCommand::Send(message, result) => {
                let send_result = if state.closed.load(Ordering::SeqCst) {
                    Err("WebSocket connection is closed".to_string())
                } else {
                    socket
                        .send(to_tungstenite_message(message))
                        .await
                        .map_err(|error| error.to_string())
                };

                if let Err(error) = &send_result {
                    state.closed.store(true, Ordering::SeqCst);
                    let _ = reader_events.try_send(ReaderEvent::Error(error.clone()));
                }
                let failed = send_result.is_err();
                let _ = result.send(send_result);
                if failed {
                    return;
                }
            }
            WriterCommand::Close(result) => {
                state.local_close_sent.store(true, Ordering::SeqCst);
                let close_result = if state.closed.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    normalize_close_result(socket.send(normal_close_message()).await)
                };
                let _ = result.send(close_result);
                return;
            }
        }
    }

    if !state.closed.load(Ordering::SeqCst) && !state.local_close_sent.swap(true, Ordering::SeqCst)
    {
        let _ = socket.send(normal_close_message()).await;
    }
}

fn normalize_close_result(result: Result<(), WebSocketError>) -> Result<(), String> {
    match result {
        Ok(())
        | Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed)
        | Err(WebSocketError::Protocol(ProtocolError::SendAfterClosing)) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn normal_close_message() -> Message {
    Message::Close(Some(CloseFrame {
        code: CloseCode::Normal,
        reason: "".into(),
    }))
}

async fn provide_reader(mut handle: Handle, mut events: mpsc::Receiver<ReaderEvent>) {
    loop {
        match handle.case().await.as_str() {
            "close" | "#close" => {
                handle.signal(literal!("ok"));
                return handle.break_();
            }
            "read" => match events.next().await {
                Some(ReaderEvent::Message(message)) => {
                    handle.signal(literal!("ok"));
                    handle.signal(literal!("message"));
                    provide_message(handle.send(), message);
                }
                Some(ReaderEvent::End) => {
                    handle.signal(literal!("ok"));
                    handle.signal(literal!("end"));
                    return handle.break_();
                }
                Some(ReaderEvent::Error(error)) => {
                    handle.signal(literal!("err"));
                    return handle.provide_string(ParString::from(error));
                }
                None => {
                    handle.signal(literal!("err"));
                    return handle.provide_string(ParString::from(
                        "WebSocket connection stopped unexpectedly",
                    ));
                }
            },
            _ => unreachable!(),
        }
    }
}

async fn provide_writer(mut handle: Handle, mut commands: mpsc::Sender<WriterCommand>) {
    loop {
        match handle.case().await.as_str() {
            "close" | "#close" => {
                let (result_tx, result_rx) = oneshot::channel();
                let result = if commands
                    .send(WriterCommand::Close(result_tx))
                    .await
                    .is_err()
                {
                    Ok(())
                } else {
                    result_rx.await.unwrap_or(Ok(()))
                };
                provide_unit_result(handle, result);
                return;
            }
            "send" => {
                let message = read_message(handle.receive()).await;
                let (result_tx, result_rx) = oneshot::channel();
                let result = if commands
                    .send(WriterCommand::Send(message, result_tx))
                    .await
                    .is_err()
                {
                    Err("WebSocket connection is closed".to_string())
                } else {
                    result_rx.await.unwrap_or_else(|_| {
                        Err("WebSocket connection stopped unexpectedly".to_string())
                    })
                };

                match result {
                    Ok(()) => handle.signal(literal!("ok")),
                    Err(error) => {
                        handle.signal(literal!("err"));
                        return handle.provide_string(ParString::from(error));
                    }
                }
            }
            _ => unreachable!(),
        }
    }
}

async fn read_message(mut handle: Handle) -> DataMessage {
    match handle.case().await.as_str() {
        "text" => DataMessage::Text(handle.string().await.as_str().to_string()),
        "binary" => DataMessage::Binary(handle.bytes().await),
        _ => unreachable!(),
    }
}

fn provide_message(mut handle: Handle, message: DataMessage) {
    match message {
        DataMessage::Text(text) => {
            handle.signal(literal!("text"));
            handle.provide_string(ParString::from(text));
        }
        DataMessage::Binary(bytes) => {
            handle.signal(literal!("binary"));
            handle.provide_bytes(bytes);
        }
    }
}

fn to_tungstenite_message(message: DataMessage) -> Message {
    match message {
        DataMessage::Text(text) => Message::Text(text.into()),
        DataMessage::Binary(bytes) => Message::Binary(bytes),
    }
}

fn provide_unit_result(mut handle: Handle, result: Result<(), String>) {
    match result {
        Ok(()) => {
            handle.signal(literal!("ok"));
            handle.break_();
        }
        Err(error) => {
            handle.signal(literal!("err"));
            handle.provide_string(ParString::from(error));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::{SinkExt, StreamExt, channel::oneshot};
    use tokio::{io::DuplexStream, time::timeout};
    use tokio_tungstenite::{
        WebSocketStream,
        tungstenite::protocol::{CloseFrame, Role, frame::coding::CloseCode},
    };

    use super::{DataMessage, ReaderEvent, WriterCommand, start_connection_pumps};

    async fn websocket_pair() -> (WebSocketStream<DuplexStream>, WebSocketStream<DuplexStream>) {
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        tokio::join!(
            WebSocketStream::from_raw_socket(server_io, Role::Server, None),
            WebSocketStream::from_raw_socket(client_io, Role::Client, None),
        )
    }

    #[tokio::test]
    async fn peer_close_ends_reader_while_writer_remains_open() {
        let (server, mut client) = websocket_pair().await;
        let (mut events, _commands) = start_connection_pumps(server);

        client
            .send(tokio_tungstenite::tungstenite::Message::Close(None))
            .await
            .unwrap();

        let event = timeout(Duration::from_secs(1), events.next())
            .await
            .expect("reader did not finish after peer close")
            .expect("reader event channel ended");
        assert!(matches!(event, ReaderEvent::End));
    }

    #[tokio::test]
    async fn ping_is_answered_without_a_par_read() {
        let (server, mut client) = websocket_pair().await;
        let (_events, _commands) = start_connection_pumps(server);
        let payload = bytes::Bytes::from_static(b"heartbeat");

        client
            .send(tokio_tungstenite::tungstenite::Message::Ping(
                payload.clone(),
            ))
            .await
            .unwrap();

        let response = timeout(Duration::from_secs(1), client.next())
            .await
            .expect("pong was not sent")
            .expect("client connection ended")
            .expect("client read failed");
        assert_eq!(
            response,
            tokio_tungstenite::tungstenite::Message::Pong(payload)
        );
    }

    #[tokio::test]
    async fn text_and_binary_messages_keep_their_kind_and_order() {
        let (server, mut client) = websocket_pair().await;
        let (mut events, _commands) = start_connection_pumps(server);

        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                "hello".into(),
            ))
            .await
            .unwrap();
        client
            .send(tokio_tungstenite::tungstenite::Message::Binary(
                bytes::Bytes::from_static(b"world"),
            ))
            .await
            .unwrap();

        let first = timeout(Duration::from_secs(1), events.next())
            .await
            .unwrap()
            .unwrap();
        let second = timeout(Duration::from_secs(1), events.next())
            .await
            .unwrap()
            .unwrap();

        assert!(matches!(
            first,
            ReaderEvent::Message(DataMessage::Text(text)) if text == "hello"
        ));
        assert!(matches!(
            second,
            ReaderEvent::Message(DataMessage::Binary(bytes)) if bytes.as_ref() == b"world"
        ));
    }

    #[tokio::test]
    async fn writer_close_flushes_without_waiting_for_reader() {
        let (server, mut client) = websocket_pair().await;
        let (mut events, mut commands) = start_connection_pumps(server);
        let (result_tx, result_rx) = oneshot::channel();

        commands
            .send(WriterCommand::Close(result_tx))
            .await
            .unwrap();
        timeout(Duration::from_secs(1), result_rx)
            .await
            .expect("writer close waited for the peer")
            .expect("writer pump dropped the result")
            .expect("writer close failed");

        let close = timeout(Duration::from_secs(1), client.next())
            .await
            .expect("client did not receive close")
            .expect("client connection ended before close")
            .expect("client read failed");
        assert_eq!(
            close,
            tokio_tungstenite::tungstenite::Message::Close(Some(CloseFrame {
                code: CloseCode::Normal,
                reason: "".into(),
            }))
        );

        let client_drain = tokio::spawn(async move { while client.next().await.is_some() {} });
        let event = timeout(Duration::from_secs(1), events.next())
            .await
            .expect("reader did not observe the peer close reply")
            .expect("reader event channel ended");
        assert!(matches!(event, ReaderEvent::End));
        client_drain.await.unwrap();
    }

    #[tokio::test]
    async fn send_after_peer_close_fails_without_blocking() {
        let (server, mut client) = websocket_pair().await;
        let (mut events, mut commands) = start_connection_pumps(server);

        client
            .send(tokio_tungstenite::tungstenite::Message::Close(None))
            .await
            .unwrap();
        let event = timeout(Duration::from_secs(1), events.next())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, ReaderEvent::End));

        let (result_tx, result_rx) = oneshot::channel();
        commands
            .send(WriterCommand::Send(
                DataMessage::Text("too late".to_string()),
                result_tx,
            ))
            .await
            .unwrap();
        let result = timeout(Duration::from_secs(1), result_rx)
            .await
            .expect("send blocked after peer close")
            .expect("writer pump dropped the result");
        assert!(result.is_err());
    }
}
