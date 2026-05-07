//package: core
use par_runtime::atom::sym;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};

macro_rules! core_data_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "Data",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_data_external!("ToString", data_to_string);
core_data_external!("Compare", data_compare);

async fn data_to_string(mut handle: Handle) {
    let value = handle.receive().data().await;
    handle.provide_string(value.to_string().into());
}

async fn data_compare(mut handle: Handle) {
    let mut pair = handle.receive();
    let left = pair.receive_data().await;
    let right = pair.data().await;
    match left.cmp(&right) {
        std::cmp::Ordering::Less => handle.signal(sym::less),
        std::cmp::Ordering::Equal => handle.signal(sym::equal),
        std::cmp::Ordering::Greater => handle.signal(sym::greater),
    }
    handle.break_();
}
