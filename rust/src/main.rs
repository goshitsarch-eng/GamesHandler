use serde_json::json;

fn main() {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty()
        && let Ok(executable) = std::env::current_exe()
    {
        let launch = executable.parent().unwrap().join("gamehandler-launch.json");
        if launch.is_file() {
            let request = std::fs::read(launch).unwrap_or_default();
            let result = gamehandler_desktop::request_json(&request);
            if result["ok"] != true {
                eprintln!("{}", result["error"]["message"]);
                std::process::exit(1);
            }
            return;
        }
    }
    let mut config = gamehandler_core::paths::config_home();
    let mut data = gamehandler_core::paths::data_home();
    while args
        .first()
        .is_some_and(|a| a == "--config-dir" || a == "--data-dir")
    {
        if args.len() < 2 || !std::path::Path::new(&args[1]).is_absolute() {
            eprintln!("Directory options require an absolute path; run --help");
            std::process::exit(2);
        }
        let value = std::path::PathBuf::from(&args[1]);
        if args[0] == "--config-dir" {
            config = value;
        } else {
            data = value;
        }
        args.drain(..2);
    }
    if args.is_empty() || args[0] == "--help" {
        println!(
            "GameHandler {}\nUsage: gamehandler-cli [--config-dir PATH] [--data-dir PATH] --list | --launch GAME_ID | --version",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if args[0] == "--version" {
        println!("GameHandler {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let context = json!({"config":config,"data":data});
    let request = match args[0].as_str() {
        "--list" => {
            json!({"context":context,"operation":"snapshot","arguments":{"query":"","category":"All","sort":"name"}})
        }
        "--launch" if args.len() == 2 => {
            json!({"context":context,"operation":"launch","arguments":{"id":args[1]}})
        }
        _ => {
            eprintln!("Unknown command or missing game id; run --help");
            std::process::exit(2);
        }
    };
    let response = gamehandler_desktop::request_json(request.to_string().as_bytes());
    if response["ok"] != true {
        eprintln!(
            "{}",
            response["error"]["message"]
                .as_str()
                .unwrap_or("Operation failed")
        );
        std::process::exit(1);
    }
    if args[0] == "--list"
        && let Some(games) = response["data"]["games"].as_array()
    {
        for g in games {
            println!(
                "{}\t{}",
                g["id"].as_str().unwrap_or(""),
                g["name"].as_str().unwrap_or("")
            );
        }
    }
}
