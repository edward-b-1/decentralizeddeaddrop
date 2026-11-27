//! Messengers talking to each other directly, with the test standing in for the server.

use deaddrop_crypto::{Error, Incoming, MAX_PLAINTEXT_BYTES, Messenger, Outgoing, ROTATE_AFTER_MESSAGES};

const CHANNEL: &str = "channel-one";
const NOW: i64 = 1_800_000_000;

/// Device IDs are just names here; in the real system they are hashes of signing keys.
fn group(names: &[&str]) -> Vec<Messenger> {
    let mut all: Vec<Messenger> = names.iter().map(|name| Messenger::new(name)).collect();
    let keys: Vec<_> = all.iter().map(|messenger| messenger.keys(NOW)).collect();
    for messenger in &mut all {
        for key in &keys {
            if key.device != messenger.device() {
                messenger.add_peer(key).unwrap();
            }
        }
    }
    set_roster(&mut all, 1, names);
    all
}

fn set_roster(all: &mut [Messenger], version: u64, members: &[&str]) {
    let members: Vec<String> = members.iter().map(|name| name.to_string()).collect();
    for messenger in all.iter_mut() {
        messenger.set_roster(CHANNEL, version, &members);
    }
}

fn index(all: &[Messenger], device: &str) -> usize {
    all.iter().position(|messenger| messenger.device() == device).unwrap()
}

/// What the app does before sending: claim a prekey for every member it has no
/// private session with yet.
fn ensure_sessions(all: &mut [Messenger], from: usize) {
    for device in all[from].missing_sessions(CHANNEL).unwrap() {
        let to = index(all, &device);
        let prekey = all[to].new_prekeys(1, false).one_time.remove(0);
        all[from].start_session(&device, &prekey).unwrap();
    }
}

fn send(all: &mut [Messenger], from: usize, text: &str) -> Outgoing {
    ensure_sessions(all, from);
    all[from].encrypt(CHANNEL, text.as_bytes(), NOW).unwrap()
}

/// Hands every share to the device it is for.
fn deliver_shares(all: &mut [Messenger], from: usize, outgoing: &Outgoing) {
    let sender = all[from].device().to_owned();
    for (device, body) in &outgoing.shares {
        let to = index(all, device);
        assert_eq!(all[to].decrypt(CHANNEL, &sender, body), Ok(Incoming::KeysReceived));
    }
}

fn open(all: &mut [Messenger], from: usize, to: usize, body: &[u8]) -> Result<String, Error> {
    let sender = all[from].device().to_owned();
    match all[to].decrypt(CHANNEL, &sender, body)? {
        Incoming::Message(plaintext) => Ok(String::from_utf8(plaintext).unwrap()),
        Incoming::KeysReceived | Incoming::KeysWanted => panic!("expected a message"),
    }
}

/// Sends from one member and checks that every other member reads it.
fn say(all: &mut [Messenger], from: usize, text: &str) -> Outgoing {
    let outgoing = send(all, from, text);
    deliver_shares(all, from, &outgoing);
    for to in 0..all.len() {
        if to != from {
            assert_eq!(open(all, from, to, &outgoing.message).as_deref(), Ok(text));
        }
    }
    outgoing
}

#[test]
fn members_of_a_channel_read_each_others_messages() {
    let mut all = group(&["anna", "ben", "cara"]);
    let first = say(&mut all, 0, "hello from anna");
    assert_eq!(first.shares.len(), 2, "the first message hands anna's ratchet to both others");
    let second = say(&mut all, 0, "and again");
    assert!(second.shares.is_empty(), "after that, messages need no hand-over");
    say(&mut all, 1, "ben here");
    say(&mut all, 2, "cara here");
    say(&mut all, 0, "anna once more");
}

#[test]
fn a_message_reveals_neither_its_text_nor_its_length() {
    let mut all = group(&["anna", "ben"]);
    let short = say(&mut all, 0, "ok");
    let longer = say(&mut all, 0, &"x".repeat(200));
    assert_eq!(short.message.len(), longer.message.len());
    let secret = "the eagle lands at midnight";
    let sealed = say(&mut all, 0, secret);
    let contains = |haystack: &[u8]| haystack.windows(secret.len()).any(|window| window == secret.as_bytes());
    assert!(!contains(&sealed.message));
    assert!(sealed.shares.iter().all(|(_, body)| !contains(body)));
}

#[test]
fn an_opened_message_cannot_be_opened_again_even_from_saved_state() {
    let mut all = group(&["anna", "ben"]);
    let one = send(&mut all, 0, "one");
    deliver_shares(&mut all, 0, &one);
    let two = send(&mut all, 0, "two");

    // Ben's state as stored before he reads anything can still open the message...
    let before = all[1].to_bytes();
    assert_eq!(open(&mut all, 0, 1, &one.message).as_deref(), Ok("one"));
    // ...but his state afterwards cannot, live or restored from storage.
    assert_eq!(open(&mut all, 0, 1, &one.message), Err(Error::AlreadyOpened));
    let mut after = Messenger::from_bytes(&all[1].to_bytes()).unwrap();
    assert_eq!(after.decrypt(CHANNEL, "anna", &one.message), Err(Error::AlreadyOpened));
    // (The test can tell: the earlier copy of his state still opens it.)
    let mut earlier = Messenger::from_bytes(&before).unwrap();
    assert_eq!(earlier.decrypt(CHANNEL, "anna", &one.message), Ok(Incoming::Message(b"one".to_vec())));

    // Reading a later message destroys the keys for everything before it as well.
    let three = send(&mut all, 0, "three");
    assert_eq!(open(&mut all, 0, 1, &three.message).as_deref(), Ok("three"));
    assert_eq!(open(&mut all, 0, 1, &two.message), Err(Error::AlreadyOpened));
}

#[test]
fn a_removed_member_cannot_read_what_is_sent_afterwards() {
    let mut all = group(&["anna", "ben", "cara"]);
    say(&mut all, 0, "before");

    // The owner publishes a roster without cara. Anna and ben learn of it; cara, cut
    // off, still believes the old one.
    set_roster(&mut all[..2], 2, &["anna", "ben"]);
    let after = send(&mut all, 0, "after");
    assert_eq!(after.shares.len(), 1, "a new ratchet, handed to ben only");
    assert_eq!(after.shares[0].0, "ben");
    deliver_shares(&mut all, 0, &after);
    assert_eq!(open(&mut all, 0, 1, &after.message).as_deref(), Ok("after"));
    assert_eq!(open(&mut all, 0, 2, &after.message), Err(Error::UnknownSession));
    // Nor can cara open the hand-over meant for ben.
    assert_eq!(all[2].decrypt(CHANNEL, "anna", &after.shares[0].1), Err(Error::CannotOpen));

    // And the others no longer accept anything from cara.
    let from_cara = send(&mut all, 2, "still here");
    assert_eq!(open(&mut all, 2, 0, &from_cara.message), Err(Error::NotAMember));
}

#[test]
fn a_new_member_reads_from_when_they_joined_and_not_before() {
    let mut all = group(&["anna", "ben", "dan"]);
    set_roster(&mut all, 1, &["anna", "ben"]);
    let before = send(&mut all, 0, "before dan");
    deliver_shares(&mut all, 0, &before);
    assert_eq!(open(&mut all, 0, 1, &before.message).as_deref(), Ok("before dan"));

    set_roster(&mut all, 2, &["anna", "ben", "dan"]);
    let after = say(&mut all, 0, "welcome dan");
    assert_eq!(after.shares.len(), 2, "a new ratchet for everyone, so dan gets nothing older");
    assert_eq!(open(&mut all, 0, 2, &before.message), Err(Error::UnknownSession));
}

#[test]
fn a_message_from_someone_not_yet_in_my_roster_waits_until_i_have_it() {
    let mut all = group(&["anna", "ben", "dan"]);
    set_roster(&mut all, 1, &["anna", "ben"]);
    // Dan has been added, but ben has not fetched the new roster yet.
    set_roster(&mut all[2..], 2, &["anna", "ben", "dan"]);
    set_roster(&mut all[..1], 2, &["anna", "ben", "dan"]);
    let hello = send(&mut all, 2, "hello, I am new");
    deliver_shares(&mut all, 2, &hello);
    assert_eq!(open(&mut all, 2, 1, &hello.message), Err(Error::NotAMember));
    // Refused, not lost: once ben has the roster the same message opens.
    set_roster(&mut all[1..2], 2, &["anna", "ben", "dan"]);
    assert_eq!(open(&mut all, 2, 1, &hello.message).as_deref(), Ok("hello, I am new"));
}

#[test]
fn an_older_roster_never_replaces_a_newer_one() {
    let mut all = group(&["anna", "ben", "cara"]);
    set_roster(&mut all, 5, &["anna", "ben"]);
    set_roster(&mut all, 4, &["anna", "ben", "cara"]);
    let outgoing = send(&mut all, 0, "hello");
    assert_eq!(outgoing.shares.len(), 1, "cara, removed in version 5, stays removed");
}

#[test]
fn the_server_cannot_pass_a_message_off_as_someone_elses() {
    let mut all = group(&["anna", "ben", "cara"]);
    say(&mut all, 0, "from anna");
    say(&mut all, 2, "from cara");
    let outgoing = send(&mut all, 0, "really from anna");
    // Delivered to ben labelled as cara's, as a stranger's, and in another channel.
    assert_eq!(all[1].decrypt(CHANNEL, "cara", &outgoing.message), Err(Error::UnknownSession));
    assert_eq!(all[1].decrypt(CHANNEL, "mallory", &outgoing.message), Err(Error::NotAMember));
    assert_eq!(all[1].decrypt("another-channel", "anna", &outgoing.message), Err(Error::UnknownChannel));
    // None of that spoiled the real delivery.
    assert_eq!(open(&mut all, 0, 1, &outgoing.message).as_deref(), Ok("really from anna"));
}

#[test]
fn a_hand_over_for_one_channel_is_refused_in_another() {
    let mut all = group(&["anna", "ben"]);
    let outgoing = send(&mut all, 0, "hello");
    assert_eq!(all[1].decrypt("another-channel", "anna", &outgoing.shares[0].1), Err(Error::CannotOpen));
}

#[test]
fn any_change_to_a_message_is_detected() {
    let mut all = group(&["anna", "ben"]);
    let outgoing = send(&mut all, 0, "untouched");
    deliver_shares(&mut all, 0, &outgoing);
    for position in (0..outgoing.message.len()).step_by(7) {
        let mut altered = outgoing.message.clone();
        altered[position] ^= 0x01;
        let result = all[1].decrypt(CHANNEL, "anna", &altered);
        assert!(result.is_err(), "byte {position} was altered and the message still opened");
    }
    for cut in [0, 1, 2, 10, outgoing.message.len() - 1] {
        assert!(all[1].decrypt(CHANNEL, "anna", &outgoing.message[..cut]).is_err(), "cut to {cut} bytes");
    }
    assert_eq!(open(&mut all, 0, 1, &outgoing.message).as_deref(), Ok("untouched"));
}

#[test]
fn a_prekey_must_be_signed_by_the_device_it_is_for() {
    let mut all = group(&["anna", "ben", "cara"]);
    let bens = all[1].new_prekeys(1, false).one_time.remove(0);
    let caras = all[2].new_prekeys(1, false).one_time.remove(0);

    // Cara's prekey offered as ben's: what a server swapping keys would do.
    assert_eq!(all[0].start_session("ben", &caras), Err(Error::BadKeys));
    // Ben's key with somebody else's signature on it.
    let mut forged = bens.clone();
    forged.signature = caras.signature.clone();
    assert_eq!(all[0].start_session("ben", &forged), Err(Error::BadKeys));
    assert_eq!(all[0].start_session("nobody", &bens), Err(Error::UnknownPeer("nobody".into())));
    assert_eq!(all[0].start_session("ben", &bens), Ok(()));
}

#[test]
fn a_device_cannot_quietly_change_its_keys() {
    let mut all = group(&["anna", "ben"]);
    let impostor = Messenger::new("ben");
    assert_eq!(all[0].add_peer(&impostor.keys(NOW)), Err(Error::KeysChanged));
    let same = all[1].keys(NOW + 5);
    assert_eq!(all[0].add_peer(&same), Ok(()), "the same keys again are fine");
    let own = all[0].keys(NOW);
    assert_eq!(all[0].add_peer(&own), Err(Error::BadKeys), "a device is not its own peer");
}

#[test]
fn sending_needs_a_session_with_every_member_first() {
    let mut all = group(&["anna", "ben", "cara"]);
    let mut missing = all[0].missing_sessions(CHANNEL).unwrap();
    missing.sort();
    assert_eq!(missing, ["ben", "cara"]);
    assert_eq!(all[0].encrypt(CHANNEL, b"hello", NOW), Err(Error::NoSession(vec!["ben".into(), "cara".into()])));
    ensure_sessions(&mut all, 0);
    assert!(all[0].missing_sessions(CHANNEL).unwrap().is_empty());
    assert!(all[0].encrypt(CHANNEL, b"hello", NOW).is_ok());
    assert_eq!(all[0].encrypt("another-channel", b"hello", NOW), Err(Error::UnknownChannel));
}

#[test]
fn two_devices_can_start_talking_at_the_same_moment() {
    let mut all = group(&["anna", "ben"]);
    // Each starts its own session with the other before hearing anything.
    let from_anna = send(&mut all, 0, "anna first");
    let from_ben = send(&mut all, 1, "ben first");
    deliver_shares(&mut all, 0, &from_anna);
    deliver_shares(&mut all, 1, &from_ben);
    assert_eq!(open(&mut all, 0, 1, &from_anna.message).as_deref(), Ok("anna first"));
    assert_eq!(open(&mut all, 1, 0, &from_ben.message).as_deref(), Ok("ben first"));
    // And they keep working afterwards, including across a new ratchet.
    set_roster(&mut all, 2, &["anna", "ben"]);
    say(&mut all, 0, "anna again");
    say(&mut all, 1, "ben again");
}

#[test]
fn the_sending_ratchet_is_replaced_after_a_hundred_messages_or_a_week() {
    let mut all = group(&["anna", "ben"]);
    for n in 0..ROTATE_AFTER_MESSAGES {
        let outgoing = say(&mut all, 0, &format!("message {n}"));
        assert_eq!(outgoing.shares.len(), usize::from(n == 0), "message {n}");
    }
    let next = say(&mut all, 0, "one more");
    assert_eq!(next.shares.len(), 1, "a new ratchet after a hundred messages");

    ensure_sessions(&mut all, 0);
    let later = all[0].encrypt(CHANNEL, b"a week on", NOW + 7 * 86_400).unwrap();
    assert_eq!(later.shares.len(), 1, "and after a week");
}

#[test]
fn everything_survives_being_saved_and_restored() {
    let mut all = group(&["anna", "ben", "cara"]);
    say(&mut all, 0, "one");
    say(&mut all, 1, "two");
    let waiting = send(&mut all, 2, "sent before the restart");
    deliver_shares(&mut all, 2, &waiting);

    let mut all: Vec<Messenger> =
        all.iter().map(|messenger| Messenger::from_bytes(&messenger.to_bytes()).unwrap()).collect();
    assert_eq!(open(&mut all, 2, 0, &waiting.message).as_deref(), Ok("sent before the restart"));
    assert_eq!(open(&mut all, 2, 1, &waiting.message).as_deref(), Ok("sent before the restart"));
    let next = say(&mut all, 0, "three");
    assert!(next.shares.is_empty(), "restored devices carry on with the same ratchets");
    say(&mut all, 2, "four");

    assert_eq!(Messenger::from_bytes(b"not a saved messenger").err(), Some(Error::Malformed));
}

#[test]
fn a_message_whose_keys_never_arrived_is_reported_not_guessed_at() {
    let mut all = group(&["anna", "ben"]);
    let outgoing = send(&mut all, 0, "hello");
    // The hand-over expired from the queue before ben collected it.
    assert_eq!(open(&mut all, 0, 1, &outgoing.message), Err(Error::UnknownSession));
}

#[test]
fn messages_up_to_the_size_limit_are_sent_and_larger_ones_refused() {
    let mut all = group(&["anna", "ben"]);
    let largest = "x".repeat(MAX_PLAINTEXT_BYTES);
    let outgoing = say(&mut all, 0, &largest);
    assert!(outgoing.message.len() <= 64 * 1024, "the largest message still fits the server's limit");
    ensure_sessions(&mut all, 0);
    assert_eq!(all[0].encrypt(CHANNEL, &vec![0u8; MAX_PLAINTEXT_BYTES + 1], NOW), Err(Error::TooLarge));
}

#[test]
fn junk_is_refused() {
    let mut all = group(&["anna", "ben"]);
    for junk in [&b""[..], b"\x00", b"\x01", b"\x02", b"\x03hello", b"\x01\x00garbage", b"\x02\x05ab"] {
        assert!(all[1].decrypt(CHANNEL, "anna", junk).is_err(), "{junk:?}");
    }
}

#[test]
fn a_member_who_missed_a_hand_over_can_ask_for_it_again() {
    let mut all = group(&["anna", "ben"]);
    // Anna's hand-over to Ben never arrives.
    let one = send(&mut all, 0, "one");
    assert_eq!(one.shares.len(), 1);
    assert_eq!(open(&mut all, 0, 1, &one.message), Err(Error::UnknownSession));

    // Ben starts a new private session with Anna and asks over it.
    let prekey = all[0].new_prekeys(1, false).one_time.remove(0);
    all[1].start_session("anna", &prekey).unwrap();
    let ask = all[1].ask_again(CHANNEL, "anna").unwrap();
    assert_eq!(all[0].decrypt(CHANNEL, "ben", &ask), Ok(Incoming::KeysWanted));

    // Anna's next message hands her ratchet over again, where it now stands.
    let two = send(&mut all, 0, "two");
    assert_eq!(two.shares.len(), 1);
    deliver_shares(&mut all, 0, &two);
    assert_eq!(open(&mut all, 0, 1, &two.message).as_deref(), Ok("two"));
    assert_eq!(open(&mut all, 0, 1, &one.message), Err(Error::AlreadyOpened), "what was missed stays missed");
    assert!(send(&mut all, 0, "three").shares.is_empty());
}

#[test]
fn asking_for_keys_gets_someone_outside_the_channel_nothing() {
    let mut all = group(&["anna", "ben", "cara"]);
    say(&mut all, 0, "to all three");
    set_roster(&mut all, 2, &["anna", "ben"]);
    say(&mut all[..2], 0, "to ben alone");

    let prekey = all[0].new_prekeys(1, false).one_time.remove(0);
    all[2].start_session("anna", &prekey).unwrap();
    let ask = all[2].ask_again(CHANNEL, "anna").unwrap();
    assert_eq!(all[0].decrypt(CHANNEL, "cara", &ask), Ok(Incoming::KeysWanted));
    let next = send(&mut all[..2], 0, "still to ben alone");
    assert!(next.shares.is_empty(), "nothing is handed to cara");
}

#[test]
fn sending_can_be_restarted_after_a_hand_over_was_lost() {
    let mut all = group(&["anna", "ben"]);
    let lost = send(&mut all, 0, "hello");
    assert_eq!(lost.shares.len(), 1);
    // The hand-over never reached ben, so anna starts again.
    all[0].restart_sending(CHANNEL);
    let again = say(&mut all, 0, "hello again");
    assert_eq!(again.shares.len(), 1, "a new ratchet, handed over afresh");
    assert_eq!(open(&mut all, 0, 1, &lost.message), Err(Error::UnknownSession));
}
