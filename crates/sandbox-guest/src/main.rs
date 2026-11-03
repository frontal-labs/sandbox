//! The guest agent as a process: `sandbox-guest-agent --unix PATH` or
//! `sandbox-guest-agent --tcp ADDR`. The guest starts it as its init service.
//! Its files live in memory, as in the agent's own tests.

use sandbox_cas::MemStore;
use sandbox_guest::GuestAgent;
use sandbox_types::ContentHash;
use sandbox_vfs::Vfs;
use std::process::ExitCode;
use std::sync::Arc;

const USAGE: &str = "usage: sandbox-guest-agent (--unix PATH | --tcp ADDR)";

fn empty_vfs() -> Arc<Vfs> {
    Arc::new(Vfs::new(ContentHash::nil(), Arc::new(MemStore::new())))
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bound = match args.as_slice() {
        [flag, path] if flag == "--unix" => GuestAgent::new(path, empty_vfs()).await,
        [flag, addr] if flag == "--tcp" => GuestAgent::new_tcp(addr, empty_vfs()).await,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let agent = match bound {
        Ok(agent) => agent,
        Err(error) => {
            eprintln!("sandbox-guest-agent: {error}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("sandbox-guest-agent: serving the guest protocol");
    match agent.run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sandbox-guest-agent: {error}");
            ExitCode::FAILURE
        }
    }
}
