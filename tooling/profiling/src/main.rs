mod command;
mod instruments;
mod workspace;

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let Some(command) = args.get(1).map(String::as_str) else {
        workspace::fatal("missing profiling command: bench or service-smoke");
    };

    match command {
        "bench" => instruments::bench(&args[2..]),
        "service-smoke" => instruments::service_smoke(&args[2..]),
        "help" | "--help" | "-h" => print_usage(),
        _ => workspace::fatal("unknown profiling command"),
    }
}

fn print_usage() {
    println!(
        "usage:
  ophelia-profiling bench <bench> <filter> [--seconds N]
  ophelia-profiling service-smoke [--seconds N]"
    );
}
