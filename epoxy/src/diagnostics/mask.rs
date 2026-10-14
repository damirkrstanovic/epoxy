use serde_json::{Map, Value, json};
use zeroize::Zeroize;

// JSON values may contain credentials even when the final output is masked.
struct SensitiveJson(Value);
impl Drop for SensitiveJson {
    fn drop(&mut self) {
        wipe_json(&mut self.0);
    }
}
fn wipe_json(value: &mut Value) {
    match std::mem::take(value) {
        Value::String(mut text) => text.zeroize(),
        Value::Object(fields) => {
            for (mut key, mut value) in fields {
                key.zeroize();
                wipe_json(&mut value);
            }
        }
        Value::Array(mut items) => items.iter_mut().for_each(wipe_json),
        _ => {}
    }
}

const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_DEPTH: usize = 16;
const MAX_NODES: usize = 128;

pub fn masked_wire(text: &str) -> Value {
    if text.len() > MAX_INPUT_BYTES {
        return json!({"trace_omitted": "input_too_large", "bytes": text.len()});
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return json!({"trace_omitted": "invalid_json", "bytes": text.len()});
    };
    let input = SensitiveJson(value);
    let mut remaining = MAX_NODES;
    if !within_budget(&input.0, 0, &mut remaining) {
        return json!({"trace_omitted": "budget_exceeded", "bytes": text.len()});
    }
    let value = sanitize(&input.0);
    drop(input);
    if value.to_string().len() > MAX_OUTPUT_BYTES {
        return json!({"trace_omitted": "output_too_large"});
    }
    value
}

fn within_budget(value: &Value, depth: usize, remaining: &mut usize) -> bool {
    if depth > MAX_DEPTH || *remaining == 0 {
        return false;
    }
    *remaining -= 1;
    match value {
        Value::Object(fields) => fields
            .values()
            .all(|v| within_budget(v, depth + 1, remaining)),
        Value::Array(items) => items.iter().all(|v| within_budget(v, depth + 1, remaining)),
        _ => true,
    }
}

pub fn known_operation(value: &str) -> Option<&'static str> {
    match value {
        "ON_OPEN_EVENT" => Some("ON_OPEN_EVENT"),
        "GET_INFO" => Some("GET_INFO"),
        "GET_PROVIDERS" => Some("GET_PROVIDERS"),
        "GET_TERMINALS" => Some("GET_TERMINALS"),
        "GET_CERTIFICATES" => Some("GET_CERTIFICATES"),
        "GET_SIGNED_XML" => Some("GET_SIGNED_XML"),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Root {
        operation: &'static str,
        response: bool,
    },
    Input(&'static str),
    Payload(&'static str),
    Certificate,
    Provider,
    Terminal,
}

enum Rule {
    Operation,
    Number,
    Alias,
    Mask,
    Document,
    Object(Shape),
    Array(Shape),
}

fn rule(shape: Shape, key: &str) -> Option<Rule> {
    use Rule::*;
    match (shape, key) {
        (Shape::Root { .. }, "operation") => Some(Operation),
        (Shape::Root { response: true, .. }, "status") => Some(Number),
        (
            Shape::Root {
                operation,
                response: false,
            },
            "input",
        ) if operation != "GET_PROVIDERS" && operation != "ON_OPEN_EVENT" => {
            Some(Object(Shape::Input(operation)))
        }
        (
            Shape::Root {
                operation,
                response: true,
            },
            "payload",
        ) => Some(Object(Shape::Payload(operation))),
        (Shape::Input("GET_INFO"), "sbSession" | "host" | "language") => Some(Mask),
        (Shape::Input("GET_TERMINALS"), "providerId") => Some(Number),
        (Shape::Input("GET_CERTIFICATES"), "terminalId") => Some(Number),
        (Shape::Input("GET_CERTIFICATES" | "GET_SIGNED_XML"), "pin") => Some(Mask),
        (Shape::Input("GET_SIGNED_XML"), "certificate") => Some(Object(Shape::Certificate)),
        (Shape::Input("GET_SIGNED_XML") | Shape::Payload("GET_SIGNED_XML"), "xml") => {
            Some(Document)
        }
        (Shape::Payload("GET_INFO"), "providerId" | "terminalId") => Some(Number),
        (Shape::Payload("GET_INFO"), "certificateId") => Some(Mask),
        (Shape::Payload("GET_PROVIDERS"), "providers") => Some(Array(Shape::Provider)),
        (Shape::Payload("GET_TERMINALS"), "terminals") => Some(Array(Shape::Terminal)),
        (Shape::Payload("GET_CERTIFICATES"), "certificates") => Some(Array(Shape::Certificate)),
        (Shape::Payload("ON_OPEN_EVENT"), "appBuild") => Some(Number),
        (
            Shape::Payload("ON_OPEN_EVENT"),
            "appName" | "appVersion" | "osName" | "osVersion" | "osArch",
        ) => Some(Mask),
        (Shape::Certificate, "alias") => Some(Alias),
        (Shape::Certificate | Shape::Provider | Shape::Terminal, "name") => Some(Mask),
        (Shape::Provider | Shape::Terminal, "id") => Some(Number),
        _ => None,
    }
}

fn sanitize(value: &Value) -> Value {
    let Some(fields) = value.as_object() else {
        return json!({"trace_omitted": "unsupported_shape"});
    };
    let Some(operation) = fields
        .get("operation")
        .and_then(Value::as_str)
        .and_then(known_operation)
    else {
        return json!({"trace_omitted": "unknown_operation"});
    };
    let response = fields.contains_key("status")
        || fields.contains_key("payload")
        || operation == "ON_OPEN_EVENT";
    if response && fields.contains_key("input") {
        return json!({"trace_omitted": "ambiguous_shape"});
    }
    sanitize_object(
        value,
        Shape::Root {
            operation,
            response,
        },
    )
}

fn sanitize_object(value: &Value, shape: Shape) -> Value {
    let Value::Object(fields) = value else {
        return json!("[MASKED]");
    };
    let mut result = Map::new();
    let mut unknown = 0;
    for (key, value) in fields {
        let safe = match rule(shape, key) {
            Some(Rule::Operation) => value
                .as_str()
                .and_then(known_operation)
                .map(Value::from)
                .unwrap_or(json!("[MASKED]")),
            Some(Rule::Number) => value.as_u64().map(Value::from).unwrap_or(json!("[MASKED]")),
            Some(Rule::Alias) => match value.as_str() {
                Some(s)
                    if s.len() <= 20 && !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    json!(s)
                }
                _ => json!("[MASKED]"),
            },
            Some(Rule::Mask) => json!("[MASKED]"),
            Some(Rule::Document) => json!({"masked": true, "bytes": value.as_str().map(str::len)}),
            Some(Rule::Object(child)) => sanitize_object(value, child),
            Some(Rule::Array(child)) => match value {
                Value::Array(items) => Value::Array(
                    items
                        .iter()
                        .map(|item| sanitize_object(item, child))
                        .collect(),
                ),
                _ => json!("[MASKED]"),
            },
            None => {
                unknown += 1;
                continue;
            }
        };
        result.insert(key.clone(), safe);
    }
    if unknown > 0 {
        result.insert("_masked_fields".into(), json!(unknown));
    }
    Value::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_credentials_documents_and_unknown_values() {
        let trace = masked_wire(r#"{"operation":"GET_SIGNED_XML","input":{"pin":"synthetic-secret","xml":"PHNlY3JldC8+","certificate":{"alias":"0","name":"Synthetic Person"},"extra":{"password":"another-secret"}}}"#).to_string();
        for secret in [
            "synthetic-secret",
            "PHNlY3JldC8+",
            "Synthetic Person",
            "another-secret",
        ] {
            assert!(!trace.contains(secret));
        }
        assert!(trace.contains("GET_SIGNED_XML"));
        assert!(trace.contains("\"alias\":\"0\""));
    }

    #[test]
    fn malformed_input_never_becomes_fallback_text() {
        let trace = masked_wire(r#"{"pin":"synthetic-secret""#).to_string();
        assert!(!trace.contains("synthetic-secret"));
        assert!(trace.contains("invalid_json"));
    }

    #[test]
    fn unknown_keys_and_values_are_masked() {
        let trace = masked_wire(r#"{"operation":"credential-in-operation","credential-in-key":"credential-in-value","input":{"host":"credential-in-host","name":"credential-in-name","PIN":"credential-in-pin","nested":[{"pin":"credential-in-array"}],"alias":"credential-in-alias"}}"#).to_string();
        assert!(!trace.contains("credential-in-"));
    }

    #[test]
    fn bounds_size_depth_and_container_count() {
        assert!(
            masked_wire(&"sensitive".repeat(10000))
                .to_string()
                .contains("input_too_large")
        );
        let deep = format!("{}\"sensitive\"{}", "[".repeat(20), "]".repeat(20));
        assert!(masked_wire(&deep).to_string().contains("budget_exceeded"));
        let array = serde_json::json!({"operation":"GET_CERTIFICATES","payload":{"certificates": vec![serde_json::json!({"name":"sensitive"}); 200]}}).to_string();
        assert!(masked_wire(&array).to_string().contains("budget_exceeded"));
    }

    #[test]
    fn safe_metadata_survives_but_identity_does_not() {
        let value = masked_wire(
            r#"{"operation":"GET_CERTIFICATES","status":0,"payload":{"certificates":[{"alias":"1","name":"Synthetic Čertificate"}],"sbSession":"encoded-secret"}}"#,
        );
        assert_eq!(value["status"], 0);
        assert_eq!(value["payload"]["certificates"][0]["alias"], "1");
        let terminals = masked_wire(
            r#"{"operation":"GET_TERMINALS","status":0,"payload":{"terminals":[{"id":1,"name":"Synthetic Token"}]}}"#,
        );
        assert_eq!(terminals["payload"]["terminals"][0]["id"], 1);
        assert!(!terminals.to_string().contains("Synthetic"));
        assert!(!value.to_string().contains("Synthetic"));
        assert!(!value.to_string().contains("encoded-secret"));
    }

    #[test]
    fn document_length_is_visible_without_document() {
        let value =
            masked_wire(r#"{"operation":"GET_SIGNED_XML","payload":{"xml":"PHNlY3JldC8+"}}"#);
        assert_eq!(value["payload"]["xml"]["bytes"], 12);
        assert!(!value.to_string().contains("PHNlY3JldC8+"));
    }

    #[test]
    fn misplaced_known_fields_cannot_disclose_credentials() {
        for input in [
            r#"{"operation":"GET_INFO","input":{"alias":"123456","id":123456,"status":123456,"certificate":{"alias":"123456"}}}"#,
            r#"{"operation":"GET_INFO","payload":{"providers":[{"id":123456}],"alias":"123456"}}"#,
            r#"{"operation":"GET_SIGNED_XML","input":{"certificate":{"alias":"0","input":{"terminalId":123456}}}}"#,
        ] {
            assert!(!masked_wire(input).to_string().contains("123456"));
        }
    }
}
