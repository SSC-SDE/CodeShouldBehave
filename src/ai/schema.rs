use serde_json::{Value, json};

pub(crate) fn response_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["headline", "changes", "questions", "unknowns"],
        "properties": {
            "headline": {"type": "string", "maxLength": 200},
            "changes": {
                "type": "array",
                "maxItems": 10,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["claim", "side", "evidence_ids"],
                    "properties": {
                        "claim": {"type": "string", "maxLength": 500},
                        "side": {"type": "string", "enum": ["baseline", "proposal", "both"]},
                        "evidence_ids": {
                            "type": "array",
                            "maxItems": 16,
                            "items": {"type": "integer", "minimum": 0}
                        }
                    }
                }
            },
            "questions": {
                "type": "array",
                "maxItems": 10,
                "items": {"type": "string", "maxLength": 300}
            },
            "unknowns": {
                "type": "array",
                "maxItems": 10,
                "items": {"type": "string", "maxLength": 300}
            }
        }
    })
}
