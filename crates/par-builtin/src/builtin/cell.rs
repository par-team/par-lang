//package: core
use std::sync::{Arc, Mutex};

use par_runtime::external_def;
use par_runtime::readback::Handle;

external_def! {
    @core/Cell.{
        Cell => cell_new,
    }
}

async fn cell_new(mut handle: Handle) {
    let value = handle.receive();
    provide_cell(handle, Arc::new(Held::new(value)));
}

/// A value held on the Rust side, closed once it's dropped.
struct Held(Mutex<Option<Handle>>);

impl Held {
    fn new(handle: Handle) -> Self {
        Self(Mutex::new(Some(handle)))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Some(handle) = self.0.get_mut().expect("lock failed").take() {
            handle.close();
        }
    }
}

/// Provides a cell. All copies of it share the `value`, which is dropped with the last one.
fn provide_cell(handle: Handle, value: Arc<Held>) {
    handle.provide_box(move |mut handle| {
        let value = Arc::clone(&value);
        async move {
            match handle.case().await.as_str() {
                "access" => {
                    let mut f = handle.receive();
                    // The value is replaced before the cell is returned, so that accesses
                    // through the returned cell happen after this one.
                    let result = {
                        let mut value = value.0.lock().expect("lock failed");
                        let old = value.take().expect("held value is only taken on drop");
                        f.send().link(old);
                        let result = f.receive();
                        *value = Some(f);
                        result
                    };
                    handle.send().link(result);
                    provide_cell(handle, value);
                }
                _ => unreachable!(),
            }
        }
    })
}
