fn main() {
    if let Err(error) = bridge_deck_lib::router_replay::run_cli(std::env::args()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
