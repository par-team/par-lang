//package: core
use par_runtime::atom::sym;
use par_runtime::readback::{Data, Handle};
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};
use std::future::Future;

macro_rules! core_list_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "List",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_list_external!("Sort", list_sort, false);
core_list_external!("SortDesc", list_sort, true);
core_list_external!("SortBy", list_sort_by, false);
core_list_external!("SortDescBy", list_sort_by, true);
core_list_external!("SortLinearBy", list_sort_linear_by, false);
core_list_external!("SortLinearDescBy", list_sort_linear_by, true);

pub(super) async fn readback_list<T, F>(
    mut handle: Handle,
    mut readback_item: impl FnMut(Handle) -> F,
) -> Vec<T>
where
    F: Future<Output = T>,
{
    let mut items = Vec::new();
    loop {
        match handle.case().await {
            sym::end => {
                handle.continue_();
                return items;
            }
            sym::item => {
                let item = readback_item(handle.receive()).await;
                items.push(item);
            }
            _ => unreachable!(),
        }
    }
}

async fn list_sort(mut handle: Handle, descending: bool) {
    let mut items = readback_list(handle.receive(), |handle| async { handle.data().await }).await;
    sort_by_key(&mut items, descending, |item| item);
    provide_data_list(handle, items);
}

async fn list_sort_by(mut handle: Handle, descending: bool) {
    let items = readback_list(handle.receive(), |handle| async { handle }).await;
    let mut key_fn = handle.receive();
    let mut keyed = Vec::with_capacity(items.len());

    for mut item in items {
        let item_for_key = item.duplicate();
        let mut f = key_fn.duplicate();
        f.send().link(item_for_key);
        let key = f.data().await;
        keyed.push((key, item));
    }

    key_fn.erase();
    sort_by_key(&mut keyed, descending, |(key, _)| key);
    provide_handle_list(handle, keyed.into_iter().map(|(_, item)| item));
}

async fn list_sort_linear_by(mut handle: Handle, descending: bool) {
    let items = readback_list(handle.receive(), |handle| async { handle }).await;
    let mut key_fn = handle.receive();
    let mut keyed = Vec::with_capacity(items.len());

    for item in items {
        let mut f = key_fn.duplicate();
        f.send().link(item);
        let key = f.receive_data().await;
        keyed.push((key, f));
    }

    key_fn.erase();
    sort_by_key(&mut keyed, descending, |(key, _)| key);
    provide_handle_list(handle, keyed.into_iter().map(|(_, item)| item));
}

fn sort_by_key<T>(items: &mut [T], descending: bool, key: impl Fn(&T) -> &Data) {
    items.sort_by(|left, right| {
        if descending {
            key(right).cmp(key(left))
        } else {
            key(left).cmp(key(right))
        }
    });
}

fn provide_data_list(mut handle: Handle, items: Vec<Data>) {
    for item in items {
        handle.signal(sym::item);
        handle.send_data(&item);
    }
    handle.signal(sym::end);
    handle.break_();
}

fn provide_handle_list(mut handle: Handle, items: impl IntoIterator<Item = Handle>) {
    for item in items {
        handle.signal(sym::item);
        handle.send().link(item);
    }
    handle.signal(sym::end);
    handle.break_();
}
