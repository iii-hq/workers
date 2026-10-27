//! Reads a `ReplayInput` on stdin and writes the request it rebuilds on
//! stdout (`harness::turn_loop::replay`, the eval loop's level-1 replay).

use std::process::ExitCode;

fn main() -> ExitCode {
    let input: harness::turn_loop::replay::ReplayInput =
        match serde_json::from_reader(std::io::stdin().lock()) {
            Ok(input) => input,
            Err(error) => {
                eprintln!("harness-replay: invalid input: {error}");
                return ExitCode::FAILURE;
            }
        };
    let request = harness::turn_loop::replay::rebuild(&input);
    match serde_json::to_writer(std::io::stdout().lock(), &request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("harness-replay: {error}");
            ExitCode::FAILURE
        }
    }
}
