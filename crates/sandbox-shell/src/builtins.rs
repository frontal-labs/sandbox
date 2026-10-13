//! Shell builtins

use sandbox_types::SandboxError;
use std::collections::HashMap;

pub struct Builtins {
    commands: HashMap<String, BuiltinFn>,
}

type BuiltinFn = Box<dyn Fn(&[String]) -> Result<i32, SandboxError> + Send + Sync>;

impl Builtins {
    pub fn new() -> Self {
        let mut builtins = Self {
            commands: HashMap::new(),
        };
        
        builtins.register("echo", |args| {
            println!("{}", args.join(" "));
            Ok(0)
        });
        
        builtins.register("pwd", |_args| {
            println!("/work");
            Ok(0)
        });
        
        builtins.register("cd", |args| {
            if args.is_empty() {
                eprintln!("cd: missing argument");
                return Ok(1);
            }
            println!("cd: {} (not implemented)", args[0]);
            Ok(0)
        });
        
        builtins.register("true", |_| Ok(0));
        builtins.register("false", |_| Ok(1));
        
        builtins
    }
    
    fn register<F>(&mut self, name: &str, f: F)
    where
        F: Fn(&[String]) -> Result<i32, SandboxError> + Send + Sync + 'static,
    {
        self.commands.insert(name.to_string(), Box::new(f));
    }
    
    pub fn get(&self, name: &str) -> Option<&BuiltinFn> {
        self.commands.get(name)
    }
    
    pub fn is_builtin(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }
}

impl Default for Builtins {
    fn default() -> Self {
        Self::new()
    }
}