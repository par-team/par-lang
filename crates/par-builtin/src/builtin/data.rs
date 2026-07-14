//package: core
use arcstr::literal;
use par_runtime::external_def;
use par_runtime::readback::Handle;

external_def! {
    @core/Data.{
        ToString => data_to_string,
        Compare => data_compare,
    }
}

async fn data_to_string(mut handle: Handle) {
    let value = handle.receive().data().await;
    handle.provide_string(value.to_string().into());
}

async fn data_compare(mut handle: Handle) {
    let mut pair = handle.receive();
    let left = pair.receive_data().await;
    let right = pair.data().await;
    match left.cmp(&right) {
        std::cmp::Ordering::Less => handle.signal(literal!("less")),
        std::cmp::Ordering::Equal => handle.signal(literal!("equal")),
        std::cmp::Ordering::Greater => handle.signal(literal!("greater")),
    }
    handle.break_();
}
