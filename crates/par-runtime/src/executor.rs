use crate::flat::arena::Arena;
use crate::flat::reducer::Reducer;
use crate::flat::runtime::{PackagePtr, Runtime};
use crate::flat::stats::Rewrites;
use crate::linker::Linked;
use crate::readback::Handle;
use futures::future::RemoteHandle;
use futures::task::{Spawn, SpawnExt};
use std::sync::Arc;

pub fn start_and_instantiate(
    spawner: Arc<dyn Spawn + Send + Sync + 'static>,
    arena: Arc<Arena<Linked>>,
    package: PackagePtr<Linked>,
) -> (Handle, RemoteHandle<Rewrites>) {
    start_and_instantiate_inner(spawner, arena, package, false)
}

pub fn start_and_instantiate_with_stats(
    spawner: Arc<dyn Spawn + Send + Sync + 'static>,
    arena: Arc<Arena<Linked>>,
    package: PackagePtr<Linked>,
) -> (Handle, RemoteHandle<Rewrites>) {
    start_and_instantiate_inner(spawner, arena, package, true)
}

fn start_and_instantiate_inner(
    spawner: Arc<dyn Spawn + Send + Sync + 'static>,
    arena: Arc<Arena<Linked>>,
    package: PackagePtr<Linked>,
    measure_net_duration: bool,
) -> (Handle, RemoteHandle<Rewrites>) {
    let (reducer, net_handle) = Reducer::from(
        Runtime::from(arena.clone()),
        spawner.clone(),
        measure_net_duration,
    );
    let reducer_future = reducer.spawn_reducer();
    let handle =
        crate::flat::readback::Handle::from_package(arena.clone(), net_handle, package).unwrap();
    (
        Handle::from(handle),
        spawner
            .spawn_with_handle(async move {
                let reducer = reducer_future.await;
                reducer.runtime.rewrites
            })
            .unwrap(),
    )
}
