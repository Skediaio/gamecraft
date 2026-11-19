//! `gamecraft [world.gamecraft] [--stdin] [--quality low|medium|high] [--invert-look]`: the window natively;
//! the same binary built for `wasm32-unknown-unknown` is the page Trunk serves.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;
    let args = match gamecraft::args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("gamecraft: {e}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        println!("{}", gamecraft::args::USAGE);
        return ExitCode::SUCCESS;
    }
    let world = match &args.world {
        Some(path) => match gamecraft::app::load_world_file(path) {
            Ok(world) => world,
            Err(e) => {
                eprintln!("gamecraft: {e}");
                return ExitCode::from(2);
            }
        },
        None => gamecraft_engine::example_world(),
    };
    match gamecraft::app::run_native(world, &args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gamecraft: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    gamecraft::app::run_web();
}
