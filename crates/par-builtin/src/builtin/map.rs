//package: core
use std::collections::BTreeMap;

use crate::builtin::list::readback_list;
use arcstr::literal;
use par_runtime::external_def;
use par_runtime::readback::{Data, Handle};

external_def! {
    @core/Map.{
        New => map_new,
        FromList => map_from_list,
    }
}

async fn map_new(handle: Handle) {
    provide_map(handle, BTreeMap::new()).await;
}

async fn map_from_list(mut handle: Handle) {
    let entries = readback_list(handle.receive(), |mut handle| async {
        let key = handle.receive_data().await;
        let value = handle;
        (key, value)
    })
    .await;

    let mut map: BTreeMap<Data, Handle> = BTreeMap::new();
    for (key, value) in entries {
        if let Some(old) = map.insert(key, value) {
            old.erase();
        }
    }
    provide_map(handle, map).await;
}

async fn provide_map(mut handle: Handle, mut map: BTreeMap<Data, Handle>) {
    loop {
        match handle.case().await.as_str() {
            "size" => {
                handle.send().provide_nat(map.len().into());
                continue;
            }
            "keys" => {
                let mut keys = handle.send();
                for key in map.keys() {
                    keys.signal(literal!("item"));
                    keys.send_data(key);
                }
                keys.signal(literal!("end"));
                keys.break_();
                continue;
            }
            "list" | "#close" => {
                for (key, value) in map.into_iter() {
                    handle.signal(literal!("item"));
                    let mut pair = handle.send();
                    pair.send_data(&key);
                    pair.link(value);
                }
                handle.signal(literal!("end"));
                return handle.break_();
            }
            "entry" => {
                let key = handle.receive_data().await;
                let removed = map.remove(&key);
                handle.send().concurrently(|mut handle| async move {
                    match removed {
                        Some(value) => {
                            handle.signal(literal!("some"));
                            handle.link(value);
                        }
                        None => {
                            handle.signal(literal!("none"));
                            handle.break_();
                        }
                    }
                });
                match handle.case().await.as_str() {
                    "put" => {
                        let new_value = handle.receive();
                        map.insert(key, new_value);
                    }
                    "delete" => {}
                    _ => unreachable!(),
                }
                continue;
            }
            _ => unreachable!(),
        }
    }
}
