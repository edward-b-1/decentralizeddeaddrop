use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use deaddropd::{AppState, Store, state};

const USAGE: &str = "\
deaddropd: the Decentralized Dead Drop server

USAGE:
    deaddropd [serve] [--db FILE] [--listen ADDRESS:PORT]
    deaddropd operator-token [--db FILE]

    serve            Run the server (the default).
    operator-token   Issue a new one-time operator token, replacing any unused one.
                     The next device to register with it becomes an operator.

OPTIONS:
    --db FILE               Database file [default: deaddrop.db]
    --listen ADDRESS:PORT   Where to listen [default: 127.0.0.1:8080]
";

struct Args {
    command: String,
    db: PathBuf,
    listen: SocketAddr,
}

fn parse_args() -> Result<Args, String> {
    let mut args =
        Args { command: "serve".into(), db: "deaddrop.db".into(), listen: SocketAddr::from(([127, 0, 0, 1], 8080)) };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        match word.as_str() {
            "serve" | "operator-token" => args.command = word,
            "--db" => args.db = words.next().ok_or("--db needs a file")?.into(),
            "--listen" => {
                let value = words.next().ok_or("--listen needs ADDRESS:PORT")?;
                args.listen = value.parse().map_err(|_| "--listen needs ADDRESS:PORT, e.g. 127.0.0.1:8080")?;
            }
            "-h" | "--help" => return Err(String::new()),
            _ => return Err("unknown argument".into()),
        }
    }
    Ok(args)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(problem) => {
            if !problem.is_empty() {
                eprintln!("error: {problem}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let store = match Store::open(&args.db) {
        Ok(store) => store,
        Err(_) => {
            eprintln!("error: cannot open the database file");
            return ExitCode::FAILURE;
        }
    };

    if args.command == "operator-token" {
        return match store.run(|db| Ok(state::new_operator_token(db)?)).await {
            Ok(token) => {
                print_operator_token(&token);
                ExitCode::SUCCESS
            }
            Err(_) => ExitCode::FAILURE,
        };
    }

    let Ok((state, operator_token)) = AppState::open(store).await else {
        return ExitCode::FAILURE;
    };
    if let Some(token) = operator_token {
        print_operator_token(&token);
    }
    deaddropd::spawn_sweeper(state.clone(), Duration::from_secs(30));

    let listener = match tokio::net::TcpListener::bind(args.listen).await {
        Ok(listener) => listener,
        Err(_) => {
            eprintln!("error: cannot listen on {}", args.listen);
            return ExitCode::FAILURE;
        }
    };
    println!("deaddropd listening on http://{} (plain HTTP: put TLS in front before exposing it)", args.listen);
    let served = deaddropd::serve(listener, state, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await;
    if served.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn print_operator_token(token: &str) {
    println!("Operator token (shown once; the first device to register with it runs this server):");
    println!("    {token}");
}
