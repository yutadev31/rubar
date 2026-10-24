use clap::Parser;
use cpst::{copy, run_daemon};

#[derive(Parser)]
#[command(name = "cpy", version, about = "Copy stdin to the system clipboard")]
struct Cpy;

fn main() {
    let mut arguments = std::env::args();
    arguments.next();
    if arguments.next().as_deref() == Some("__internal_daemonize") {
        if let Err(error) = run_daemon() {
            eprintln!("cpy: {error}");
            std::process::exit(1);
        }
        return;
    }

    Cpy::parse();
    if let Err(error) = copy() {
        eprintln!("cpy: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_primary_options() {
        assert!(Cpy::try_parse_from(["cpy", "-p"]).is_err());
        assert!(Cpy::try_parse_from(["cpy", "--primary"]).is_err());
    }
}
