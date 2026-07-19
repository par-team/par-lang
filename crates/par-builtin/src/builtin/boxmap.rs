//package: core
use std::sync::Arc;

use par_runtime::external_def;
use tokio::sync::Mutex;

use crate::builtin::list::readback_list;
use arcstr::literal;
use im::OrdMap;
use par_runtime::readback::{Data, Handle};

external_def! {
    @core/BoxMap.{
        New => boxmap_new,
        FromList => boxmap_from_list,
    }
}

async fn boxmap_new(handle: Handle) {
    provide_boxmap(handle, OrdMap::new());
}

async fn boxmap_from_list(mut handle: Handle) {
    let entries = readback_list(handle.receive(), |mut handle| async move {
        let key = handle.receive_data().await;
        let value = Arc::new(Mutex::new(handle));
        (key, value)
    })
    .await;

    let mut map: OrdMap<Data, Arc<Mutex<Handle>>> = OrdMap::new();
    for (k, v) in entries {
        if let Some(old) = map.insert(k, v) {
            erase_stored_handle(old);
        }
    }

    provide_boxmap(handle, map);
}

fn erase_stored_handle(handle: Arc<Mutex<Handle>>) {
    if let Ok(handle) = Arc::try_unwrap(handle) {
        handle.into_inner().erase();
    }
}

fn provide_boxmap(handle: Handle, map: OrdMap<Data, Arc<Mutex<Handle>>>) {
    handle.provide_box(move |mut handle| {
        let mut map = map.clone();
        async move {
            match handle.case().await.as_str() {
                "size" => {
                    return handle.provide_nat(map.len().into());
                }
                "keys" => {
                    for key in map.keys() {
                        handle.signal(literal!("item"));
                        handle.send_data(key);
                    }
                    handle.signal(literal!("end"));
                    return handle.break_();
                }
                "list" => {
                    for (key, value) in map.iter() {
                        handle.signal(literal!("item"));
                        let mut pair = handle.send();
                        pair.send_data(key);
                        pair.link(value.lock().await.duplicate());
                    }
                    handle.signal(literal!("end"));
                    return handle.break_();
                }
                "get" => {
                    let key = handle.receive_data().await;
                    match map.get(&key) {
                        Some(value) => {
                            handle.signal(literal!("some"));
                            return handle.link(value.lock().await.duplicate());
                        }
                        None => {
                            handle.signal(literal!("none"));
                            return handle.break_();
                        }
                    }
                }
                "put" => {
                    let key = handle.receive_data().await;
                    let value = handle.receive();
                    map.insert(key, Arc::new(Mutex::new(value)));
                    return provide_boxmap(handle, map);
                }
                "delete" => {
                    let key = handle.receive_data().await;
                    map.remove(&key);
                    return provide_boxmap(handle, map);
                }
                _ => unreachable!(),
            }
        }
    })
}
