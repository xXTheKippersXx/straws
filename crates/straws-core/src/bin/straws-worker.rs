//! Standalone nec2c worker: `straws-worker -i deck.nec -o deck.out`, or a
//! persistent one with `--nec2c-server` (see `straws_core::worker`).
//! Used by the test suite and handy for running nec2c directly.

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(straws_core::worker::SERVER_FLAG.as_ref()) {
        straws_core::worker::serve();
        return;
    }
    let code = straws_core::worker::run_nec2c(std::env::args_os().skip(1).collect());
    std::process::exit(code);
}
