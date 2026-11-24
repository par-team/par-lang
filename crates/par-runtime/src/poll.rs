use crate::registry::PackageRef::Special;
use crate::registry::{DefinitionRef, ExternalDef};
use arcstr::ArcStr;
use std::pin::Pin;

pub const POLL_TOKEN: DefinitionRef = DefinitionRef {
    package: Special("#internal"),
    path: &[],
    module: "",
    name: "#poll_token",
};

fn poll_token(
    handle: crate::readback::Handle,
) -> Pin<Box<dyn Send + core::future::Future<Output = ()>>> {
    Box::pin(poll_token_server(handle))
}

async fn poll_token_server(mut handle: crate::readback::Handle) {
    use futures::future::BoxFuture;
    use futures::stream::FuturesUnordered;
    use futures::stream::StreamExt as _;

    let mut clients: FuturesUnordered<BoxFuture<'static, crate::flat::readback::Handle>> =
        FuturesUnordered::new();

    loop {
        let op = handle.case().await;
        match op.as_str() {
            "#poll" => {
                // payload: (result_slot) next_slot
                // implemented as a Pair(left = next_slot, right = result_slot)
                let mut result_slot = handle.receive();

                if clients.is_empty() {
                    result_slot.signal(ArcStr::from("#empty"));
                    result_slot.break_();
                    continue;
                }

                let client = clients
                    .next()
                    .await
                    .expect("poll clients stream unexpectedly empty");

                result_slot.signal(ArcStr::from("#client"));
                result_slot.link(crate::readback::Handle::from(client));
            }

            "#submit" => {
                // payload: (stream) next_slot
                // implemented as a Pair(left = next_slot, right = stream)
                let mut stream = handle.receive();

                loop {
                    let tag = stream.case().await;
                    match tag.as_str() {
                        "#end" => {
                            stream.continue_();
                            break;
                        }
                        "#item" => {
                            // payload: (client) tail
                            // implemented as a Pair(left = tail, right = client)
                            let client = stream.receive();
                            let client = client.handle;
                            clients.push(Box::pin(async move { client.await_ready().await }));
                        }
                        other => panic!("Invalid submit stream item: {other}"),
                    }
                }
            }

            "#close" => {
                // payload: !
                handle.continue_();
                break;
            }

            other => panic!("Invalid poll token operation: {other}"),
        }
    }
}

inventory::submit!(ExternalDef {
    path: POLL_TOKEN,
    f: poll_token,
});
