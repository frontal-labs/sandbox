//! Runtime operations (V8 custom ops)

use deno_core::{op2, OpState};

#[op2(fast)]
#[bigint]
pub fn op_now(_state: &mut OpState) -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[op2]
#[string]
pub fn op_env_get(_state: &mut OpState, #[string] key: String) -> String {
    std::env::var(&key).unwrap_or_default()
}

#[op2(fast)]
pub fn op_env_set(_state: &mut OpState, #[string] key: String, #[string] value: String) {
    std::env::set_var(key, value);
}

#[op2]
#[string]
pub fn op_fs_read(_state: &mut OpState, #[string] _path: String) -> String {
    String::new()
}

#[op2(fast)]
pub fn op_fs_write(_state: &mut OpState, #[string] _path: String, #[string] _data: String) {
    // Placeholder
}

#[op2]
#[string]
pub fn op_fs_stat(_state: &mut OpState, #[string] _path: String) -> String {
    String::new()
}

#[op2]
#[string]
pub fn op_fs_readdir(_state: &mut OpState, #[string] _path: String) -> String {
    String::new()
}

#[op2(fast)]
pub fn op_spawn(_state: &mut OpState, #[string] _command: String, #[string] _args: String) -> i32 {
    0
}

#[op2]
#[string]
pub fn op_fetch(_state: &mut OpState, #[string] _url: String, #[string] _options: String) -> String {
    String::new()
}