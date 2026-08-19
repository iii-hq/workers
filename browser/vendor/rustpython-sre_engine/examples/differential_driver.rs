use std::io::{self, BufRead, Write};

use rustpython_sre_engine::compiler;
use rustpython_sre_engine::{Request, SearchIter, State, StrDrive};

fn decode_hex(value: &str) -> Result<String, String> {
    if !value.len().is_multiple_of(2) {
        return Err("odd hex input".to_string());
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn encode_hex(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn findall(pattern: &str, text: &str, ignore_case: bool) -> Result<Vec<Vec<String>>, String> {
    let compiled = compiler::compile(pattern, ignore_case).map_err(|error| error.to_string())?;
    let req = Request::new(text, 0, text.count(), &compiled.codes, false);
    let mut iter = SearchIter {
        req,
        state: State::default(),
    };
    let chars: Vec<char> = text.chars().collect();
    let mut matches = Vec::new();
    while iter.next().is_some() {
        let mut groups = Vec::new();
        if compiled.groups == 0 {
            groups.push(
                chars[iter.state.start..iter.state.cursor.position]
                    .iter()
                    .collect(),
            );
        } else {
            for group in 0..compiled.groups {
                let (start, end) = iter.state.marks.get(group);
                groups.push(match (start.into_option(), end.into_option()) {
                    (Some(start), Some(end)) => chars[start..end].iter().collect(),
                    _ => String::new(),
                });
            }
        }
        matches.push(groups);
    }
    Ok(matches)
}

fn encode_matches(matches: &[Vec<String>]) -> String {
    let mut encoded = matches.len().to_string();
    for groups in matches {
        encoded.push('\t');
        encoded.push_str(&groups.len().to_string());
        for group in groups {
            encoded.push(':');
            encoded.push_str(&encode_hex(group));
        }
    }
    encoded
}

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        let mut fields = line.split('\t');
        let ignore_case = fields.next() == Some("1");
        let pattern = fields.next().and_then(|value| decode_hex(value).ok());
        let text = fields.next().and_then(|value| decode_hex(value).ok());
        let result = match (pattern, text) {
            (Some(pattern), Some(text)) => match findall(&pattern, &text, ignore_case) {
                Ok(matches) => format!("OK\t{}", encode_matches(&matches)),
                Err(error) => format!("ERR\t{}", encode_hex(&error)),
            },
            _ => format!("ERR\t{}", encode_hex("invalid differential input")),
        };
        writeln!(stdout, "{result}").expect("stdout");
        stdout.flush().expect("stdout flush");
    }
}
