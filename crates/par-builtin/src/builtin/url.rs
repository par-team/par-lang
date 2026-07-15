use arcstr::literal;
use par_runtime::external_def;
use percent_encoding::percent_decode_str;
use url::Url as ParsedUrl;

use par_runtime::primitive::ParString;
use par_runtime::readback::Handle;

external_def! {
    @core/Url.FromString => url_from_string
}

async fn url_from_string(mut handle: Handle) {
    let input = handle.receive().string().await;
    match ParsedUrl::parse(input.as_str()) {
        Ok(url) => {
            handle.signal(literal!("ok"));
            provide_url_value(handle, url);
        }
        Err(err) => {
            handle.signal(literal!("err"));
            handle.provide_string(ParString::from(err.to_string()));
        }
    }
}

pub(super) fn provide_url_value(handle: Handle, url: ParsedUrl) {
    handle.provide_box(move |mut handle| {
        let mut url = url.clone();
        async move {
            loop {
                match handle.case().await.as_str() {
                    "full" => {
                        handle.provide_string(ParString::copy_from_slice(url.as_str()));
                        return;
                    }
                    "protocol" => {
                        handle.provide_string(ParString::copy_from_slice(url.scheme()));
                        return;
                    }
                    "host" => {
                        let host = match url.port() {
                            Some(port) => format!("{}:{}", url.host_str().unwrap_or(""), port),
                            None => url.host_str().unwrap_or("").to_string(),
                        };
                        handle.provide_string(ParString::from(host));
                        return;
                    }
                    "path" => {
                        let decoded = percent_decode_str(url.path()).decode_utf8_lossy();
                        handle.provide_string(decoded.into_owned().into());
                        return;
                    }
                    "query" => {
                        for (key, value) in url.query_pairs() {
                            handle.signal(literal!("item"));
                            let key = key.into_owned();
                            let value = value.into_owned();
                            handle.send().concurrently(|mut pair| async move {
                                pair.send().provide_string(ParString::from(key));
                                pair.provide_string(ParString::from(value));
                            });
                        }
                        handle.signal(literal!("end"));
                        handle.break_();
                        return;
                    }
                    "appendPath" => {
                        let segment = handle.receive().string().await;
                        append_path(&mut url, segment.as_str());
                        return provide_url_value(handle, url);
                    }
                    "addQuery" => {
                        let key = handle.receive().string().await;
                        let value = handle.receive().string().await;
                        url.query_pairs_mut()
                            .append_pair(key.as_str(), value.as_str());
                        return provide_url_value(handle, url);
                    }
                    _ => unreachable!(),
                }
            }
        }
    })
}

fn append_path(url: &mut ParsedUrl, segment: &str) {
    if segment.is_empty() {
        return;
    }

    let parts: Vec<&str> = segment.split('/').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() || url.cannot_be_a_base() {
        return;
    }

    if url.path().is_empty() {
        url.set_path("/");
    }

    let mut segments = url.path_segments_mut().expect("base URL");
    for part in parts {
        segments.push(part);
    }
}
