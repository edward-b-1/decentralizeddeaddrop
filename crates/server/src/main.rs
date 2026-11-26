use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use deaddropd::{AppState, Store, state, tls};

const USAGE: &str = "\
deaddropd: the Decentralized Dead Drop server

USAGE:
    deaddropd [serve] [OPTIONS]
    deaddropd fingerprint [--db FILE] [--tls-cert FILE --tls-key FILE]
    deaddropd operator-token [--db FILE]

    serve            Run the server (the default).
    fingerprint      Print the server's fingerprint, which phones use to recognise it.
    operator-token   Issue a new one-time operator token, replacing any unused one.
                     The next device to register with it becomes an operator.

OPTIONS:
    --db FILE               Database file [default: deaddrop.db]
    --listen ADDRESS:PORT   Where to listen [default: 127.0.0.1:7443]
    --public HOST:PORT      The address phones will use, to print ready-made links.
    --tls-cert FILE         Certificate (PEM) from a certificate authority, with
    --tls-key FILE          its private key (PEM). Without these the server makes
                            its own key and certificate beside the database.
    --no-tls                Plain HTTP. Only for development, or behind a proxy on
                            this machine that provides TLS itself.
";

struct Args {
    command: String,
    db: PathBuf,
    listen: SocketAddr,
    public: Option<String>,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    no_tls: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        command: "serve".into(),
        db: "deaddrop.db".into(),
        listen: SocketAddr::from(([127, 0, 0, 1], 7443)),
        public: None,
        tls_cert: None,
        tls_key: None,
        no_tls: false,
    };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        match word.as_str() {
            "serve" | "fingerprint" | "operator-token" => args.command = word,
            "--db" => args.db = words.next().ok_or("--db needs a file")?.into(),
            "--listen" => {
                let value = words.next().ok_or("--listen needs ADDRESS:PORT")?;
                args.listen = value.parse().map_err(|_| "--listen needs ADDRESS:PORT, e.g. 0.0.0.0:7443")?;
            }
            "--public" => {
                let value = words.next().ok_or("--public needs HOST:PORT")?;
                let port = value.rsplit_once(':').and_then(|(host, port)| (!host.is_empty()).then_some(port));
                if port.is_none_or(|port| port.parse::<u16>().is_err()) {
                    return Err("--public needs HOST:PORT, e.g. 203.0.113.7:7443".into());
                }
                args.public = Some(value);
            }
            "--tls-cert" => args.tls_cert = Some(words.next().ok_or("--tls-cert needs a file")?.into()),
            "--tls-key" => args.tls_key = Some(words.next().ok_or("--tls-key needs a file")?.into()),
            "--no-tls" => args.no_tls = true,
            "-h" | "--help" => return Err(String::new()),
            _ => return Err("unknown argument".into()),
        }
    }
    if args.tls_cert.is_some() != args.tls_key.is_some() {
        return Err("--tls-cert and --tls-key go together".into());
    }
    if args.no_tls && args.tls_cert.is_some() {
        return Err("--no-tls cannot be combined with --tls-cert".into());
    }
    Ok(args)
}

/// The server's TLS identity: the operator's files, or its own beside the database.
fn identity(args: &Args) -> Result<tls::Identity, tls::TlsError> {
    match (&args.tls_cert, &args.tls_key) {
        (Some(certificate), Some(key)) => tls::load(certificate, key),
        _ => {
            let (certificate, key) = (args.db.with_extension("cert.pem"), args.db.with_extension("key.pem"));
            let (identity, created) = tls::load_or_create(&certificate, &key)?;
            if created {
                println!("Made this server's key ({}) and certificate ({}).", key.display(), certificate.display());
                println!("Keep the key file safe and backed up: phones recognise the server by it.");
            }
            Ok(identity)
        }
    }
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

    if args.command == "fingerprint" {
        return match identity(&args) {
            Ok(identity) => {
                println!("{}", deaddrop_tls::display(&identity.fingerprint));
                ExitCode::SUCCESS
            }
            Err(problem) => {
                eprintln!("error: {problem}");
                ExitCode::FAILURE
            }
        };
    }

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

    let identity = if args.no_tls {
        None
    } else {
        match identity(&args) {
            Ok(identity) => Some(identity),
            Err(problem) => {
                eprintln!("error: {problem}");
                return ExitCode::FAILURE;
            }
        }
    };
    let Ok((state, operator_token)) = AppState::open(store).await else {
        return ExitCode::FAILURE;
    };

    if let Some(identity) = &identity {
        println!("Server fingerprint (phones recognise the server by this; it is part of every link):");
        println!("    {}", deaddrop_tls::display(&identity.fingerprint));
    }
    if let Some(token) = &operator_token {
        print_operator_token(token);
    }
    if let (Some(identity), Some(public)) = (&identity, &args.public) {
        let link = format!("deaddrop://{public}/#fp={}", identity.fingerprint);
        match &operator_token {
            Some(token) => println!("Link for the operator's phone (holds the operator token):\n    {link}&t={token}"),
            None => println!("Link to this server (an invite adds its own token):\n    {link}"),
        }
    }
    deaddropd::spawn_sweeper(state.clone(), Duration::from_secs(30));

    let listener = match std::net::TcpListener::bind(args.listen) {
        Ok(listener) => listener,
        Err(_) => {
            eprintln!("error: cannot listen on {}", args.listen);
            return ExitCode::FAILURE;
        }
    };
    let stop = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let served = match &identity {
        Some(identity) => {
            println!("deaddropd listening on https://{}", args.listen);
            deaddropd::serve_tls(listener, state, identity.config.clone(), stop).await
        }
        None => {
            println!("deaddropd listening on http://{} (PLAIN HTTP, no TLS)", args.listen);
            if !args.listen.ip().is_loopback() {
                println!("WARNING: plain HTTP on an address other machines can reach. Sign-in tokens,");
                println!("server passwords and who talks to whom are readable by anyone on the path.");
            }
            match listener.set_nonblocking(true).and_then(|()| tokio::net::TcpListener::from_std(listener)) {
                Ok(listener) => deaddropd::serve(listener, state, stop).await,
                Err(error) => Err(error),
            }
        }
    };
    if served.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn print_operator_token(token: &str) {
    println!("Operator token (shown once; the first device to register with it runs this server):");
    println!("    {token}");
}
