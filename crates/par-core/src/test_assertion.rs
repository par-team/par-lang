use std::sync::mpsc;

use par_runtime::atom::sym;
use par_runtime::readback::Handle;

#[derive(Debug, Clone, PartialEq)]
pub struct AssertionResult {
    pub description: String,
    pub passed: bool,
}

pub async fn provide_test(handle: Handle, sender: mpsc::Sender<AssertionResult>) {
    provide_test_inner(handle, sender);
}

fn provide_test_inner(handle: Handle, sender: mpsc::Sender<AssertionResult>) {
    handle.provide_box(move |mut handle| {
        let sender = sender.clone();
        async move {
            match handle.case().await {
                sym::assert => {
                    let description = handle.receive().string().await.as_str().to_string();
                    println!("{}", description);
                    let mut bool_handle = handle.receive();

                    let passed = match bool_handle.case().await {
                        sym::true_ => {
                            bool_handle.continue_();
                            true
                        }
                        sym::false_ => {
                            bool_handle.continue_();
                            false
                        }
                        variant => {
                            panic!(
                                "Test.assert expected Bool, got unexpected variant: {}",
                                variant
                            );
                        }
                    };

                    let result = AssertionResult {
                        description,
                        passed,
                    };
                    let _ = sender.send(result);

                    provide_test_inner(handle, sender);
                }
                sym::done => {
                    handle.break_();
                }
                sym::id => {
                    let argument = handle.send();
                    argument.provide_box(|mut handle| async move {
                        let arg = handle.receive();
                        handle.link(arg);
                    });

                    provide_test_inner(handle, sender);
                }
                sym::leak => {
                    let argument = handle.send();
                    argument.provide_box(|handle| async move {
                        drop(handle);
                    });

                    provide_test_inner(handle, sender);
                }
                other => {
                    panic!("Unexpected method call on Test: {}", other);
                }
            }
        }
    })
}
