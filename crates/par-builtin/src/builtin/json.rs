//package: core
use std::{collections::BTreeMap, sync::Arc};

use crate::builtin::list::readback_list;
use par_runtime::atom::sym;
use par_runtime::primitive::ParString;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};
use serde_json::{Map, Number, Value};

macro_rules! core_json_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "Json",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_json_external!("Encode", json_encode);
core_json_external!("Decode", json_decode);

#[derive(Clone, Debug)]
enum JsonValue {
    Null,
    Bool(bool),
    String(String),
    Number(f64),
    List(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

async fn json_encode(mut handle: Handle) {
    let json = readback_json(handle.receive()).await;
    let encoded = encode_to_string(&json).expect("JSON encoding should not fail");
    handle.provide_string(ParString::from(encoded));
}

async fn json_decode(mut handle: Handle) {
    let string = handle.receive().string().await;
    match decode_from_string(string.as_str()) {
        Ok(json) => {
            handle.signal(sym::ok);
            provide_json_value(handle, json);
        }
        Err(err) => {
            handle.signal(sym::err);
            handle.provide_string(ParString::from(err));
        }
    }
}

fn encode_to_string(value: &JsonValue) -> Result<String, serde_json::Error> {
    serde_json::to_string(&json_to_serde(value))
}

fn decode_from_string(string: &str) -> Result<JsonValue, String> {
    let value = serde_json::from_str::<Value>(string).map_err(|err| err.to_string())?;
    serde_to_json(value)
}

fn json_to_serde(value: &JsonValue) -> Value {
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(value) => Value::Bool(*value),
        JsonValue::String(value) => Value::String(value.clone()),
        JsonValue::Number(value) => {
            if value.is_finite() {
                match Number::from_f64(*value) {
                    Some(number) => Value::Number(number),
                    None => Value::Null,
                }
            } else {
                Value::Null
            }
        }
        JsonValue::List(values) => Value::Array(values.iter().map(json_to_serde).collect()),
        JsonValue::Object(entries) => {
            let mut object = Map::new();
            for (key, value) in entries {
                object.insert(key.clone(), json_to_serde(value));
            }
            Value::Object(object)
        }
    }
}

fn serde_to_json(value: Value) -> Result<JsonValue, String> {
    match value {
        Value::Null => Ok(JsonValue::Null),
        Value::Bool(value) => Ok(JsonValue::Bool(value)),
        Value::String(value) => Ok(JsonValue::String(value)),
        Value::Number(value) => value
            .as_f64()
            .map(JsonValue::Number)
            .ok_or_else(|| format!("JSON number is out of range for Float: {value}")),
        Value::Array(values) => values
            .into_iter()
            .map(serde_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(JsonValue::List),
        Value::Object(entries) => entries
            .into_iter()
            .map(|(key, value)| serde_to_json(value).map(|value| (key, value)))
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(JsonValue::Object),
    }
}

async fn readback_json(mut handle: Handle) -> JsonValue {
    match handle.case().await {
        sym::null => {
            handle.continue_();
            JsonValue::Null
        }
        sym::bool => JsonValue::Bool(readback_bool(handle).await),
        sym::string => JsonValue::String(handle.string().await.as_str().to_owned()),
        sym::number => JsonValue::Number(handle.float().await),
        sym::list => {
            JsonValue::List(readback_list(handle, |handle| Box::pin(readback_json(handle))).await)
        }
        sym::object => JsonValue::Object(Box::pin(readback_object(handle)).await),
        _ => unreachable!(),
    }
}

async fn readback_object(mut handle: Handle) -> BTreeMap<String, JsonValue> {
    handle.signal(sym::list);
    let mut entries = BTreeMap::new();
    loop {
        match handle.case().await {
            sym::end => {
                handle.continue_();
                return entries;
            }
            sym::item => {
                let mut pair = handle.receive();
                let key = pair.receive().string().await.as_str().to_owned();
                let value = Box::pin(readback_json(pair)).await;
                entries.insert(key, value);
            }
            _ => unreachable!(),
        }
    }
}

async fn readback_bool(mut handle: Handle) -> bool {
    match handle.case().await {
        sym::true_ => {
            handle.continue_();
            true
        }
        sym::false_ => {
            handle.continue_();
            false
        }
        _ => unreachable!(),
    }
}

fn provide_json_value(mut handle: Handle, value: JsonValue) {
    match value {
        JsonValue::Null => {
            handle.signal(sym::null);
            handle.break_();
        }
        JsonValue::Bool(value) => {
            handle.signal(sym::bool);
            provide_bool(handle, value);
        }
        JsonValue::String(value) => {
            handle.signal(sym::string);
            handle.provide_string(ParString::from(value));
        }
        JsonValue::Number(value) => {
            handle.signal(sym::number);
            handle.provide_float(value);
        }
        JsonValue::List(values) => {
            handle.signal(sym::list);
            for value in values {
                handle.signal(sym::item);
                provide_json_value(handle.send(), value);
            }
            handle.signal(sym::end);
            handle.break_();
        }
        JsonValue::Object(entries) => {
            handle.signal(sym::object);
            provide_object(handle, entries);
        }
    }
}

fn provide_object(handle: Handle, entries: BTreeMap<String, JsonValue>) {
    let entries = Arc::new(entries);
    handle.provide_box(move |mut handle| {
        let entries = entries.clone();
        async move {
            match handle.case().await {
                sym::size => handle.provide_nat(entries.len().into()),
                sym::keys => {
                    for key in entries.keys() {
                        handle.signal(sym::item);
                        handle.send().provide_string(ParString::from(key.clone()));
                    }
                    handle.signal(sym::end);
                    handle.break_();
                }
                sym::list => {
                    for (key, value) in entries.iter() {
                        handle.signal(sym::item);
                        let mut pair = handle.send();
                        pair.send().provide_string(ParString::from(key.clone()));
                        provide_json_value(pair, value.clone());
                    }
                    handle.signal(sym::end);
                    handle.break_();
                }
                sym::get => {
                    let key = handle.receive().string().await;
                    match entries.get(key.as_str()) {
                        Some(value) => {
                            handle.signal(sym::some);
                            provide_json_value(handle, value.clone());
                        }
                        None => {
                            handle.signal(sym::none);
                            handle.break_();
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
    })
}

fn provide_bool(mut handle: Handle, value: bool) {
    if value {
        handle.signal(sym::true_);
    } else {
        handle.signal(sym::false_);
    }
    handle.break_();
}
