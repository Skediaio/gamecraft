//! `gamecraft [world.gamecraft | PROJECT] [--stdin] [--quality low|medium|high]
//! [--invert-look]`: the window natively; the same binary built for `wasm32-unknown-unknown` is
//! the page Trunk serves.

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
    let game = match &args.world {
        Some(path) if path.is_dir() => gamecraft::game::Game::from_project(path),
        Some(path) => gamecraft::app::load_world_file(path).map(gamecraft::game::Game::new),
        None => Ok(gamecraft::game::Game::new(gamecraft_engine::example_world())),
    };
    let game = match game {
        Ok(game) => game,
        Err(e) => {
            eprintln!("gamecraft: {e}");
            return ExitCode::from(2);
        }
    };
    match gamecraft::app::run_native(game, &args) {
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
