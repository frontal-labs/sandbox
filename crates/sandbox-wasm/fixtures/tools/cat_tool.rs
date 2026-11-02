// cat_tool - Reads stdin and writes to stdout
use std::io::{self, Read, Write};

fn main() -> io::Result<()> {
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut buffer = Vec::new();
    stdin.read_to_end(&mut buffer)?;
    stdout.write_all(&buffer)?;
    stdout.flush()?;
    Ok(())
}