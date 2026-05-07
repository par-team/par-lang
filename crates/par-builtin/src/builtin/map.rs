//package: core
use std::collections::BTreeMap;

use crate::builtin::list::readback_list;
use par_runtime::atom::sym;
use par_runtime::readback::{Data, Handle};
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};

macro_rules! core_map_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "Map",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_map_external!("New", map_new);
core_map_external!("FromList", map_from_list);

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
        match handle.case().await {
            sym::size => {
                handle.send().provide_nat(map.len().into());
                continue;
            }
            sym::keys => {
                let mut keys = handle.send();
                for key in map.keys() {
                    keys.signal(sym::item);
                    keys.send_data(key);
                }
                keys.signal(sym::end);
                keys.break_();
                continue;
            }
            sym::list => {
                for (key, value) in map.into_iter() {
                    handle.signal(sym::item);
                    let mut pair = handle.send();
                    pair.send_data(&key);
                    pair.link(value);
                }
                handle.signal(sym::end);
                return handle.break_();
            }
            sym::entry => {
                let key = handle.receive_data().await;
                let removed = map.remove(&key);
                handle.send().concurrently(|mut handle| async move {
                    match removed {
                        Some(value) => {
                            handle.signal(sym::some);
                            handle.link(value);
                        }
                        None => {
                            handle.signal(sym::none);
                            handle.break_();
                        }
                    }
                });
                match handle.case().await {
                    sym::put => {
                        let new_value = handle.receive();
                        map.insert(key, new_value);
                    }
                    sym::delete => {}
                    _ => unreachable!(),
                }
                continue;
            }
            _ => unreachable!(),
        }
    }
}
