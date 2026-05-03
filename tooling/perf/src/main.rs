mod command;
mod criterion;
mod model;
mod scenario;
mod table;
mod workspace;

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let Some(first) = args.get(1) else {
        workspace::fatal("missing test binary or command");
    };

    match first.as_str() {
        "compare" => scenario::compare_runs(&args[2..]),
        "criterion" => criterion::command(&args[2..]),
        "help" | "--help" | "-h" => print_usage(),
        _ => scenario::run_perf_tests(first, &args[2..]),
    }
}

fn print_usage() {
    println!(
        "usage:
  ophelia-perf <test-binary> [runner-options]
  ophelia-perf compare <new-run> <old-run>
  ophelia-perf criterion baseline <name>
  ophelia-perf criterion compare <name>"
    );
}
