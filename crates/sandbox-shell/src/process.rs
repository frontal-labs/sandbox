//! Shell process management

use std::collections::HashMap;
use std::hash::Hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VirtualPid(u32);

impl VirtualPid {
    pub fn new() -> Self {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        Self(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

pub struct Process {
    pub pid: VirtualPid,
    pub command: String,
    pub stdin: Option<Vec<u8>>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
}

impl Process {
    pub fn new(command: String) -> Self {
        Self {
            pid: VirtualPid::new(),
            command,
            stdin: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: None,
        }
    }
}

pub struct ProcessTable {
    processes: HashMap<VirtualPid, Process>,
}

impl ProcessTable {
    pub fn new() -> Self {
        Self { processes: HashMap::new() }
    }

    pub fn add(&mut self, process: Process) -> VirtualPid {
        let pid = process.pid;
        self.processes.insert(pid, process);
        pid
    }

    pub fn get(&self, pid: VirtualPid) -> Option<&Process> {
        self.processes.get(&pid)
    }

    pub fn remove(&mut self, pid: VirtualPid) -> Option<Process> {
        self.processes.remove(&pid)
    }
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self::new()
    }
}