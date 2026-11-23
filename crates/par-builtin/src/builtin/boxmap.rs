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
        let value = Stored::new(handle);
        (key, value)
    })
    .await;

    // Earlier values of repeated keys are closed as they're replaced.
    let map: OrdMap<Data, Arc<Stored>> = entries.into_iter().collect();
    provide_boxmap(handle, map);
}

/// A value stored in a map. All versions of a map share their stored values, so a value
/// is closed once the last version holding it is gone.
struct Stored(Mutex<Option<Handle>>);

impl Stored {
    fn new(handle: Handle) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Some(handle))))
    }

    async fn duplicate(&self) -> Handle {
        self.0
            .lock()
            .await
            .as_mut()
            .expect("stored value is only taken on drop")
            .duplicate()
    }
}

impl Drop for Stored {
    fn drop(&mut self) {
        if let Some(handle) = self.0.get_mut().take() {
            handle.close();
        }
    }
}

fn provide_boxmap(handle: Handle, map: OrdMap<Data, Arc<Stored>>) {
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
                        pair.link(value.duplicate().await);
                    }
                    handle.signal(literal!("end"));
                    return handle.break_();
                }
                "get" => {
                    let key = handle.receive_data().await;
                    match map.get(&key) {
                        Some(value) => {
                            handle.signal(literal!("some"));
                            return handle.link(value.duplicate().await);
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
                    map.insert(key, Stored::new(value));
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
