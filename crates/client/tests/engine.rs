//! The client engine against a real server over real TLS: what the app will do,
//! start to finish, through the same code the app will call.

use std::time::Duration;

use deaddrop_client::{Account, ClientError, Event, Link, ServerAddress, Wake, WayIn, WaysIn};
use deaddrop_core::encoding::random_id;
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

    fn sync(&mut self) -> Vec<Event> {
        self.try_sync().unwrap()
    }

    fn try_sync(&mut self) -> Result<Vec<Event>, ClientError> {
        let stored = &mut self.stored;
        self.account.sync(&self.keys, &mut |bytes| *stored = bytes.to_vec())
    }

    fn send(&mut self, channel: &str, text: &str) -> Result<bool, ClientError> {
        let stored = &mut self.stored;
        self.account.send(&self.keys, channel, text, &mut |bytes| *stored = bytes.to_vec())
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

    // The others carry on: Ben counts as having left, and the channel he owned is gone.
    cara.sync();
    assert!(cara.account.channel(&bens).is_none());
    assert_eq!(cara.account.channel(&id).unwrap().left, [ben.account.device()]);
    anna.sync();
    assert_eq!(anna.account.channel(&id).unwrap().members.len(), 2);
    assert_eq!(anna.send(&id, "just us now"), Ok(true));
    assert_eq!(Person::texts(&cara.sync()), ["Anna: just us now"]);
}

#[test]
fn a_device_the_operator_removes_counts_as_having_left() {
    let server = TestServer::start();
    let mut anna = Person::operator(&server);
    let mut ben = Person::invited_by(&mut anna);
    let mut cara = Person::invited_by(&mut anna);
    let id = channel(&mut ben, "Ben", &mut [(&mut anna, "Anna"), (&mut cara, "Cara")]);

    anna.account.remove_device(&anna.keys, cara.account.device()).unwrap();
    anna.sync();
    assert_eq!(anna.account.channel(&id).unwrap().left, [cara.account.device()]);
    ben.sync();
    assert_eq!(ben.account.channel(&id).unwrap().members.len(), 2);
    assert_eq!(ben.send(&id, "two of us"), Ok(true));
    assert_eq!(Person::texts(&anna.sync()), ["Ben: two of us"]);
}
