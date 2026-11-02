// echo_tool - Simple tool that echoes its arguments
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() <= 1 {
        eprintln!("Usage: echo_tool <args...>");
        std::process::exit(1);
    }
    println!("{}", args[1..].join(" "));
}