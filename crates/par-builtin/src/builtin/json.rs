//package: core
use std::{collections::BTreeMap, sync::Arc};

use crate::builtin::list::readback_list;
use arcstr::literal;
use par_runtime::external_def;
use par_runtime::primitive::ParString;
use par_runtime::readback::Handle;
use serde_json::{Map, Number, Value};

external_def! {
    @core/Json.{
        Encode => json_encode,
        Decode => json_decode,
    }
}

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
            handle.signal(literal!("ok"));
            provide_json_value(handle, json);
        }
        Err(err) => {
            handle.signal(literal!("err"));
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
    match handle.case().await.as_str() {
        "null" => {
            handle.continue_();
            JsonValue::Null
        }
        "bool" => JsonValue::Bool(readback_bool(handle).await),
        "string" => JsonValue::String(handle.string().await.as_str().to_owned()),
        "number" => JsonValue::Number(handle.float().await),
        "list" => {
            JsonValue::List(readback_list(handle, |handle| Box::pin(readback_json(handle))).await)
        }
        "object" => JsonValue::Object(Box::pin(readback_object(handle)).await),
        _ => unreachable!(),
    }
}

async fn readback_object(mut handle: Handle) -> BTreeMap<String, JsonValue> {
    handle.signal(literal!("list"));
    let mut entries = BTreeMap::new();
    loop {
        match handle.case().await.as_str() {
            "end" => {
                handle.continue_();
                return entries;
            }
            "item" => {
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
    match handle.case().await.as_str() {
        "true" => {
            handle.continue_();
            true
        }
        "false" => {
            handle.continue_();
            false
        }
        _ => unreachable!(),
    }
}

fn provide_json_value(mut handle: Handle, value: JsonValue) {
    match value {
        JsonValue::Null => {
            handle.signal(literal!("null"));
            handle.break_();
        }
        JsonValue::Bool(value) => {
            handle.signal(literal!("bool"));
            provide_bool(handle, value);
        }
        JsonValue::String(value) => {
            handle.signal(literal!("string"));
            handle.provide_string(ParString::from(value));
        }
        JsonValue::Number(value) => {
            handle.signal(literal!("number"));
            handle.provide_float(value);
        }
        JsonValue::List(values) => {
            handle.signal(literal!("list"));
            for value in values {
                handle.signal(literal!("item"));
                provide_json_value(handle.send(), value);
            }
            handle.signal(literal!("end"));
            handle.break_();
        }
        JsonValue::Object(entries) => {
            handle.signal(literal!("object"));
            provide_object(handle, entries);
        }
    }
}

fn provide_object(handle: Handle, entries: BTreeMap<String, JsonValue>) {
    let entries = Arc::new(entries);
    handle.provide_box(move |mut handle| {
        let entries = entries.clone();
        async move {
            match handle.case().await.as_str() {
                "size" => handle.provide_nat(entries.len().into()),
                "keys" => {
                    for key in entries.keys() {
                        handle.signal(literal!("item"));
                        handle.send().provide_string(ParString::from(key.clone()));
                    }
                    handle.signal(literal!("end"));
                    handle.break_();
                }
                "list" => {
                    for (key, value) in entries.iter() {
                        handle.signal(literal!("item"));
                        let mut pair = handle.send();
                        pair.send().provide_string(ParString::from(key.clone()));
                        provide_json_value(pair, value.clone());
                    }
                    handle.signal(literal!("end"));
                    handle.break_();
                }
                "get" => {
                    let key = handle.receive().string().await;
                    match entries.get(key.as_str()) {
                        Some(value) => {
                            handle.signal(literal!("some"));
                            provide_json_value(handle, value.clone());
                        }
                        None => {
                            handle.signal(literal!("none"));
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
        handle.signal(literal!("true"));
    } else {
        handle.signal(literal!("false"));
    }
    handle.break_();
}
