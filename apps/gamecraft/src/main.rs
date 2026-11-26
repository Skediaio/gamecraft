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
    if let (Some(path), Some(pack)) = (&args.world, &args.pack)
        && path.is_dir()
    {
        eprintln!(
            "gamecraft: {} is a project folder, which brings its own assets; --pack {} goes \
             with a world file",
            path.display(),
            pack.display()
        );
        return ExitCode::from(2);
    }
    let game = match &args.world {
        Some(path) if path.is_dir() => gamecraft::game::Game::from_project(path),
        Some(path) => gamecraft::game::Game::from_world_file(path, args.pack.as_deref()),
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
