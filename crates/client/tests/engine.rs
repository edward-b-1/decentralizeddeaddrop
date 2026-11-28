//! The client engine against a real server over real TLS: what the app will do,
//! start to finish, through the same code the app will call.

use std::time::Duration;

use deaddrop_client::{Account, ClientError, Event, Link, ServerAddress, Wake, WayIn, WaysIn, Why};
use deaddrop_core::encoding::random_id;
use deaddrop_core::records::{Closed, Leave};
use deaddrop_core::soft::SoftDevice;
use deaddropd::{AppState, Store, tls};

struct TestServer {
    address: ServerAddress,
    operator_token: String,
    state: AppState,
    runtime: tokio::runtime::Runtime,
    files: std::path::PathBuf,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.files);
    }
}

impl TestServer {
    fn start() -> TestServer {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (state, operator_token) = runtime.block_on(AppState::open(Store::in_memory().unwrap())).unwrap();
        let files = std::env::temp_dir().join(format!("deaddrop-client-test-{}", random_id()));
        std::fs::create_dir(&files).unwrap();
        let (identity, _) = tls::load_or_create(&files.join("cert.pem"), &files.join("key.pem")).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (serving, config) = (state.clone(), identity.config.clone());
        runtime.spawn(async move { deaddropd::serve_tls(listener, serving, config, std::future::pending()).await });
        TestServer {
            address: ServerAddress { host: "127.0.0.1".into(), port, fingerprint: identity.fingerprint },
            operator_token: operator_token.unwrap(),
            state,
            runtime,
            files,
        }
    }

    /// Changes the server's database behind its back: what a server that lies, or one
    /// that has been broken into, could do.
    fn tamper(&self, sql: String) {
        let work = self.state.store.run(move |db| Ok(db.execute_batch(&sql)?));
        self.runtime.block_on(work).unwrap();
    }

    /// How many rows a table on the server holds.
    fn rows(&self, table: &'static str) -> i64 {
        let work = self
            .state
            .store
            .run(move |db| Ok(db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))?));
        self.runtime.block_on(work).unwrap()
    }
}

/// A person: their device keys, their account, and what their app last stored.
struct Person {
    keys: SoftDevice,
    account: Account,
    stored: Vec<u8>,
}

impl Person {
    /// The first device on a server, which registers with the operator token.
    fn operator(server: &TestServer) -> Person {
        let link =
            Link { server: server.address.clone(), server_token: Some(server.operator_token.clone()), channel: None };
        Person::from_link(&link.to_string())
    }

    /// Joins a server from a link, exactly as pasted or scanned.
    fn from_link(link: &str) -> Person {
        let link = Link::parse(link).expect("a well-formed link");
        let keys = SoftDevice::new();
        let way_in = WayIn { token: link.server_token.clone(), password: None };
        let account = Account::join(&link.server, &way_in, &keys).unwrap();
        let stored = account.to_bytes();
        Person { keys, account, stored }
    }

    /// A new person let onto the server by its operator.
    fn invited_by(operator: &mut Person) -> Person {
        let link = operator.account.invite_to_server(&operator.keys, 1, 3600).unwrap();
        Person::from_link(&link.to_string())
    }

    /// Collects, and then says the messages are stored, as an app does once it has
    /// written them to its own history.
    fn sync(&mut self) -> Vec<Event> {
        self.try_sync().unwrap()
    }

    fn try_sync(&mut self) -> Result<Vec<Event>, ClientError> {
        let events = self.collect()?;
        self.account.stored(&Person::ids(&events));
        self.stored = self.account.to_bytes();
        Ok(events)
    }

    /// Collects without saying the messages are stored: an app that has not yet
    /// written them down, or that is closed before it can.
    fn collect(&mut self) -> Result<Vec<Event>, ClientError> {
        let stored = &mut self.stored;
        self.account.sync(&self.keys, &mut |bytes| *stored = bytes.to_vec())
    }

    /// Hands the engine a message and has it try the server at once, as an app does.
    /// True if nothing is left waiting to go.
    fn send(&mut self, channel: &str, text: &str) -> Result<bool, ClientError> {
        let stored = &mut self.stored;
        let mut save = |bytes: &[u8]| *stored = bytes.to_vec();
        self.account.send(channel, text, &mut save)?;
        self.account.flush(&self.keys, &mut save)
    }

    /// Takes the signal away, or brings it back: the app as it was last stored, pointed
    /// at a dead port or at the server again.
    fn signal(&mut self, server: &TestServer, on: bool) {
        let (real, dead) = (format!("\"port\":{}", server.address.port), "\"port\":9".to_owned());
        let (from, to) = if on { (dead, real) } else { (real, dead) };
        let stored = String::from_utf8(self.stored.clone()).unwrap();
        assert!(stored.contains(&from));
        self.stored = stored.replace(&from, &to).into_bytes();
        self.restart();
    }

    /// The IDs of the messages among some events.
    fn ids(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Message { id, .. } | Event::Unreadable { id, .. } => Some(id.clone()),
                Event::ChannelsChanged => None,
            })
            .collect()
    }

    /// The texts among some events, as "name: text".
    fn texts(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Message { sender_name, text, .. } => {
                    Some(format!("{}: {text}", sender_name.as_deref().unwrap_or("?")))
                }
                _ => None,
            })
            .collect()
    }

    /// As if the app had been closed and opened again: everything comes from storage.
    fn restart(&mut self) {
        self.keys = SoftDevice::from_bytes(&self.keys.to_bytes()).unwrap();
        self.account = Account::from_bytes(&self.stored).unwrap();
    }
}

/// `owner` creates a channel and lets each of `others` in, going through an invite link
/// each time. Everyone is synced at the end. Returns the channel's ID.
fn channel(owner: &mut Person, owner_name: &str, others: &mut [(&mut Person, &str)]) -> String {
    let id = owner.account.create_channel(&owner.keys, "Family", owner_name, 3600).unwrap();
    for (person, name) in others.iter_mut() {
        admit(owner, &id, person, name);
    }
    owner.sync();
    for (person, _) in others.iter_mut() {
        person.sync();
    }
    id
}

fn admit(owner: &mut Person, channel: &str, person: &mut Person, name: &str) {
    let link = owner.account.invite_to_channel(&owner.keys, channel, 1, 3600).unwrap().to_string();
    person.account.ask_to_join(&person.keys, &Link::parse(&link).unwrap(), name).unwrap();
    owner.account.admit(&owner.keys, channel, person.account.device()).unwrap();
}

#[test]
fn two_people_set_up_a_server_and_a_channel_and_talk() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    assert!(anna.account.operator() && anna.account.approved());
    let mut ben = Person::invited_by(&mut anna);
    assert!(!ben.account.operator());

    let id = anna.account.create_channel(&anna.keys, "Family", "Anna", 3600).unwrap();
    let link = anna.account.invite_to_channel(&anna.keys, &id, 1, 3600).unwrap().to_string();

    // Ben asks; until Anna lets him in he is not in the channel.
    ben.account.ask_to_join(&ben.keys, &Link::parse(&link).unwrap(), "Ben").unwrap();
    assert_eq!(ben.account.asked().collect::<Vec<_>>(), [id.as_str()]);
    ben.sync();
    assert!(ben.account.channel(&id).is_none());

    let requests = anna.account.join_requests(&anna.keys, &id).unwrap();
    assert_eq!((requests.len(), requests[0].name.as_str()), (1, "Ben"));
    anna.account.admit(&anna.keys, &id, ben.account.device()).unwrap();

    assert!(ben.sync().contains(&Event::ChannelsChanged));
    let seen = ben.account.channel(&id).unwrap();
    assert_eq!((seen.name.as_str(), seen.max_age, seen.owner.as_str()), ("Family", 3600, anna.account.device()));
    assert_eq!(seen.name_of(anna.account.device()), Some("Anna"));
    assert_eq!(ben.account.asked().count(), 0);

    assert_eq!(anna.send(&id, "hello Ben"), Ok(true));
    assert_eq!(server.rows("messages"), 2, "a key hand-over and the message wait on the server");
    assert_eq!(Person::texts(&ben.sync()), ["Anna: hello Ben"]);
    assert_eq!(server.rows("messages"), 0, "collected, so wiped");

    assert_eq!(ben.send(&id, "hello Anna"), Ok(true));
    assert_eq!(ben.send(&id, "and again"), Ok(true));
    assert_eq!(Person::texts(&anna.sync()), ["Ben: hello Anna", "Ben: and again"]);
    assert_eq!(server.rows("messages"), 0);
    assert!(anna.sync().is_empty(), "nothing is delivered twice");
}

#[test]
fn someone_added_later_reads_from_then_on_and_someone_removed_reads_no_more() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);

    anna.send(&id, "before Cara").unwrap();
    assert_eq!(Person::texts(&ben.sync()), ["Anna: before Cara"]);

    admit(&mut anna, &id, &mut cara, "Cara");
    assert!(Person::texts(&cara.sync()).is_empty(), "nothing from before she joined");
    ben.sync();
    ben.send(&id, "welcome Cara").unwrap();
    assert_eq!(Person::texts(&cara.sync()), ["Ben: welcome Cara"]);
    assert_eq!(Person::texts(&anna.sync()), ["Ben: welcome Cara"]);

    // Anna removes Ben.
    let bens_device = ben.account.device().to_owned();
    anna.account.remove(&anna.keys, &id, &bens_device).unwrap();
    cara.sync();
    anna.send(&id, "just us now").unwrap();
    assert_eq!(Person::texts(&cara.sync()), ["Anna: just us now"]);

    let events = ben.sync();
    assert!(events.contains(&Event::ChannelsChanged) && Person::texts(&events).is_empty());
    assert!(ben.account.channel(&id).is_none(), "the channel is gone from Ben's app");
    assert_eq!(ben.send(&id, "hello?"), Err(ClientError::UnknownChannel));
    assert_eq!(cara.account.channel(&id).unwrap().members.len(), 2);
}

#[test]
fn only_the_server_with_the_right_fingerprint_is_spoken_to() {
    let server = TestServer::start();
    let other = TestServer::start();
    // The right address with another server's fingerprint: as if the address had been
    // taken over by someone else.
    let hijacked = ServerAddress { fingerprint: other.address.fingerprint.clone(), ..server.address.clone() };
    let way_in = WayIn { token: Some(server.operator_token.clone()), password: None };
    assert_eq!(Account::join(&hijacked, &way_in, &SoftDevice::new()).err(), Some(ClientError::Unreachable));
    assert_eq!(server.rows("devices"), 0, "nothing was sent to it, not even the token");

    assert!(Account::join(&server.address, &way_in, &SoftDevice::new()).is_ok());
}

#[test]
fn a_channel_that_is_not_owned_by_who_the_invite_says_is_refused() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut mallory = Person::invited_by(&mut anna);
    let id = anna.account.create_channel(&anna.keys, "Family", "Anna", 3600).unwrap();
    let mut link = anna.account.invite_to_channel(&anna.keys, &id, 1, 3600).unwrap();

    // An invite that names Mallory as the owner of Anna's channel.
    link.channel.as_mut().unwrap().owner = mallory.account.device().to_owned();
    let refused = ben.account.ask_to_join(&ben.keys, &link, "Ben");
    assert!(matches!(refused, Err(ClientError::Untrusted(_))), "{refused:?}");

    // An invite to a channel on some other server.
    let elsewhere = TestServer::start();
    let mut foreign = anna.account.invite_to_channel(&anna.keys, &id, 1, 3600).unwrap();
    foreign.server = elsewhere.address.clone();
    assert_eq!(mallory.account.ask_to_join(&mallory.keys, &foreign, "M"), Err(ClientError::WrongServer));
}

#[test]
fn a_device_waits_when_the_operator_approves_each_one() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let ways = anna.account.ways_in(&anna.keys).unwrap();
    assert_eq!(ways, WaysIn { open: false, tokens: true, approval: false, password: false });
    anna.account.set_ways_in(&anna.keys, &WaysIn { approval: true, ..ways }).unwrap();

    let mut ben = Person::invited_by(&mut anna);
    assert!(!ben.account.approved());
    assert_eq!(ben.try_sync(), Err(ClientError::AwaitingApproval));

    let waiting = anna.account.waiting_devices(&anna.keys).unwrap();
    assert_eq!(waiting, [ben.account.device()]);
    anna.account.approve_device(&anna.keys, &waiting[0]).unwrap();
    assert!(ben.try_sync().is_ok());
    assert!(ben.account.approved());

    // A non-operator cannot use the operator's controls.
    assert!(matches!(ben.account.ways_in(&ben.keys), Err(ClientError::Refused { status: 403, .. })));
}

#[test]
fn the_server_password_is_a_way_in() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    anna.account.set_server_password(&anna.keys, Some("correct horse")).unwrap();

    let wrong = WayIn { token: None, password: Some("wrong".into()) };
    let refused = Account::join(&server.address, &wrong, &SoftDevice::new()).err();
    assert!(matches!(refused, Some(ClientError::Refused { status: 403, .. })));
    let right = WayIn { token: None, password: Some("correct horse".into()) };
    assert!(Account::join(&server.address, &right, &SoftDevice::new()).is_ok());
}

#[test]
fn an_app_that_is_closed_and_reopened_carries_on() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);
    anna.send(&id, "one").unwrap();
    assert_eq!(Person::texts(&ben.sync()), ["Anna: one"]);

    anna.restart();
    ben.restart();
    assert_eq!(ben.account.channel(&id).unwrap().name, "Family");
    anna.send(&id, "two").unwrap();
    assert_eq!(Person::texts(&ben.sync()), ["Anna: two"]);
    ben.send(&id, "three").unwrap();
    assert_eq!(Person::texts(&anna.sync()), ["Ben: three"]);

    assert_eq!(Account::from_bytes(b"nonsense").err(), Some(ClientError::BadState));
}

#[test]
fn a_message_is_kept_until_the_app_says_it_has_stored_it() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);
    anna.send(&id, "one").unwrap();

    // Ben's phone collects the message, and is switched off before the app has written
    // it into its history. By then the server has wiped its copy.
    let first = ben.collect().unwrap();
    assert_eq!(Person::texts(&first), ["Anna: one"]);
    assert_eq!(server.rows("messages"), 0);
    ben.restart();

    // It is still there, under the same ID, however many times it is asked for.
    assert_eq!(ben.account.received(), first);
    assert_eq!(ben.collect().unwrap(), first);
    anna.send(&id, "two").unwrap();
    let both = ben.collect().unwrap();
    assert_eq!(Person::texts(&both), ["Anna: one", "Anna: two"]);
    assert_eq!(both[0], first[0]);

    // The app stores the first and says so; only the second is handed over from then on.
    ben.account.stored(&Person::ids(&first));
    ben.stored = ben.account.to_bytes();
    ben.restart();
    assert_eq!(Person::texts(&ben.collect().unwrap()), ["Anna: two"]);
    assert!(ben.sync().len() == 1 && ben.sync().is_empty(), "stored, so not handed over again");
}

#[test]
fn a_message_is_in_the_saved_state_before_the_server_is_told_to_wipe_it() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);
    anna.send(&id, "one").unwrap();

    // Every state Ben's phone is asked to save, with what the server held at that moment.
    let mut saves: Vec<(Vec<u8>, i64)> = Vec::new();
    ben.account.sync(&ben.keys, &mut |bytes| saves.push((bytes.to_vec(), server.rows("messages")))).unwrap();
    assert_eq!(server.rows("messages"), 0);

    // The phone dies the instant the server has been told: what is on its disk is the
    // last state saved while the server still had the message.
    let (on_disk, _) = saves.iter().rev().find(|(_, held)| *held > 0).expect("saved before the server is told");
    ben.account = Account::from_bytes(on_disk).unwrap();
    assert_eq!(Person::texts(&ben.account.received()), ["Anna: one"]);
    assert_eq!(Person::texts(&ben.sync()), ["Anna: one"]);

    // State saved by a version that did not keep opened messages still loads.
    let mut old: serde_json::Value = serde_json::from_slice(&ben.stored).unwrap();
    let fields = old.as_object_mut().unwrap();
    assert!(fields.remove("inbox").is_some());
    assert!(Account::from_bytes(old.to_string().as_bytes()).is_ok());
}

#[test]
fn a_message_written_with_no_signal_goes_when_the_server_is_back() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);
    anna.send(&id, "first").unwrap();
    assert_eq!(Person::texts(&ben.sync()), ["Anna: first"]);

    // Anna's app loses the server: the same stored state, pointed at a dead port.
    let port = format!("\"port\":{}", server.address.port);
    let stored = String::from_utf8(anna.stored.clone()).unwrap();
    assert!(stored.contains(&port));
    anna.account = Account::from_bytes(stored.replace(&port, "\"port\":9").as_bytes()).unwrap();
    assert_eq!(anna.send(&id, "written offline"), Ok(false), "sealed and kept, not sent");
    assert_eq!(anna.account.unsent(), 1);
    assert_eq!(server.rows("messages"), 0);

    // Signal returns. The stored state still holds the sealed message.
    let stored = String::from_utf8(anna.stored.clone()).unwrap();
    anna.account = Account::from_bytes(stored.replace("\"port\":9", &port).as_bytes()).unwrap();
    anna.sync();
    assert_eq!(anna.account.unsent(), 0);
    assert_eq!(Person::texts(&ben.sync()), ["Anna: written offline"]);
}

#[test]
fn the_open_connection_says_when_there_is_something_to_collect() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);

    let mut live = ben.account.open_live(&ben.keys).unwrap();
    assert_eq!(live.wait(Duration::from_millis(300)), Ok(Wake::Quiet));
    anna.send(&id, "are you there?").unwrap();
    assert_eq!(live.wait(Duration::from_secs(10)), Ok(Wake::Sync));
    assert_eq!(Person::texts(&ben.sync()), ["Anna: are you there?"]);
}

#[test]
fn members_see_a_new_maximum_message_age_and_who_has_left() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben"), (&mut cara, "Cara")]);

    anna.account.set_max_age(&anna.keys, &id, 600).unwrap();
    assert!(ben.sync().contains(&Event::ChannelsChanged));
    assert_eq!(ben.account.channel(&id).unwrap().max_age, 600);
    assert_eq!(ben.account.set_max_age(&ben.keys, &id, 60), Err(ClientError::NotOwner));

    // Cara leaves. Ben hears of it; Anna's phone, as the owner's, then signs a member
    // list without her, and they carry on.
    cara.account.leave(&cara.keys, &id).unwrap();
    assert!(cara.account.channel(&id).is_none());
    ben.sync();
    assert_eq!(ben.account.channel(&id).unwrap().left, [cara.account.device()]);
    anna.sync();
    let held = anna.account.channel(&id).unwrap();
    assert!(held.left.is_empty());
    assert!(held.members.iter().all(|member| member.device != cara.account.device()));
    assert_eq!(anna.send(&id, "still here"), Ok(true));
    assert_eq!(Person::texts(&ben.sync()), ["Anna: still here"]);
    assert_eq!(ben.account.channel(&id).unwrap().members.len(), 2);
    assert!(Person::texts(&cara.sync()).is_empty());
}

#[test]
fn taking_a_server_off_a_phone_leaves_nothing_waiting_for_it() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben"), (&mut cara, "Cara")]);
    let bens = ben.account.create_channel(&ben.keys, "Ben's own", "Ben", 3600).unwrap();
    admit(&mut ben, &bens, &mut cara, "Cara");
    cara.sync();
    assert!(cara.account.channel(&bens).is_some());

    assert_eq!(anna.send(&id, "are you there?"), Ok(true));
    cara.sync();
    assert!(server.rows("messages") > 0, "Ben has not collected it");

    ben.account.leave_server(&ben.keys).unwrap();
    assert_eq!(server.rows("devices"), 2);
    assert_eq!(server.rows("messages"), 0, "nothing waits for a phone that will never collect it");
    assert!(ben.try_sync().is_err());

    // The others carry on. Ben left his signed word behind for the channel he was in,
    // so Anna's phone takes him off its list unasked; the channel he owned is gone.
    assert_eq!(server.rows("leaves"), 1);
    cara.sync();
    assert!(cara.account.channel(&bens).is_none());
    assert_eq!(cara.account.channel(&id).unwrap().confirmed, [ben.account.device()]);
    anna.sync();
    assert_eq!(anna.account.channel(&id).unwrap().members.len(), 2);
    assert_eq!(anna.send(&id, "just us now"), Ok(true));
    assert_eq!(Person::texts(&cara.sync()), ["Anna: just us now"]);
}

#[test]
fn a_typed_address_gives_a_fingerprint_to_compare() {
    let server = TestServer::start();
    let port = server.address.port;
    let found = deaddrop_tls::ask_fingerprint("127.0.0.1", port, Duration::from_secs(5));
    assert_eq!(found.as_deref(), Some(server.address.fingerprint.as_str()));

    // Nothing listening, and something that is not a Dead Drop server.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    assert_eq!(deaddrop_tls::ask_fingerprint("127.0.0.1", closed, Duration::from_secs(2)), None);
    let plain = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let plain_port = plain.local_addr().unwrap().port();
    std::thread::spawn(move || {
        use std::io::Write;
        if let Ok((mut stream, _)) = plain.accept() {
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
        }
    });
    assert_eq!(deaddrop_tls::ask_fingerprint("127.0.0.1", plain_port, Duration::from_secs(2)), None);
}

#[test]
fn channels_and_people_can_be_renamed() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben"), (&mut cara, "Cara")]);

    // The owner renames the channel, and herself.
    anna.account.rename_channel(&anna.keys, &id, "Household").unwrap();
    assert_eq!(ben.account.rename_channel(&ben.keys, &id, "Mine"), Err(ClientError::NotOwner));
    let stored = &mut anna.stored;
    let done = anna.account.set_my_name(&anna.keys, &id, "Mum", &mut |bytes| *stored = bytes.to_vec());
    assert_eq!(done, Ok(true));
    ben.sync();
    let held = ben.account.channel(&id).unwrap();
    assert_eq!((held.name.as_str(), held.name_of(anna.account.device())), ("Household", Some("Mum")));

    // A member asks; the owner's phone makes the change when it collects.
    let stored = &mut ben.stored;
    let taken = ben.account.set_my_name(&ben.keys, &id, "cara", &mut |bytes| *stored = bytes.to_vec());
    assert!(matches!(taken, Err(ClientError::Invalid(_))), "names are unique whatever the capitals");
    let done = ben.account.set_my_name(&ben.keys, &id, "Benjamin", &mut |bytes| *stored = bytes.to_vec());
    assert_eq!(done, Ok(false));
    assert!(anna.sync().contains(&Event::ChannelsChanged));
    assert!(Person::texts(&cara.sync()).is_empty(), "the request is not shown as a message");
    assert_eq!(cara.account.channel(&id).unwrap().name_of(ben.account.device()), Some("Benjamin"));
    ben.sync();

    // Everyone still reads everyone.
    assert_eq!(ben.send(&id, "new name, same me"), Ok(true));
    assert_eq!(Person::texts(&anna.sync()), ["Benjamin: new name, same me"]);
    assert_eq!(Person::texts(&cara.sync()), ["Benjamin: new name, same me"]);
}

#[test]
fn safety_numbers_match_only_between_the_two_real_devices() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let ben = Person::invited_by(&mut anna);
    let eve = Person::invited_by(&mut anna);
    let annas = anna.account.safety_number(ben.account.device());
    assert_eq!(annas, ben.account.safety_number(anna.account.device()));
    // If the server had put Eve's device in Ben's place, Anna's number would differ from Ben's.
    assert_ne!(anna.account.safety_number(eve.account.device()), ben.account.safety_number(anna.account.device()));
}

#[test]
fn keys_that_expired_from_the_queue_are_asked_for_again() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);

    // Anna's first message and the keys for it expire before Ben collects them.
    assert_eq!(anna.send(&id, "one"), Ok(true));
    let wipe = server.state.store.run(|db| Ok(db.execute("DELETE FROM messages", [])?));
    server.runtime.block_on(wipe).unwrap();

    // Her next message uses keys Ben never got. He cannot read it, and asks.
    assert_eq!(anna.send(&id, "two"), Ok(true));
    let events = ben.sync();
    assert!(Person::texts(&events).is_empty());
    assert!(events.iter().any(|event| matches!(event, Event::Unreadable { .. })));

    // Anna's phone hears the request, and her next message carries the keys again.
    anna.sync();
    assert_eq!(anna.send(&id, "three"), Ok(true));
    assert_eq!(Person::texts(&ben.sync()), ["Anna: three"]);
    assert_eq!(ben.send(&id, "got that one"), Ok(true));
    assert_eq!(Person::texts(&anna.sync()), ["Ben: got that one"]);
}

#[test]
fn the_operator_sees_devices_by_id_and_can_remove_one() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let ben = Person::invited_by(&mut anna);
    let devices = anna.account.devices(&anna.keys).unwrap();
    assert_eq!(devices.len(), 2);
    assert!(devices.iter().any(|device| device.device == anna.account.device() && device.operator));
    assert!(devices.iter().any(|device| device.device == ben.account.device() && !device.operator && !device.waiting));
    anna.account.remove_device(&anna.keys, ben.account.device()).unwrap();
    assert_eq!(anna.account.devices(&anna.keys).unwrap().len(), 1);
}

#[test]
fn someone_whose_device_is_gone_stays_listed_until_the_owner_removes_them() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let mut dan = Person::invited_by(&mut anna);
    let id = channel(&mut ben, "Ben", &mut [(&mut anna, "Anna"), (&mut cara, "Cara")]);
    let gone = vec![cara.account.device().to_owned()];

    // The operator removes Cara's device. She signed nothing; the others have only the
    // server's word that her device is no more.
    anna.account.remove_device(&anna.keys, cara.account.device()).unwrap();
    anna.sync();
    assert_eq!(anna.account.channel(&id).unwrap().left, gone);
    ben.sync();
    let held = ben.account.channel(&id).unwrap();
    assert_eq!((&held.left, &held.missing), (&gone, &gone));
    assert!(held.confirmed.is_empty());
    assert_eq!(held.members.len(), 3, "Ben's phone, the owner's, signed nobody out");

    // The channel works meanwhile, and Ben can let someone else in.
    assert_eq!(ben.send(&id, "two of us"), Ok(true));
    assert_eq!(Person::texts(&anna.sync()), ["Ben: two of us"]);
    admit(&mut ben, &id, &mut dan, "Dan");
    assert_eq!(ben.account.channel(&id).unwrap().members.len(), 4);

    // Taking Cara off the list is Ben's decision, and he takes it.
    ben.account.remove(&ben.keys, &id, &gone[0]).unwrap();
    let held = ben.account.channel(&id).unwrap();
    assert_eq!(held.members.len(), 3);
    assert!(held.left.is_empty() && held.missing.is_empty());
}

#[test]
fn a_server_that_only_says_someone_has_left_gets_nobody_signed_out() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let mut dan = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben"), (&mut cara, "Cara")]);
    let version = anna.account.channel(&id).unwrap().roster_version;
    let cara_id = cara.account.device().to_owned();

    // The server lies: Cara has left, it says, with nothing signed by her to show for it.
    server.tamper(format!("UPDATE members SET gone = 1 WHERE device = '{cara_id}'"));
    anna.sync();
    let held = anna.account.channel(&id).unwrap();
    assert_eq!(held.left, [cara_id.as_str()]);
    assert!(held.confirmed.is_empty() && held.missing.is_empty());
    assert_eq!((held.members.len(), held.roster_version), (3, version), "Anna's phone signed nothing");

    // Signing a list for another reason does not drop her on the quiet either.
    admit(&mut anna, &id, &mut dan, "Dan");
    let held = anna.account.channel(&id).unwrap();
    assert_eq!(held.members.len(), 4);
    assert_eq!(held.name_of(&cara_id), Some("Cara"));

    // The server stops lying, and she is where she was.
    server.tamper(format!("UPDATE members SET gone = 0 WHERE device = '{cara_id}'"));
    anna.sync();
    assert!(anna.account.channel(&id).unwrap().left.is_empty());
    cara.sync();
    assert_eq!(anna.send(&id, "still with us"), Ok(true));
    assert_eq!(Person::texts(&cara.sync()), ["Anna: still with us"]);
}

#[test]
fn a_leave_is_believed_only_from_the_member_themselves_and_only_once() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben"), (&mut cara, "Cara")]);
    let cara_id = cara.account.device().to_owned();
    let still_in = |person: &Person| {
        let held = person.account.channel(&id).unwrap();
        held.members.len() == 3 && held.confirmed.is_empty() && held.left.is_empty()
    };

    // Ben signs a leave in Cara's name, and the server passes it off as hers.
    let forged = ben.keys.seal(&Leave { v: 1, channel: id.clone(), device: cara_id.clone(), roster: 3, issued: 0 });
    let key: String = ben.keys.signing_key_der().iter().map(|byte| format!("{byte:02x}")).collect();
    server.tamper(format!(
        "INSERT INTO leaves VALUES ('{id}', '{cara_id}', X'{key}', '{}', '{}')",
        forged.record, forged.signature
    ));
    anna.sync();
    ben.sync();
    assert!(still_in(&anna) && still_in(&ben), "not her key, so not her word");
    server.tamper("DELETE FROM leaves".into());

    // Cara really leaves. Ben's phone can tell it is her own word, and Anna's phone, as
    // the owner's, signs a list without her. The server keeps a copy of what she signed.
    cara.account.leave(&cara.keys, &id).unwrap();
    server.tamper("CREATE TABLE kept AS SELECT * FROM leaves".into());
    ben.sync();
    assert_eq!(ben.account.channel(&id).unwrap().confirmed, [cara_id.as_str()]);
    anna.sync();
    assert_eq!(anna.account.channel(&id).unwrap().members.len(), 2);
    assert_eq!(server.rows("leaves"), 0, "spent once the owner's list is without her");

    // She comes back, and the server plays her old leave again.
    admit(&mut anna, &id, &mut cara, "Cara");
    cara.sync();
    server.tamper("INSERT INTO leaves SELECT * FROM kept".into());
    anna.sync();
    ben.sync();
    assert!(still_in(&anna) && still_in(&ben), "signed before she was let back in, so it no longer counts");
    assert_eq!(anna.send(&id, "welcome back"), Ok(true));
    assert_eq!(Person::texts(&cara.sync()), ["Anna: welcome back"]);
}

#[test]
fn someone_taken_out_or_whose_channel_is_deleted_is_shown_the_owners_signature() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben"), (&mut cara, "Cara")]);

    // Anna takes Ben out. The server keeps her signed list for him; his phone checks it,
    // gives the channel up, and says it has seen it.
    let bens_device = ben.account.device().to_owned();
    anna.account.remove(&anna.keys, &id, &bens_device).unwrap();
    assert_eq!(server.rows("removals"), 1);
    assert!(ben.sync().contains(&Event::ChannelsChanged));
    assert!(ben.account.channel(&id).is_none());
    assert_eq!(server.rows("removals"), 0);

    // Anna deletes the channel. All that is left of it is her signed word, for Cara's
    // phone (and for her own, had it not heard that the deletion went through).
    cara.sync();
    anna.account.delete_channel(&anna.keys, &id).unwrap();
    assert_eq!((server.rows("channels"), server.rows("removals")), (0, 2));
    assert!(cara.sync().contains(&Event::ChannelsChanged));
    assert!(cara.account.channel(&id).is_none());
    anna.sync();
    assert_eq!(server.rows("removals"), 0);
}

#[test]
fn a_server_that_only_says_a_phone_is_out_of_a_channel_does_not_cost_it_its_keys() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    // Anna's first member list, from before Ben was let in, is put by to be played later.
    let id = anna.account.create_channel(&anna.keys, "Family", "Anna", 3600).unwrap();
    server.tamper("CREATE TABLE early AS SELECT id, roster, roster_sig FROM channels".into());
    admit(&mut anna, &id, &mut ben, "Ben");
    admit(&mut anna, &id, &mut cara, "Cara");
    for person in [&mut anna, &mut ben, &mut cara] {
        person.sync();
    }
    assert_eq!(anna.send(&id, "hello"), Ok(true));
    assert_eq!(Person::texts(&ben.sync()), ["Anna: hello"]);
    let ben_id = ben.account.device().to_owned();
    let hex = |bytes: Vec<u8>| bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let (annas_key, caras_key) = (hex(anna.keys.signing_key_der()), hex(cara.keys.signing_key_der()));

    // The server lies: Ben is not in the channel, it says, with nothing from Anna to show.
    server.tamper(format!("DELETE FROM members WHERE device = '{ben_id}'"));
    assert!(ben.sync().contains(&Event::ChannelsChanged));
    let held = ben.account.channel(&id).unwrap();
    assert!(held.unlisted, "kept, and marked as not on the server");
    assert_eq!(held.members.len(), 3);
    assert!(matches!(ben.send(&id, "hello?"), Err(ClientError::Invalid(_))));
    assert!(!ben.sync().contains(&Event::ChannelsChanged), "nothing new to tell the app");

    // It tries harder: Anna's own list from before Ben was let in; then a deletion Cara
    // signed, passed off under Cara's key and under Anna's.
    server.tamper(format!(
        "INSERT INTO removals SELECT id, '{ben_id}', X'{annas_key}', 0, roster, roster_sig, 99999999999 FROM early"
    ));
    ben.sync();
    assert!(ben.account.channel(&id).is_some(), "a list older than the one Ben is in proves nothing");
    let forged = cara.keys.seal(&Closed { v: 1, channel: id.clone(), issued: 0 });
    for key in [&caras_key, &annas_key] {
        server.tamper(format!(
            "INSERT OR REPLACE INTO removals VALUES ('{id}', '{ben_id}', X'{key}', 1, '{}', '{}', 99999999999)",
            forged.record, forged.signature
        ));
        ben.sync();
        assert!(ben.account.channel(&id).is_some(), "not the owner's signature");
    }
    assert_eq!(server.rows("removals"), 1, "what failed its check is not said to have been seen");

    // The server stops lying. Ben has his keys still, and is where he was.
    server.tamper(format!("DELETE FROM removals; INSERT INTO members (channel, device) VALUES ('{id}', '{ben_id}')"));
    assert!(ben.sync().contains(&Event::ChannelsChanged));
    assert!(!ben.account.channel(&id).unwrap().unlisted);
    assert_eq!(anna.send(&id, "still with us"), Ok(true));
    assert_eq!(Person::texts(&ben.sync()), ["Anna: still with us"]);
    assert_eq!(ben.send(&id, "I am"), Ok(true));
    assert_eq!(Person::texts(&anna.sync()), ["Ben: I am"]);
}

#[test]
fn a_channel_the_server_no_longer_shows_is_the_persons_to_take_off_their_phone() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let id = channel(&mut ben, "Ben", &mut [(&mut cara, "Cara")]);

    // The operator removes Ben's device, and his channel goes with it. He signed
    // nothing, so Cara's phone has only the server's word that the channel is gone.
    anna.account.remove_device(&anna.keys, ben.account.device()).unwrap();
    assert_eq!(server.rows("channels"), 0);
    cara.sync();
    assert!(cara.account.channel(&id).unwrap().unlisted);
    cara.restart();
    assert!(cara.account.channel(&id).unwrap().unlisted, "remembered across a restart");

    // Taking it off her phone is her decision, and needs nothing from the server.
    cara.account.leave(&cara.keys, &id).unwrap();
    assert!(cara.account.channel(&id).is_none());
    assert!(!cara.sync().contains(&Event::ChannelsChanged));
}

#[test]
fn only_a_device_holding_the_invite_is_let_in_and_under_the_name_it_asked_for() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let eve = Person::invited_by(&mut anna);
    let id = anna.account.create_channel(&anna.keys, "Family", "Anna", 3600).unwrap();
    let (ben_id, eve_id) = (ben.account.device().to_owned(), eve.account.device().to_owned());

    // Anna makes an invite, and her app is closed and opened again before anyone uses it.
    let link = anna.account.invite_to_channel(&anna.keys, &id, 1, 3600).unwrap();
    anna.stored = anna.account.to_bytes();
    anna.restart();
    ben.account.ask_to_join(&ben.keys, &link, "Ben").unwrap();
    server.tamper("CREATE TABLE asked AS SELECT * FROM join_requests".into());
    let nobody = |anna: &mut Person| anna.account.join_requests(&anna.keys, &id).unwrap().is_empty();

    // The server puts Eve's device where Ben's was, keeping his name and his stamp.
    server.tamper(format!("UPDATE join_requests SET device = '{eve_id}'"));
    assert!(nobody(&mut anna), "the stamp is for Ben's device, not Eve's");
    assert_eq!(server.rows("join_requests"), 0, "and what cannot be let in is declined");
    let refused = anna.account.admit(&anna.keys, &id, &eve_id);
    assert!(matches!(refused, Err(ClientError::Refused { status: 404, .. })), "{refused:?}");

    // It makes a request up for Eve, and it gives Ben's request another name.
    server.tamper(format!("INSERT INTO join_requests VALUES ('{id}', '{eve_id}', 'Ben', 'bm90IGEgc3RhbXA=')"));
    assert!(nobody(&mut anna));
    server.tamper("INSERT INTO join_requests SELECT * FROM asked; UPDATE join_requests SET name = 'Bob'".into());
    assert!(nobody(&mut anna), "the stamp is for the name Ben asked for");
    assert_eq!(anna.account.channel(&id).unwrap().members.len(), 1, "nobody was let in");

    // Ben's request as he made it is shown, and lets him in.
    server.tamper("INSERT INTO join_requests SELECT * FROM asked".into());
    let requests = anna.account.join_requests(&anna.keys, &id).unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!((requests[0].device.as_str(), requests[0].name.as_str()), (ben_id.as_str(), "Ben"));
    anna.account.admit(&anna.keys, &id, &ben_id).unwrap();
    ben.sync();
    assert_eq!(anna.send(&id, "welcome"), Ok(true));
    assert_eq!(Person::texts(&ben.sync()), ["Anna: welcome"]);

    // The invite is spent. Once Anna has taken Ben out, his old request, played
    // again by the server, is not a way back in.
    anna.account.remove(&anna.keys, &id, &ben_id).unwrap();
    server.tamper("INSERT INTO join_requests SELECT * FROM asked".into());
    assert!(nobody(&mut anna));
}

#[test]
fn an_invite_made_for_two_lets_two_in_and_no_more() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let mut dan = Person::invited_by(&mut anna);
    let id = anna.account.create_channel(&anna.keys, "Family", "Anna", 3600).unwrap();
    let link = anna.account.invite_to_channel(&anna.keys, &id, 2, 3600).unwrap();
    server.tamper(format!("CREATE TABLE spent AS SELECT * FROM tokens WHERE scope = '{id}'"));

    ben.account.ask_to_join(&ben.keys, &link, "Ben").unwrap();
    anna.account.admit(&anna.keys, &id, ben.account.device()).unwrap();
    cara.account.ask_to_join(&cara.keys, &link, "Cara").unwrap();
    anna.account.admit(&anna.keys, &id, cara.account.device()).unwrap();
    assert_eq!(anna.account.channel(&id).unwrap().members.len(), 3);

    // The server will not take the invite a third time. Were it to (here, by being
    // made to), Anna's phone would not: the invite has let in the two it was made for.
    let refused = dan.account.ask_to_join(&dan.keys, &link, "Dan");
    assert!(matches!(refused, Err(ClientError::Refused { status: 403, .. })), "{refused:?}");
    server.tamper("INSERT INTO tokens SELECT hash, scope, 1, expires FROM spent".into());
    dan.account.ask_to_join(&dan.keys, &link, "Dan").unwrap();
    assert!(anna.account.join_requests(&anna.keys, &id).unwrap().is_empty());
    assert_eq!(server.rows("join_requests"), 0);
}

#[test]
fn a_message_the_server_cannot_take_just_now_waits_and_goes_later() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);

    // The server is in trouble: it answers, but with a fault of its own.
    server.tamper("ALTER TABLE messages RENAME TO mislaid".into());
    assert_eq!(anna.send(&id, "one"), Ok(false));
    assert_eq!(anna.send(&id, "two"), Ok(false));
    // Waiting, in order: Ben's keys for these messages, the first message, the second.
    assert_eq!((anna.account.unsent(), anna.account.waiting().len()), (3, 2));
    assert!(anna.account.not_sent().is_empty(), "a fault is not a refusal");

    // It is put right, and Anna's app has been closed and opened in between. The keys
    // went first, so Ben can read what follows them.
    server.tamper("ALTER TABLE mislaid RENAME TO messages".into());
    anna.restart();
    assert_eq!(anna.account.waiting().len(), 2);
    anna.sync();
    assert_eq!((anna.account.unsent(), anna.account.waiting().len()), (0, 0));
    assert!(anna.account.not_sent().is_empty());
    assert_eq!(Person::texts(&ben.sync()), ["Anna: one", "Anna: two"]);
}

#[test]
fn a_message_the_server_will_never_take_is_said_not_to_have_been_sent() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);

    // One the server has taken leaves nothing behind in the engine.
    assert_eq!(ben.send(&id, "hello"), Ok(true));
    assert!(ben.account.waiting().is_empty() && ben.account.not_sent().is_empty());

    // One that is too large to send never reaches the server. What follows it does.
    assert_eq!(anna.send(&id, &"x".repeat(40_000)), Ok(true), "nothing is left waiting");
    assert_eq!(anna.send(&id, "short"), Ok(true));
    let refused = anna.account.not_sent().to_vec();
    assert_eq!(refused.len(), 1);
    assert_eq!((refused[0].channel.as_str(), &refused[0].why), (id.as_str(), &Why::TooLarge));
    assert_eq!(Person::texts(&ben.sync()), ["Anna: short"]);
    assert_eq!(Person::texts(&anna.sync()), ["Ben: hello"]);

    // Anna takes Ben out. His phone has not heard when he writes again.
    anna.account.remove(&anna.keys, &id, ben.account.device()).unwrap();
    assert_eq!(ben.send(&id, "anyone there?"), Ok(true), "nothing is left waiting");
    let refused = ben.account.not_sent().to_vec();
    assert_eq!(refused.len(), 1);
    assert_eq!((refused[0].channel.as_str(), &refused[0].why), (id.as_str(), &Why::NotInChannel));
    assert_eq!(server.rows("messages"), 0);

    // It stays listed, through the app closing, until the app says it has noted it.
    ben.restart();
    assert_eq!(ben.account.not_sent(), refused);
    ben.account.stored(&[refused[0].id.clone()]);
    assert!(ben.account.not_sent().is_empty());
}

#[test]
fn what_was_waiting_for_a_channel_this_phone_is_taken_out_of_is_not_sent() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);

    // Ben writes with no signal, and Anna takes him out before it returns.
    ben.signal(&server, false);
    assert_eq!(ben.send(&id, "running late"), Ok(false));
    let waiting = ben.account.waiting();
    assert_eq!(waiting.len(), 1);
    anna.account.remove(&anna.keys, &id, ben.account.device()).unwrap();

    // The signal returns to a server that will not store anything just now, so the
    // message is still waiting when Ben's phone learns, from Anna's signature, that it
    // is out of the channel and gives the channel up.
    server.tamper("ALTER TABLE messages RENAME TO mislaid".into());
    ben.signal(&server, true);
    assert!(ben.try_sync().is_err(), "the server cannot hand over its queue either");
    assert!(ben.account.channel(&id).is_none());
    assert!(ben.account.waiting().is_empty());
    let refused = ben.account.not_sent();
    assert_eq!(refused.len(), 1);
    assert_eq!((&refused[0].id, &refused[0].why), (&waiting[0], &Why::NotInChannel));
}

#[test]
fn a_message_written_with_no_signal_to_someone_new_waits_for_their_keys() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);
    admit(&mut anna, &id, &mut cara, "Cara");
    ben.sync();
    cara.sync();

    // Ben has never written to Cara, so sealing anything for her takes a key of hers
    // from the server. With no signal the message is taken all the same, and kept.
    ben.signal(&server, false);
    assert_eq!(ben.send(&id, "welcome Cara"), Ok(false));
    assert_eq!((ben.account.waiting().len(), server.rows("messages")), (1, 0));

    ben.signal(&server, true);
    assert_eq!(ben.account.waiting().len(), 1, "kept through the app closing");
    ben.sync();
    assert!(ben.account.waiting().is_empty() && ben.account.not_sent().is_empty());
    assert_eq!(Person::texts(&cara.sync()), ["Ben: welcome Cara"]);
    assert_eq!(Person::texts(&anna.sync()), ["Ben: welcome Cara"]);
}

#[test]
fn a_waiting_message_can_be_taken_back_and_what_follows_it_still_goes() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let id = channel(&mut anna, "Anna", &mut [(&mut ben, "Ben")]);

    // Ben has written, so Anna's phone can seal for him without the server's help.
    assert_eq!(ben.send(&id, "hello"), Ok(true));
    assert_eq!(Person::texts(&anna.sync()), ["Ben: hello"]);

    // No signal. The first message is sealed, with Ben's keys for the channel ahead of
    // it; the other two are not sealed yet.
    anna.signal(&server, false);
    for text in ["not this", "nor this", "this one"] {
        assert_eq!(anna.send(&id, text), Ok(false));
    }
    let waiting = anna.account.waiting();
    assert_eq!((waiting.len(), anna.account.unsent()), (3, 4));

    // Anna takes two of them back: the sealed one and one that is not.
    assert!(anna.account.cancel(&waiting[0]) && anna.account.cancel(&waiting[1]));
    assert!(!anna.account.cancel(&waiting[0]), "it is not there to take back twice");
    assert_eq!(anna.account.waiting(), [waiting[2].clone()]);
    assert_eq!(anna.account.unsent(), 2, "Ben's keys still go: the message that is left needs them");
    anna.stored = anna.account.to_bytes();

    // The signal returns. Only the one she left goes, and Ben can read it.
    anna.signal(&server, true);
    anna.sync();
    assert!(anna.account.waiting().is_empty() && anna.account.not_sent().is_empty());
    assert_eq!(Person::texts(&ben.sync()), ["Anna: this one"]);
    assert!(!anna.account.cancel(&waiting[2]), "the server has it: too late to take back");
}
