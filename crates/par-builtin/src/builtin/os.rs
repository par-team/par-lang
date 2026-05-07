//package: basic
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use futures::future::BoxFuture;
use num_bigint::BigUint;
use par_runtime::atom::sym;
use par_runtime::primitive::ParString;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};
use tokio::{
    fs::{self, DirEntry, File, OpenOptions, ReadDir},
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};

macro_rules! basic_os_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::BASIC,
                path: &[],
                module: "Os",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

basic_os_external!("Path", path_from_bytes);
basic_os_external!("Stdin", os_stdin);
basic_os_external!("Stdout", os_stdout);
basic_os_external!("Stderr", os_stderr);
basic_os_external!("OpenFile", os_open_file);
basic_os_external!("CreateOrReplaceFile", os_create_or_replace_file);
basic_os_external!("CreateNewFile", os_create_new_file);
basic_os_external!("AppendToFile", os_append_to_file);
basic_os_external!("CreateOrAppendToFile", os_create_or_append_to_file);
basic_os_external!("CreateDir", os_create_dir);
basic_os_external!("RemoveFile", os_remove_file);
basic_os_external!("RemoveDir", os_remove_dir);
basic_os_external!("MoveFile", os_move_file);
basic_os_external!("MoveDir", os_move_dir);
basic_os_external!("ListDir", os_list_dir);
basic_os_external!("TraverseDir", os_traverse_dir);
basic_os_external!("Env", envmap_new);

async fn path_from_bytes(mut handle: Handle) {
    let b = handle.receive().bytes().await;
    // Unsafe: we accept arbitrary OS-encoded bytes without validation
    let os: &OsStr = unsafe { OsStr::from_encoded_bytes_unchecked(b.as_ref()) };
    let p = PathBuf::from(os);
    provide_path(handle, p);
}

fn provide_path(handle: Handle, path: PathBuf) {
    handle.provide_box(move |mut handle| {
        let path = path.clone();
        async move {
            match handle.case().await {
                sym::name => {
                    let bytes = path
                        .file_name()
                        .map(|n| os_to_bytes(n))
                        .unwrap_or_else(|| Bytes::new());
                    handle.provide_bytes(bytes);
                }
                sym::absolute => {
                    let abs = absolute_path(&path);
                    let bytes = os_to_bytes(abs.as_os_str());
                    handle.provide_bytes(bytes);
                }
                sym::parts => {
                    provide_bytes_parts(handle, &path);
                }
                sym::parent => match path.parent() {
                    Some(p) => {
                        handle.signal(sym::some);
                        provide_path(handle, p.to_path_buf());
                    }
                    None => {
                        handle.signal(sym::none);
                        handle.break_();
                    }
                },
                sym::append => {
                    let b = handle.receive().bytes().await;
                    let os: &OsStr = unsafe { OsStr::from_encoded_bytes_unchecked(b.as_ref()) };
                    let p2 = path.join(Path::new(os));
                    provide_path(handle, p2);
                }
                _ => unreachable!(),
            }
        }
    });
}

fn absolute_path(p: &Path) -> PathBuf {
    match p.canonicalize() {
        Ok(abs) => abs,
        Err(_) => {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(p)
            }
        }
    }
}

fn provide_bytes_parts(mut handle: Handle, p: &Path) {
    for part in p.iter() {
        handle.signal(sym::item);
        let bytes = os_to_bytes(part);
        handle.send().provide_bytes(bytes);
    }
    handle.signal(sym::end);
    handle.break_();
}

#[cfg(unix)]
fn os_to_bytes(os: &OsStr) -> Bytes {
    use std::os::unix::ffi::OsStrExt;
    Bytes::copy_from_slice(os.as_bytes())
}

#[cfg(windows)]
fn os_to_bytes(os: &OsStr) -> Bytes {
    Bytes::copy_from_slice(os.as_encoded_bytes())
}

#[cfg(not(any(unix, windows)))]
fn os_to_bytes(os: &OsStr) -> Bytes {
    Bytes::from(os.to_string_lossy().as_ref())
}

async fn provide_bytes_reader_from_async(mut handle: Handle, mut reader: impl AsyncRead + Unpin) {
    let mut buf = vec![0u8; 512];
    loop {
        match handle.case().await {
            sym::close => {
                handle.signal(sym::ok);
                return handle.break_();
            }
            sym::read => match reader.read(&mut buf[..]).await {
                Ok(n) => {
                    if n == 0 {
                        handle.signal(sym::ok);
                        handle.signal(sym::end);
                        return handle.break_();
                    }
                    handle.signal(sym::ok);
                    handle.signal(sym::chunk);
                    handle
                        .send()
                        .provide_bytes(Bytes::copy_from_slice(&buf[..n]));
                    continue;
                }
                Err(err) => {
                    handle.signal(sym::err);
                    return handle.provide_string(ParString::from(err.to_string()));
                }
            },
            _ => unreachable!(),
        }
    }
}

async fn provide_bytes_writer_from_async(mut handle: Handle, mut writer: impl AsyncWrite + Unpin) {
    loop {
        match handle.case().await {
            sym::close => {
                // Try to flush pending data before closing
                match writer.flush().await {
                    Ok(()) => {
                        handle.signal(sym::ok);
                        return handle.break_();
                    }
                    Err(err) => {
                        handle.signal(sym::err);
                        return handle.provide_string(ParString::from(err.to_string()));
                    }
                }
            }
            sym::flush => match writer.flush().await {
                Ok(()) => {
                    handle.signal(sym::ok);
                    continue;
                }
                Err(err) => {
                    handle.signal(sym::err);
                    return handle.provide_string(ParString::from(err.to_string()));
                }
            },
            sym::write => {
                let bytes = handle.receive().bytes().await;
                match writer.write_all(bytes.as_ref()).await {
                    Ok(()) => {
                        handle.signal(sym::ok);
                        continue;
                    }
                    Err(err) => {
                        handle.signal(sym::err);
                        return handle.provide_string(ParString::from(err.to_string()));
                    }
                }
            }
            _ => unreachable!(),
        }
    }
}

fn provide_unit_io_result(mut handle: Handle, result: std::io::Result<()>) {
    match result {
        Ok(()) => {
            handle.signal(sym::ok);
            handle.break_();
        }
        Err(err) => {
            handle.signal(sym::err);
            handle.provide_string(ParString::from(err.to_string()));
        }
    }
}

// Provide List<Os.Path> for the directory entries of `base` using a pre-opened ReadDir.
async fn provide_list_dir(mut handle: Handle, base: &Path, rd: &mut ReadDir) {
    let mut entries: Vec<(Bytes, std::ffi::OsString)> = Vec::new();
    while let Ok(Some(entry)) = rd.next_entry().await {
        let name = entry.file_name();
        // Sort key: raw bytes if available, fallback to lossy string
        let key = os_to_bytes(&name);
        entries.push((key, name));
    }
    // Sort deterministically by the byte-representation of file name
    entries.sort_by(|(a, _), (b, _)| a.as_ref().cmp(b.as_ref()));

    for (_, name) in entries {
        let child = base.join(Path::new(&name));
        handle.signal(sym::item);
        provide_path(handle.send(), child);
    }
    handle.signal(sym::end);
    handle.break_();
}

// Directory tree node used for traverseDir
enum DirNode {
    File(PathBuf),
    Dir {
        path: PathBuf,
        children: Vec<DirNode>,
    },
}

// Recursively build the full directory tree. Returns an error message if any IO fails.
fn build_dir_tree(dir: PathBuf) -> BoxFuture<'static, Result<Vec<DirNode>, String>> {
    Box::pin(async move {
        let mut rd = fs::read_dir(&dir).await.map_err(|e| format!("{}", e))?;

        // Collect entries first to allow deterministic sorting
        let mut items: Vec<(Bytes, DirEntry)> = Vec::new();
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name();
            let key = os_to_bytes(&name);
            items.push((key, entry));
        }
        items.sort_by(|(a, _), (b, _)| a.as_ref().cmp(b.as_ref()));

        let mut result = Vec::new();
        for (_, entry) in items {
            let ty = entry.file_type().await.map_err(|e| format!("{}", e))?;
            let child_path = entry.path();
            if ty.is_dir() {
                let children = build_dir_tree(child_path.clone()).await?;
                result.push(DirNode::Dir {
                    path: child_path,
                    children,
                });
            } else {
                // Treat symlinks and others as files to avoid cycles
                result.push(DirNode::File(child_path));
            }
        }
        Ok(result)
    })
}

fn provide_dir_tree<'a>(mut handle: Handle, nodes: &'a [DirNode]) {
    match nodes.split_first() {
        None => {
            handle.signal(sym::end);
            handle.break_();
        }
        Some((node, tail)) => match node {
            DirNode::File(path) => {
                handle.signal(sym::file);
                provide_path(handle.send(), path.clone());
                provide_dir_tree(handle, tail);
            }
            DirNode::Dir { path, children } => {
                handle.signal(sym::dir);
                provide_path(handle.send(), path.clone());
                provide_dir_tree(handle.send(), children.as_slice());
                provide_dir_tree(handle, tail);
            }
        },
    }
}

async fn os_stdin(handle: Handle) {
    provide_bytes_reader_from_async(handle, tokio::io::stdin()).await;
}

async fn os_stdout(handle: Handle) {
    provide_bytes_writer_from_async(handle, tokio::io::stdout()).await;
}

async fn os_stderr(handle: Handle) {
    provide_bytes_writer_from_async(handle, tokio::io::stderr()).await;
}

async fn os_open_file(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    match File::open(&path).await {
        Ok(file) => {
            handle.signal(sym::ok);
            return provide_bytes_reader_from_async(handle, file).await;
        }
        Err(err) => {
            handle.signal(sym::err);
            return handle.provide_string(ParString::from(err.to_string()));
        }
    }
}

async fn os_create_or_replace_file(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .await
    {
        Ok(file) => {
            handle.signal(sym::ok);
            return provide_bytes_writer_from_async(handle, file).await;
        }
        Err(err) => {
            handle.signal(sym::err);
            return handle.provide_string(ParString::from(err.to_string()));
        }
    }
}

async fn os_create_new_file(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .await
    {
        Ok(file) => {
            handle.signal(sym::ok);
            return provide_bytes_writer_from_async(handle, file).await;
        }
        Err(err) => {
            handle.signal(sym::err);
            return handle.provide_string(ParString::from(err.to_string()));
        }
    }
}

async fn os_append_to_file(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    match OpenOptions::new()
        .write(true)
        .append(true)
        .open(&path)
        .await
    {
        Ok(file) => {
            handle.signal(sym::ok);
            return provide_bytes_writer_from_async(handle, file).await;
        }
        Err(err) => {
            handle.signal(sym::err);
            return handle.provide_string(ParString::from(err.to_string()));
        }
    }
}

async fn os_create_or_append_to_file(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    match OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(&path)
        .await
    {
        Ok(file) => {
            handle.signal(sym::ok);
            return provide_bytes_writer_from_async(handle, file).await;
        }
        Err(err) => {
            handle.signal(sym::err);
            return handle.provide_string(ParString::from(err.to_string()));
        }
    }
}

async fn os_create_dir(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    match fs::create_dir_all(&path).await {
        Ok(()) => {
            handle.signal(sym::ok);
            return handle.break_();
        }
        Err(err) => {
            handle.signal(sym::err);
            return handle.provide_string(ParString::from(err.to_string()));
        }
    }
}

async fn os_remove_file(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    provide_unit_io_result(handle, fs::remove_file(&path).await);
}

async fn os_remove_dir(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    provide_unit_io_result(handle, fs::remove_dir(&path).await);
}

async fn os_move_file(mut handle: Handle) {
    let src = pathbuf_from_os_path(handle.receive()).await;
    let dst = pathbuf_from_os_path(handle.receive()).await;
    provide_unit_io_result(handle, fs::rename(&src, &dst).await);
}

async fn os_move_dir(mut handle: Handle) {
    let src = pathbuf_from_os_path(handle.receive()).await;
    let dst = pathbuf_from_os_path(handle.receive()).await;
    provide_unit_io_result(handle, fs::rename(&src, &dst).await);
}

async fn os_list_dir(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    match fs::read_dir(&path).await {
        Ok(mut rd) => {
            handle.signal(sym::ok);
            return provide_list_dir(handle, &path, &mut rd).await;
        }
        Err(err) => {
            handle.signal(sym::err);
            return handle.provide_string(ParString::from(err.to_string()));
        }
    }
}

async fn os_traverse_dir(mut handle: Handle) {
    let path = pathbuf_from_os_path(handle.receive()).await;
    match build_dir_tree(path.clone()).await {
        Ok(nodes) => {
            handle.signal(sym::ok);
            return provide_dir_tree(handle, nodes.as_slice());
        }
        Err(err) => {
            handle.signal(sym::err);
            return handle.provide_string(ParString::from(err));
        }
    }
}

async fn envmap_new(handle: Handle) {
    handle.provide_box(move |mut handle| async move {
        match handle.case().await {
            sym::size => {
                return handle.provide_nat(BigUint::from(std::env::vars_os().count()));
            }
            sym::keys => {
                let vars: Vec<_> = std::env::vars_os().into_iter().collect();
                for (name, _) in vars {
                    handle.signal(sym::item);
                    handle.send().provide_bytes(os_to_bytes(&name));
                }
                handle.signal(sym::end);
                return handle.break_();
            }
            sym::list => {
                let vars: Vec<_> = std::env::vars_os().into_iter().collect();
                for (name, value) in vars {
                    handle.signal(sym::item);
                    let mut pair = handle.send();
                    pair.send().provide_bytes(os_to_bytes(&name));
                    pair.provide_bytes(os_to_bytes(&value));
                }
                handle.signal(sym::end);
                return handle.break_();
            }
            sym::get => {
                let name = handle.receive().bytes().await;
                let name_os: &OsStr = unsafe { OsStr::from_encoded_bytes_unchecked(name.as_ref()) };
                match std::env::var_os(name_os) {
                    Some(val) => {
                        let bytes = os_to_bytes(&val);
                        handle.signal(sym::some);
                        return handle.provide_bytes(bytes);
                    }
                    None => {
                        handle.signal(sym::none);
                        return handle.break_();
                    }
                }
            }
            _ => unreachable!(),
        }
    });
}

async fn pathbuf_from_os_path(mut handle: Handle) -> PathBuf {
    handle.signal(sym::absolute);
    let path_bytes = handle.bytes().await;
    let os_str = unsafe { OsStr::from_encoded_bytes_unchecked(&path_bytes) };
    PathBuf::from(os_str)
}
