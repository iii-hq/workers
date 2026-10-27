//! Reads a `ReplayInput` on stdin and writes the request it rebuilds on
//! stdout, or, given `{"sizes": SizesInput}`, every step of the run
//! measured (`harness::turn_loop::replay`, the eval loop's level-1 replay).

use std::process::ExitCode;

use harness::turn_loop::replay;

fn main() -> ExitCode {
    let input: serde_json::Value = match serde_json::from_reader(std::io::stdin().lock()) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("harness-replay: invalid input: {error}");
            return ExitCode::FAILURE;
        }
    };
    let written = match input.get("sizes") {
        Some(run) => serde_json::from_value(run.clone())
            .map(|run| serde_json::to_writer(std::io::stdout().lock(), &replay::sizes(&run))),
        None => serde_json::from_value(input)
            .map(|step| serde_json::to_writer(std::io::stdout().lock(), &replay::rebuild(&step))),
    };
    match written {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(error)) | Err(error) => {
            eprintln!("harness-replay: {error}");
            ExitCode::FAILURE
        }
    }
}
