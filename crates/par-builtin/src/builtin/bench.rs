use par_runtime::external_def;
use par_runtime::readback::Handle;

external_def! {
    @core/Bench.BlackBox => bench_black_box
}

async fn bench_black_box(mut handle: Handle) {
    let x = handle.receive();
    handle.link(x);
}
