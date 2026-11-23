//package: core
use std::sync::{Arc, Mutex};

use par_runtime::external_def;
use par_runtime::readback::Handle;

external_def! {
    @core/Cell.{
        Cell => cell_new,
        Apply => cell_apply,
        ApplyAndGet => cell_apply_and_get,
    }
}

async fn cell_new(mut handle: Handle) {
    let value = handle.receive();
    provide_cell(handle, Arc::new(Held::new(value)));
}

async fn cell_apply(mut handle: Handle) {
    let f = handle.receive();
    provide_applying(handle, f, false);
}

async fn cell_apply_and_get(mut handle: Handle) {
    let f = handle.receive();
    provide_applying(handle, f, true);
}

/// A value held on the Rust side, closed once it's dropped.
struct Held(Mutex<Option<Handle>>);

impl Held {
    fn new(handle: Handle) -> Self {
        Self(Mutex::new(Some(handle)))
    }

    fn duplicate(&self) -> Handle {
        self.0
            .lock()
            .expect("lock failed")
            .as_mut()
            .expect("held value is only taken on drop")
            .duplicate()
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

/// Provides a function for `.access`, which replaces the value with `f` applied to it.
/// Its result is a copy of the new value if `and_get`, otherwise `!`.
fn provide_applying(handle: Handle, f: Handle, and_get: bool) {
    let f = Held::new(f);
    handle.provide_box(move |mut callback| {
        let mut f = f.duplicate();
        async move {
            f.send().link(callback.receive());
            if and_get {
                callback.send().link(f.duplicate());
            } else {
                callback.send().break_();
            }
            callback.link(f);
        }
    })
}
