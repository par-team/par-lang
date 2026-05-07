//package: core
use std::sync::Arc;

use par_runtime::atom::sym;
use tokio::sync::Mutex;

use crate::builtin::list::readback_list;
use im::OrdMap;
use par_runtime::readback::{Data, Handle};
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};

macro_rules! core_boxmap_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "BoxMap",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_boxmap_external!("New", boxmap_new);
core_boxmap_external!("FromList", boxmap_from_list);

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
            match handle.case().await {
                sym::size => {
                    return handle.provide_nat(map.len().into());
                }
                sym::keys => {
                    for key in map.keys() {
                        handle.signal(sym::item);
                        handle.send_data(key);
                    }
                    handle.signal(sym::end);
                    return handle.break_();
                }
                sym::list => {
                    for (key, value) in map.iter() {
                        handle.signal(sym::item);
                        let mut pair = handle.send();
                        pair.send_data(key);
                        pair.link(value.lock().await.duplicate());
                    }
                    handle.signal(sym::end);
                    return handle.break_();
                }
                sym::get => {
                    let key = handle.receive_data().await;
                    match map.get(&key) {
                        Some(value) => {
                            handle.signal(sym::some);
                            return handle.link(value.lock().await.duplicate());
                        }
                        None => {
                            handle.signal(sym::none);
                            return handle.break_();
                        }
                    }
                }
                sym::put => {
                    let key = handle.receive_data().await;
                    let value = handle.receive();
                    map.insert(key, Arc::new(Mutex::new(value)));
                    return provide_boxmap(handle, map);
                }
                sym::delete => {
                    let key = handle.receive_data().await;
                    map.remove(&key);
                    return provide_boxmap(handle, map);
                }
                _ => unreachable!(),
            }
        }
    })
}
