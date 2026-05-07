//package: basic
use std::io::{Write, stdin, stdout};

use par_runtime::atom::sym;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};

use par_core::frontend::ParString;

async fn console_open(mut handle: Handle) {
    loop {
        match handle.case().await {
            sym::close => {
                handle.break_();
                break;
            }

            sym::print => {
                println!("{}", handle.receive().string().await.as_str(),);
            }

            sym::prompt => {
                let prompt = handle.receive().string().await;
                print!("{}", prompt.as_str());
                let _ = stdout().flush();
                let mut buf = String::new();
                let result = stdin().read_line(&mut buf);

                handle.send().concurrently(|mut handle| async move {
                    match result {
                        Ok(n) if n > 0 => {
                            let string =
                                ParString::copy_from_slice(buf.trim_end_matches(&['\n', '\r']));
                            handle.signal(sym::ok);
                            handle.provide_string(string);
                        }
                        _ => {
                            handle.signal(sym::err);
                            handle.break_();
                        }
                    }
                });
            }
            _ => unreachable!(),
        }
    }
}

macro_rules! basic_console_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::BASIC,
                path: &[],
                module: "Console",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

basic_console_external!("Open", console_open);
