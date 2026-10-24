#![forbid(unsafe_code)]

mod format;
mod install;

use aap_protocol::ipc::socket_path;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `install-widget [DIR]` and `uninstall-widget [DIR]` manage the Quickshell files.
    if let Some(cmd @ ("install-widget" | "uninstall-widget")) = args.first().map(String::as_str) {
        let dir = args.get(1).map(std::path::PathBuf::from).or_else(install::default_dir);
        let Some(dir) = dir else {
            eprintln!("cannot find the Quickshell config directory; pass it as an argument");
            return ExitCode::FAILURE;
        };
        let result = if cmd == "install-widget" { install::install(&dir) } else { install::uninstall(&dir) };
        return match result {
            Ok(msg) => { println!("{msg}"); ExitCode::SUCCESS }
            Err(msg) => { eprintln!("{msg}"); ExitCode::FAILURE }
        };
    }
    // `status --json` prints the raw status line instead of the readable summary.
    let (verb, raw_json) = match args.as_slice() {
        [verb] => (verb, false),
        [verb, flag] if verb == "status" && flag == "--json" => (verb, true),
        _ => {
            eprintln!("usage: hypora-pods-ctl <verb> | status [--json] | install-widget [DIR] | uninstall-widget [DIR]\n  verbs: status, noise:anc, adaptive:40, ...");
            return ExitCode::from(2);
        }
    };
    let Some(path) = socket_path() else {
        eprintln!("XDG_RUNTIME_DIR is not set");
        return ExitCode::FAILURE;
    };
    let Ok(mut stream) = UnixStream::connect(&path) else {
        eprintln!("hypora-pods is not running ({})", path.display());
        return ExitCode::FAILURE;
    };
    if writeln!(stream, "{verb}").is_err() {
        eprintln!("lost connection to the daemon");
        return ExitCode::FAILURE;
    }
    let mut reply = String::new();
    if BufReader::new(stream).read_line(&mut reply).is_err() || reply.is_empty() {
        eprintln!("no reply from the daemon");
        return ExitCode::FAILURE;
    }
    // "ok" is silent; anything else (status JSON or an error) is shown.
    if reply.trim_end() == "ok" {
        // nothing to say
    } else if verb == "status" && !raw_json {
        match serde_json::from_str::<aap_protocol::status::Status>(&reply) {
            Ok(status) => print!("{}", format::render(&status)),
            Err(_) => print!("{reply}"),
        }
    } else {
        print!("{reply}");
    }
    if reply.starts_with("error:") { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
