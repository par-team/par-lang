use par_runtime::external_def;
use par_runtime::readback::Handle;

async fn debug_log(mut handle: Handle) {
    let string = handle.receive().string().await;
    eprintln!("{}", string.as_str());
    handle.break_();
}

external_def! {
    @core/Debug.Log => debug_log
}
