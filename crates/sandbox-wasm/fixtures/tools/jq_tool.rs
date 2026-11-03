// jq_tool - Simple JSON processor (like jq)
// Supports basic filtering and transformation

use std::io::{self, Read};
use serde_json::Value;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: jq_tool <filter> [file]");
        eprintln!("Filters: . (identity), .field, .[index], .[] (iterate), length, keys");
        std::process::exit(1);
    }

    let filter = &args[1];
    let input = if args.len() > 2 {
        std::fs::read_to_string(&args[2])?
    } else {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        buf
    };

    let value: Value = serde_json::from_str(&input)?;
    let result = apply_filter(&value, filter)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn apply_filter(value: &Value, filter: &str) -> anyhow::Result<Value> {
    match filter {
        "." => Ok(value.clone()),
        f if f.starts_with(".[]") => {
            // Iterate over array
            if let Value::Array(arr) = value {
                Ok(Value::Array(arr.iter().map(|v| apply_filter(v, &f[3..])).collect::<anyhow::Result<_>>()?))
            } else {
                anyhow::bail!("Cannot iterate non-array");
            }
        }
        f if f.starts_with(".") && f.len() > 1 => {
            // Field access: .field or .field.subfield
            let path = &f[1..];
            let mut current = value;
            for part in path.split('.').filter(|part| !part.is_empty()) {
                if let Some(idx) = part.strip_suffix("]").and_then(|p| p.strip_prefix('[')).and_then(|i| i.parse::<usize>().ok()) {
                    current = current.get(idx).ok_or_else(|| anyhow::anyhow!("Index out of bounds"))?;
                } else {
                    current = current.get(part).ok_or_else(|| anyhow::anyhow!("Field '{}' not found", part))?;
                }
            }
            Ok(current.clone())
        }
        "length" => {
            match value {
                Value::Array(arr) => Ok(Value::Number(arr.len().into())),
                Value::Object(obj) => Ok(Value::Number(obj.len().into())),
                Value::String(s) => Ok(Value::Number(s.len().into())),
                _ => Ok(Value::Number(1.into())),
            }
        }
        "keys" => {
            if let Value::Object(obj) = value {
                Ok(Value::Array(obj.keys().map(|k| Value::String(k.clone())).collect()))
            } else {
                anyhow::bail!("keys only works on objects");
            }
        }
        _ => anyhow::bail!("Unknown filter: {}", filter),
    }
}