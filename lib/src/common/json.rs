#![allow(unused_imports)]

use std::{borrow::Cow, collections::HashMap};

use opentelemetry::{Array, Key, Value};
use opentelemetry_api::logs::AnyValue;

#[cfg(feature = "json")]
pub(crate) fn get_attributes_as_json<'a, C>(attribs: C) -> String
where C: Iterator<Item = (&'a Key, &'a Value)>
{
    let mut payload: std::collections::BTreeMap<String, serde_json::Value> = Default::default();

    for attrib in attribs {
        let field_name = &attrib.0.to_string();
        match attrib.1 {
            Value::Bool(b) => {
                payload.insert(field_name.clone(), serde_json::Value::Bool(*b));
            }
            Value::I64(i) => {
                payload.insert(
                    field_name.clone(),
                    serde_json::Value::Number(serde_json::Number::from(*i)),
                );
            }
            Value::F64(f) => {
                payload.insert(
                    field_name.clone(),
                    serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap()),
                );
            }
            Value::String(s) => {
                payload.insert(field_name.clone(), serde_json::Value::String(s.to_string()));
            }
            Value::Array(array) => match array {
                Array::Bool(v) => {
                    payload.insert(
                        field_name.clone(),
                        serde_json::Value::Array(
                            v.iter().map(|b| serde_json::Value::Bool(*b)).collect(),
                        ),
                    );
                }
                Array::I64(v) => {
                    payload.insert(
                        field_name.clone(),
                        serde_json::Value::Array(
                            v.iter()
                                .map(|i| serde_json::Value::Number(serde_json::Number::from(*i)))
                                .collect(),
                        ),
                    );
                }
                Array::F64(v) => {
                    payload.insert(
                        field_name.clone(),
                        serde_json::Value::Array(
                            v.iter()
                                .map(|f| {
                                    serde_json::Value::Number(
                                        serde_json::Number::from_f64(*f).unwrap(),
                                    )
                                })
                                .collect(),
                        ),
                    );
                }
                Array::String(v) => {
                    payload.insert(
                        field_name.clone(),
                        serde_json::Value::Array(
                            v.iter()
                                .map(|s| serde_json::Value::String(s.to_string()))
                                .collect(),
                        ),
                    );
                }
            },
        }
    }

    if let Ok(json_string) = serde_json::to_string(&payload) {
        json_string
    } else {
        todo!()
    }
}

#[cfg(feature = "json")]
fn get_value(av: &AnyValue) -> serde_json::Value {
    match av {
        AnyValue::Boolean(b) => {
            serde_json::Value::Bool(*b)
        }
        AnyValue::Int(i) => {
                serde_json::Value::Number(serde_json::Number::from(*i))
        }
        AnyValue::Double(f) => {
                serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap())
        }
        AnyValue::String(s) => {
            serde_json::Value::String(s.to_string())
        }
        AnyValue::ListAny(values) => {
            serde_json::Value::Array(values.iter().map(|v| get_value(v)).collect())
        }
        AnyValue::Bytes(bs) => {
            // TODO: Probably not the best way to represent a byte array. Maybe hex string instead?
            serde_json::Value::Array(bs.iter().map(|b| serde_json::Value::Number(serde_json::Number::from(*b))).collect())
        }
        AnyValue::Map(m) => {
            let mut map = serde_json::Map::new();
            for (k, v) in m {
                map.insert(k.to_string(), get_value(v));
            }
            serde_json::Value::Object(map)
        }
    }
}

#[cfg(feature = "json")]
pub(crate) fn get_log_attributes_as_json<'a, C>(attribs: C) -> String
where C: Iterator<Item = (&'a Key, &'a AnyValue)> {
    let mut payload: std::collections::BTreeMap<String, serde_json::Value> = Default::default();

    for attrib in attribs {
        let field_name = attrib.0.to_string();
        let value = get_value(attrib.1);

        payload.insert(field_name, value);
    }

    if let Ok(json_string) = serde_json::to_string(&payload) {
        json_string
    } else {
        todo!()
    }
}

#[allow(dead_code)]
pub(crate) fn extract_common_schema_parta_exts<'a, C>(
    attributes: C,
) -> HashMap<&'static str, Vec<(&'static str, Cow<'a, str>)>>
where
    C: IntoIterator<Item = (&'a Key, &'a Value)>,
{
    // Pull out PartA fields from the resource

    let mut has_cloud = false;
    let mut service_name: Cow<str> = Cow::default();
    let mut service_namespace: Cow<str> = Cow::default();
    let mut service_instance_id: Cow<str> = Cow::default();
    let mut enduser_id: Cow<str> = Cow::default();

    for cfg in attributes {
        let key_str = cfg.0.as_str();
        match key_str {
            "service.namespace" => {
                service_namespace = cfg.1.as_str();
                has_cloud = true;
            }
            "service.name" => {
                service_name = cfg.1.as_str();
                has_cloud = true;
            }
            "service.instance.id" => {
                service_instance_id = cfg.1.as_str();
                has_cloud = true;
            }
            "enduser.id" => enduser_id = cfg.1.as_str(),
            // TODO: Part A ext "sdk.ver"
            _ => (),
        }
    }

    #[allow(non_snake_case)]
    let mut partA_exts = HashMap::with_capacity(2);
    if has_cloud {
        let mut values = Vec::<(&'static str, Cow<str>)>::with_capacity(2);
        if !service_name.is_empty() && !service_namespace.is_empty() {
            values.push((
                "role",
                Cow::Owned(std::fmt::format(format_args!(
                    "[{service_namespace}]/{service_name}"
                ))),
            ));
        } else if !service_name.is_empty() {
            values.push(("role", service_name));
        } else if !service_namespace.is_empty() {
            values.push(("role", service_namespace));
        }

        if !service_instance_id.is_empty() {
            values.push(("roleInstance", service_instance_id));
        } else {
            // TODO: Get machine hostname
        }

        partA_exts.insert("ext_cloud", values);
    }

    if !enduser_id.is_empty() {
        let values: Vec<(&str, Cow<str>)> = vec![("userId", enduser_id)];

        partA_exts.insert("ext_app", values);
    }

    partA_exts
}
