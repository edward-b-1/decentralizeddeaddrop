//! A command-line Dead Drop client, for testing servers and for scripts.
//!
//! It is the same engine the phone app uses. The one difference matters: here the
//! device's keys sit in the state file in the clear, where a phone keeps them in secure
//! hardware. Treat the state file like a password.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use deaddrop_client::{Account, ClientError, Event, Link, Wake, WayIn};
use deaddrop_core::encoding::{b64, unb64};
use deaddrop_core::soft::SoftDevice;
use serde_json::{Value, json};

const USAGE: &str = "\
deaddrop: a command-line Decentralized Dead Drop client

USAGE:
    deaddrop --state FILE <command>

    join LINK [--password PASSWORD]      Register on the server a link points to
    status                               This device, its server and its channels
    server-invite                        Make a link that lets a device onto the server (operator)
    server-password [PASSWORD]           Set the server password, or remove it (operator)
    new-channel NAME --as MY-NAME [--max-age SECONDS]
    invite CHANNEL                       Make a link that lets someone ask to join
    ask LINK --as MY-NAME                Ask to join the channel a link invites to
    requests CHANNEL                     Who is asking to join (owner)
    admit CHANNEL DEVICE                 Let a device in (owner)
    remove CHANNEL DEVICE                Remove a member (owner)
    leave CHANNEL
    leave-server                         Delete this device from the server, with the channels it owns
    send CHANNEL TEXT...
    receive                              Collect and print what is waiting
    listen                               Stay connected and print messages as they arrive

    CHANNEL is a channel's ID or its name. The state file holds this device's keys
    unprotected: keep it private.
";

struct State {
    path: PathBuf,
    keys: SoftDevice,
    account: Account,
}

fn fail(message: impl std::fmt::Display) -> String {
    message.to_string()
}

/// Writes the state file by way of a temporary file, so a crash never leaves half of one.
fn store(path: &Path, keys: &SoftDevice, account: &[u8]) -> Result<(), String> {
    let text = json!({ "keys": b64(&keys.to_bytes()), "account": b64(account) }).to_string();
    let temporary = path.with_extension("writing");
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|_| fail("cannot write the state file"))
}

impl State {
    fn load(path: &Path) -> Result<State, String> {
        let text = std::fs::read_to_string(path).map_err(|_| fail("no state file there yet: use `join` first"))?;
        let saved: Value = serde_json::from_str(&text).map_err(|_| fail("the state file is damaged"))?;
        let part = |name: &str| saved[name].as_str().and_then(unb64).ok_or(fail("the state file is damaged"));
        Ok(State {
            path: path.to_owned(),
            keys: SoftDevice::from_bytes(&part("keys")?).ok_or(fail("the state file is damaged"))?,
            account: Account::from_bytes(&part("account")?).map_err(fail)?,
        })
    }

    fn save(&self) -> Result<(), String> {
        store(&self.path, &self.keys, &self.account.to_bytes())
    }

    /// Finds a channel by ID or by name.
    fn channel(&self, wanted: &str) -> Result<String, String> {
        let matches: Vec<&str> = self
            .account
            .channels()
            .filter(|channel| channel.id == wanted || channel.name == wanted)
            .map(|channel| channel.id.as_str())
            .collect();
        match matches.as_slice() {
            [one] => Ok((*one).to_owned()),
            [] => Err(fail(format!("no channel called {wanted} (try `receive` to catch up)"))),
            _ => Err(fail(format!("more than one channel is called {wanted}: use its ID"))),
        }
    }

    fn sync(&mut self) -> Result<Vec<Event>, ClientError> {
        let (path, keys) = (&self.path, &self.keys);
        self.account.sync(keys, &mut |bytes| {
            let _ = store(path, keys, bytes);
        })
    }

    fn send(&mut self, channel: &str, text: &str) -> Result<bool, ClientError> {
        let (path, keys) = (&self.path, &self.keys);
        self.account.send(keys, channel, text, &mut |bytes| {
            let _ = store(path, keys, bytes);
        })
    }

    fn print(&self, events: &[Event]) {
        for event in events {
            let name = |id: &str| self.account.channel(id).map_or(id.to_owned(), |channel| channel.name.clone());
            match event {
                Event::Message { channel, sender, sender_name, text, .. } => {
                    println!("[{}] {}: {text}", name(channel), sender_name.as_deref().unwrap_or(sender));
                }
                Event::Unreadable { channel, sender } => {
                    println!("[{}] (a message from {sender} could not be read)", name(channel));
                }
                Event::ChannelsChanged => println!("(your channels have changed: see `status`)"),
            }
        }
    }
}

/// Takes `--name VALUE` out of the arguments, if it is there.
fn option(args: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    let Some(at) = args.iter().position(|arg| arg == name) else { return Ok(None) };
    if at + 1 >= args.len() {
        return Err(fail(format!("{name} needs a value")));
    }
    args.remove(at);
    Ok(Some(args.remove(at)))
}

fn status(account: &Account) {
    println!("Device {} on {}", account.device(), account.address().authority());
    println!("Server fingerprint {}", deaddrop_tls::display(&account.address().fingerprint));
    if account.operator() {
        println!("This device runs the server.");
    }
    if !account.approved() {
        println!("Waiting for the operator to approve this device.");
    }
    for channel in account.channels() {
        let role = if channel.owner == account.device() { ", owned by this device" } else { "" };
        println!("\n{} ({}{role})", channel.name, channel.id);
        println!("  maximum message age: {} seconds", channel.max_age);
        for member in &channel.members {
            let gone = if channel.left.contains(&member.device) { " (has left)" } else { "" };
            println!("  {} {}{gone}", member.device, member.name);
        }
    }
    for channel in account.asked() {
        println!("\nAsked to join {channel}; waiting for its owner.");
    }
    if account.unsent() > 0 {
        println!("\n{} sealed message(s) waiting to be sent.", account.unsent());
    }
}

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let path = PathBuf::from(option(&mut args, "--state")?.ok_or(fail("--state FILE is needed"))?);
    let my_name = option(&mut args, "--as")?.ok_or(fail("--as MY-NAME is needed"));
    let password = option(&mut args, "--password")?;
    let max_age = option(&mut args, "--max-age")?;
    let mut words = args.into_iter();
    let command = words.next().ok_or(fail("which command?"))?;
    let mut next = |what: &str| words.next().ok_or(fail(format!("that command needs {what}")));

    if command == "join" {
        if path.exists() {
            return Err(fail("that state file already holds a device: use a new file for a new server"));
        }
        let link = Link::parse(&next("a link")?).ok_or(fail(ClientError::BadLink))?;
        let keys = SoftDevice::new();
        let way_in = WayIn { token: link.server_token.clone(), password };
        let account = Account::join(&link.server, &way_in, &keys).map_err(fail)?;
        store(&path, &keys, &account.to_bytes())?;
        println!("Registered as device {} on {}.", account.device(), link.server.authority());
        if account.operator() {
            println!("This device runs the server.");
        }
        if !account.approved() {
            println!("Waiting for the operator to approve this device.");
        }
        return Ok(());
    }

    let mut state = State::load(&path)?;
    // A second handle on the same keys, so the account can be borrowed alongside them.
    let keys = SoftDevice::from_bytes(&state.keys.to_bytes()).expect("just loaded");
    match command.as_str() {
        "status" => status(&state.account),
        "server-invite" => println!("{}", state.account.invite_to_server(&keys, 1, 86_400).map_err(fail)?),
        "server-password" => {
            let password = words.next();
            state.account.set_server_password(&keys, password.as_deref()).map_err(fail)?;
            println!(
                "{}",
                if password.is_some() { "The server password is set." } else { "The server has no password now." }
            );
        }
        "new-channel" => {
            let name = next("a name")?;
            let max_age = match max_age {
                Some(text) => text.parse().map_err(|_| fail("--max-age is a number of seconds"))?,
                None => 86_400,
            };
            let id = state.account.create_channel(&keys, &name, &my_name?, max_age).map_err(fail)?;
            println!("Created {name} ({id}).");
        }
        "invite" => {
            let channel = state.channel(&next("a channel")?)?;
            println!("{}", state.account.invite_to_channel(&keys, &channel, 1, 86_400).map_err(fail)?);
        }
        "ask" => {
            let link = Link::parse(&next("a link")?).ok_or(fail(ClientError::BadLink))?;
            state.account.ask_to_join(&keys, &link, &my_name?).map_err(fail)?;
            println!("Asked. The channel appears once its owner lets you in.");
        }
        "requests" => {
            let channel = state.channel(&next("a channel")?)?;
            for request in state.account.join_requests(&keys, &channel).map_err(fail)? {
                println!("{} {}", request.device, request.name);
            }
        }
        "admit" => {
            let channel = state.channel(&next("a channel")?)?;
            state.account.admit(&keys, &channel, &next("a device ID")?).map_err(fail)?;
        }
        "remove" => {
            let channel = state.channel(&next("a channel")?)?;
            state.account.remove(&keys, &channel, &next("a device ID")?).map_err(fail)?;
        }
        "leave" => {
            let channel = state.channel(&next("a channel")?)?;
            state.account.leave(&keys, &channel).map_err(fail)?;
        }
        "leave-server" => {
            state.account.leave_server(&keys).map_err(fail)?;
            println!("This device is deleted from the server. The state file is of no further use.");
        }
        "send" => {
            let channel = state.channel(&next("a channel")?)?;
            let text = words.collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                return Err(fail("send needs some text"));
            }
            if !state.send(&channel, &text).map_err(fail)? {
                println!("The server cannot be reached. The message is sealed and will go with the next `receive`.");
            }
        }
        "receive" => {
            let events = state.sync().map_err(fail)?;
            state.print(&events);
        }
        "listen" => loop {
            // Collect first, then wait for the server to say there is more. If the
            // connection drops, try again shortly: nothing is lost in between.
            match state.sync() {
                Ok(events) => state.print(&events),
                Err(error) => eprintln!("{error}"),
            }
            match state.account.open_live(&keys) {
                Ok(mut live) => while let Ok(Wake::Quiet) = live.wait(Duration::from_secs(60)) {},
                Err(_) => std::thread::sleep(Duration::from_secs(5)),
            }
        },
        _ => return Err(fail(format!("unknown command {command}"))),
    }
    state.save()
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("error: {problem}");
            ExitCode::FAILURE
        }
    }
}
