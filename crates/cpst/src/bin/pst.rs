use clap::Parser;
use cpst::paste;

#[derive(Parser)]
#[command(name = "pst", version, about = "Paste the system clipboard to stdout")]
struct Pst;

fn main() {
    Pst::parse();
    if let Err(error) = paste() {
        eprintln!("pst: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_primary_options() {
        assert!(Pst::try_parse_from(["pst", "-p"]).is_err());
        assert!(Pst::try_parse_from(["pst", "--primary"]).is_err());
    }
}
