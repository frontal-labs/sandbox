// env_tool - Prints environment variables as JSON
use std::env;
use std::collections::BTreeMap;

fn main() {
    let env_vars: BTreeMap<_, _> = env::vars().collect();
    let json = serde_json::to_string_pretty(&env_vars).unwrap();
    println!("{}", json);
}