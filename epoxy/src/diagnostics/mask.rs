use serde_json::{Map, Value, json};

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
    let mut remaining = MAX_NODES;
    if !within_budget(&value, 0, &mut remaining) {
        return json!({"trace_omitted": "budget_exceeded", "bytes": text.len()});
    }
    let value = sanitize(value);
    if value.to_string().len() > MAX_OUTPUT_BYTES {
        return json!({"trace_omitted": "output_too_large"});
    }
    value
}

fn within_budget(value: &Value, depth: usize, remaining: &mut usize) -> bool {
    if depth > MAX_DEPTH || *remaining == 0 { return false; }
    *remaining -= 1;
    match value {
        Value::Object(fields) => fields.values().all(|v| within_budget(v, depth + 1, remaining)),
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

fn sanitize(value: Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut result = Map::new();
            let mut unknown = 0;
            for (key, value) in fields {
                let safe = match key.as_str() {
                    "operation" => value.as_str().and_then(known_operation).map(Value::from).unwrap_or(json!("[MASKED]")),
                    "status" | "id" | "providerId" | "terminalId" | "appBuild" => value.as_u64().map(Value::from).unwrap_or(json!("[MASKED]")),
                    "alias" => match value.as_str() {
                        Some(s) if s.len() <= 20 && !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => json!(s),
                        _ => json!("[MASKED]"),
                    },
                    "input" | "payload" | "certificate" => if value.is_object() { sanitize(value) } else { json!("[MASKED]") },
                    "providers" | "terminals" | "certificates" => if value.is_array() { sanitize(value) } else { json!("[MASKED]") },
                    "xml" => json!({"masked": true, "bytes": value.as_str().map(str::len)}),
                    "pin" | "name" | "sbSession" | "host" | "language" | "certificateId" | "appName" | "appVersion" | "osName" | "osVersion" | "osArch" => json!("[MASKED]"),
                    _ => { unknown += 1; continue; }
                };
                result.insert(key, safe);
            }
            if unknown > 0 { result.insert("_masked_fields".into(), json!(unknown)); }
            Value::Object(result)
        },
        Value::Array(items) => Value::Array(items.into_iter().map(sanitize).collect()),
        _ => json!("[MASKED]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_credentials_documents_and_unknown_values() {
        let trace = masked_wire(r#"{"operation":"GET_SIGNED_XML","input":{"pin":"synthetic-secret","xml":"PHNlY3JldC8+","certificate":{"alias":"0","name":"Synthetic Person"},"extra":{"password":"another-secret"}}}"#).to_string();
        for secret in ["synthetic-secret", "PHNlY3JldC8+", "Synthetic Person", "another-secret"] { assert!(!trace.contains(secret)); }
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
        assert!(masked_wire(&"sensitive".repeat(10000)).to_string().contains("input_too_large"));
        let deep = format!("{}\"sensitive\"{}", "[".repeat(20), "]".repeat(20));
        assert!(masked_wire(&deep).to_string().contains("budget_exceeded"));
        let array = serde_json::json!({"operation":"GET_CERTIFICATES","payload":{"certificates": vec![serde_json::json!({"name":"sensitive"}); 200]}}).to_string();
        assert!(masked_wire(&array).to_string().contains("budget_exceeded"));
    }

    #[test]
    fn safe_metadata_survives_but_identity_does_not() {
        let value = masked_wire(r#"{"operation":"GET_CERTIFICATES","status":0,"payload":{"certificates":[{"alias":"1","name":"Synthetic Čertificate"}],"terminals":[{"id":1,"name":"Synthetic Token"}],"sbSession":"encoded-secret"}}"#);
        assert_eq!(value["status"], 0);
        assert_eq!(value["payload"]["certificates"][0]["alias"], "1");
        assert_eq!(value["payload"]["terminals"][0]["id"], 1);
        assert!(!value.to_string().contains("Synthetic"));
        assert!(!value.to_string().contains("encoded-secret"));
    }

    #[test]
    fn document_length_is_visible_without_document() {
        let value = masked_wire(r#"{"operation":"GET_SIGNED_XML","payload":{"xml":"PHNlY3JldC8+"}}"#);
        assert_eq!(value["payload"]["xml"]["bytes"], 12);
        assert!(!value.to_string().contains("PHNlY3JldC8+"));
    }
}
