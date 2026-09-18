fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: gsm-ctrlc-helper <pid> <creation-filetime>");
        return std::process::ExitCode::from(2);
    }
    let (Ok(pid), Ok(created)) = (args[0].parse::<u32>(), args[1].parse::<u64>()) else {
        return std::process::ExitCode::from(2);
    };
    match gsm_infra::local::process::signal(pid, created) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}
