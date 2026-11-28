//! Channels: owner-signed records, invites, join requests and the roster.

mod common;

use axum::http::StatusCode;
use common::*;
use deaddrop_core::encoding::{random_id, unb64};
use deaddrop_core::keys;
use deaddrop_core::records::{ChannelRecord, Leave, Roster, RosterMember, Signed, open_leave};
use serde_json::json;

#[tokio::test]
async fn a_channel_is_what_its_owner_signed() {
    let server = Server::new().await;
    let (owner, other) = (server.phone().await, server.phone().await);
    let channel = server.channel(&owner, 86_400).await;

    // Anyone on the server who knows the ID can fetch the record and check it themselves.
    let (status, shown) = server.as_phone(&other, GET, &format!("/v1/channels/{channel}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(shown["member"], json!(false));
    let owner_key = unb64(shown["owner_key"].as_str().unwrap()).unwrap();
    let signed: Signed = serde_json::from_value(shown["channel"].clone()).unwrap();
    let record: ChannelRecord = signed.open(&owner_key).unwrap();
    assert_eq!(record.max_age, 86_400);
    assert_eq!(keys::device_id(&owner_key), record.owner, "the owner ID in the record pins the owner's key");

    let (_, mine) = server.as_phone(&owner, GET, "/v1/channels", None).await;
    assert_eq!(mine["channels"], json!([{ "id": channel, "owner": true, "version": 1, "roster_version": 1 }]));
    let (status, _) = server.as_phone(&other, GET, &format!("/v1/channels/{}", random_id()), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn channel_records_must_be_signed_by_the_device_creating_them() {
    let server = Server::new().await;
    let (owner, other) = (server.phone().await, server.phone().await);
    let id = random_id();
    let good_roster = owner.device.seal(&roster(&id, 1, &[(&owner, "Owner")]));
    let create = |channel: Signed, roster: Signed| {
        let body = json!({ "channel": channel, "roster": roster });
        async { server.as_phone(&owner, POST, "/v1/channels", Some(body)).await.0 }
    };

    // Signed by somebody else.
    let forged = other.device.seal(&channel_record(&owner, &id, 3600, 1));
    assert_eq!(create(forged, good_roster.clone()).await, StatusCode::BAD_REQUEST);
    // Signed by the caller but naming another owner.
    let misnamed = owner.device.seal(&channel_record(&other, &id, 3600, 1));
    assert_eq!(create(misnamed, good_roster.clone()).await, StatusCode::BAD_REQUEST);
    // Altered after signing.
    let mut altered = owner.device.seal(&channel_record(&owner, &id, 3600, 1));
    altered.record = altered.record.replace("3600", "7200");
    assert_eq!(create(altered, good_roster.clone()).await, StatusCode::BAD_REQUEST);
    // No maximum message age.
    let ageless = owner.device.seal(&channel_record(&owner, &id, 0, 1));
    assert_eq!(create(ageless, good_roster.clone()).await, StatusCode::BAD_REQUEST);
    // A roster for a different channel, or with extra people in it.
    let good = owner.device.seal(&channel_record(&owner, &id, 3600, 1));
    let elsewhere = owner.device.seal(&roster(&random_id(), 1, &[(&owner, "Owner")]));
    assert_eq!(create(good.clone(), elsewhere).await, StatusCode::BAD_REQUEST);
    let crowded = owner.device.seal(&roster(&id, 1, &[(&owner, "Owner"), (&other, "Other")]));
    assert_eq!(create(good.clone(), crowded).await, StatusCode::BAD_REQUEST);
    assert_eq!(server.rows("channels").await, 0);

    assert_eq!(create(good.clone(), good_roster.clone()).await, StatusCode::CREATED);
    assert_eq!(create(good, good_roster).await, StatusCode::CONFLICT, "channel IDs cannot be reused");
}

#[tokio::test]
async fn joining_needs_an_invite_and_the_owners_signature() {
    let server = Server::new().await;
    let (owner, joiner) = (server.phone().await, server.phone().await);
    let channel = server.channel(&owner, 3600).await;
    let requests_path = format!("/v1/channels/{channel}/join-requests");

    // No invite, a made-up invite, an invite for another channel: all refused alike.
    let other_channel = server.channel(&owner, 3600).await;
    let (_, reply) =
        server.as_phone(&owner, POST, &format!("/v1/channels/{other_channel}/invites"), Some(json!({}))).await;
    for token in [json!(""), json!("made-up"), reply["token"].clone()] {
        let body = json!({ "token": token, "name": "Joiner" });
        let (status, _) = server.as_phone(&joiner, POST, &requests_path, Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    // Only the owner can make invites.
    let (status, _) = server.as_phone(&joiner, POST, &format!("/v1/channels/{channel}/invites"), Some(json!({}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    server.ask_to_join(&owner, &channel, &joiner, "Joiner").await;
    let (_, mine) = server.as_phone(&joiner, GET, "/v1/channels", None).await;
    assert_eq!(mine, json!({ "channels": [], "requests": [channel] }), "asking is not joining");
    let (status, _) = server.as_phone(&joiner, GET, &format!("/v1/channels/{channel}/members"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The owner sees the request with the joiner's keys, and only the owner does.
    let (status, _) = server.as_phone(&joiner, GET, &requests_path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, list) = server.as_phone(&owner, GET, &requests_path, None).await;
    let request = &list["requests"][0];
    assert_eq!((&request["device_id"], &request["name"]), (&json!(joiner.id), &json!("Joiner")));
    let key = unb64(request["signing_key"].as_str().unwrap()).unwrap();
    assert_eq!(keys::device_id(&key), joiner.id);

    // Signing a roster with the joiner in it is what lets them in.
    let new_roster = roster(&channel, 2, &[(&owner, "Owner"), (&joiner, "Joiner")]);
    assert_eq!(server.put_roster(&owner, &channel, &new_roster).await.0, StatusCode::NO_CONTENT);
    assert_eq!(server.rows("join_requests").await, 0);

    let (status, view) = server.as_phone(&joiner, GET, &format!("/v1/channels/{channel}/members"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, shown) = server.as_phone(&joiner, GET, &format!("/v1/channels/{channel}"), None).await;
    let owner_key = unb64(shown["owner_key"].as_str().unwrap()).unwrap();
    let signed: Signed = serde_json::from_value(view["roster"].clone()).unwrap();
    assert_eq!(signed.open::<Roster>(&owner_key).unwrap(), new_roster);
    assert_eq!(view["keys"].as_array().unwrap().len(), 2);
    assert_eq!(view["left"], json!([]));

    // A member cannot ask again.
    let (_, reply) = server.as_phone(&owner, POST, &format!("/v1/channels/{channel}/invites"), Some(json!({}))).await;
    let body = json!({ "token": reply["token"], "name": "Joiner" });
    assert_eq!(server.as_phone(&joiner, POST, &requests_path, Some(body)).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn only_an_owner_signed_newer_roster_is_accepted() {
    let server = Server::new().await;
    let (owner, member, stranger) = (server.phone().await, server.phone().await, server.phone().await);
    let channel = server.channel_with(&owner, 3600, &[(&member, "Member")]).await;
    let path = format!("/v1/channels/{channel}/roster");
    let both = [(&owner, "Owner"), (&member, "Member")];

    // A member cannot publish a roster, even one they signed themselves.
    let body = json!({ "roster": member.device.seal(&roster(&channel, 3, &both)) });
    assert_eq!(server.as_phone(&member, PUT, &path, Some(body)).await.0, StatusCode::FORBIDDEN);
    // The owner cannot publish one somebody else signed.
    let body = json!({ "roster": member.device.seal(&roster(&channel, 3, &both)) });
    assert_eq!(server.as_phone(&owner, PUT, &path, Some(body)).await.0, StatusCode::BAD_REQUEST);
    // The version must go up.
    assert_eq!(server.put_roster(&owner, &channel, &roster(&channel, 2, &both)).await.0, StatusCode::CONFLICT);
    // Nobody is added who has not asked.
    let with_stranger = roster(&channel, 3, &[(&owner, "Owner"), (&member, "Member"), (&stranger, "Stranger")]);
    assert_eq!(server.put_roster(&owner, &channel, &with_stranger).await.0, StatusCode::CONFLICT);
    // The owner stays in, and no two members share a name (whatever the capitals).
    assert_eq!(
        server.put_roster(&owner, &channel, &roster(&channel, 3, &[(&member, "Member")])).await.0,
        StatusCode::BAD_REQUEST
    );
    let same_name = roster(&channel, 3, &[(&owner, "Sam"), (&member, "SAM")]);
    assert_eq!(server.put_roster(&owner, &channel, &same_name).await.0, StatusCode::BAD_REQUEST);

    let (_, mine) = server.as_phone(&owner, GET, "/v1/channels", None).await;
    assert_eq!(mine["channels"][0]["roster_version"], json!(2), "none of that changed anything");
    assert_eq!(server.put_roster(&owner, &channel, &roster(&channel, 3, &both)).await.0, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn the_owner_can_decline_a_request() {
    let server = Server::new().await;
    let (owner, joiner) = (server.phone().await, server.phone().await);
    let channel = server.channel(&owner, 3600).await;
    server.ask_to_join(&owner, &channel, &joiner, "Joiner").await;
    let path = format!("/v1/channels/{channel}/join-requests/{}", joiner.id);
    assert_eq!(server.as_phone(&joiner, DELETE, &path, None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(server.as_phone(&owner, DELETE, &path, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(server.as_phone(&owner, DELETE, &path, None).await.0, StatusCode::NOT_FOUND);
    let (_, mine) = server.as_phone(&joiner, GET, "/v1/channels", None).await;
    assert_eq!(mine["requests"], json!([]));
}

/// What a phone signs to say it is leaving a channel.
fn leave_of(phone: &Phone, channel: &str) -> Leave {
    Leave { v: 1, channel: channel.to_owned(), device: phone.id.clone(), roster: 2, issued: 1_800_000_000 }
}

#[tokio::test]
async fn a_member_who_leaves_signs_for_it_and_whether_to_drop_them_is_the_owners_decision() {
    let server = Server::new().await;
    let (owner, member, other) = (server.phone().await, server.phone().await, server.phone().await);
    let channel = server.channel_with(&owner, 3600, &[(&member, "Member")]).await;
    let elsewhere = server.channel(&owner, 3600).await;
    let path = format!("/v1/channels/{channel}/members/me");
    let (server_ref, path_ref, member_ref) = (&server, &path, &member);
    let leave = |signed: Signed| async move {
        server_ref.as_phone(member_ref, DELETE, path_ref, Some(json!({ "leave": signed }))).await.0
    };

    assert_eq!(server.as_phone(&owner, DELETE, &path, None).await.0, StatusCode::CONFLICT, "the owner cannot leave");
    // What is kept has to be the leaver's own signed word about this channel.
    assert_eq!(leave(owner.device.seal(&leave_of(&member, &channel))).await, StatusCode::BAD_REQUEST);
    assert_eq!(leave(member.device.seal(&leave_of(&owner, &channel))).await, StatusCode::BAD_REQUEST);
    assert_eq!(leave(member.device.seal(&leave_of(&member, &elsewhere))).await, StatusCode::BAD_REQUEST);
    assert_eq!(server.as_phone(&member, DELETE, &path, Some(json!({ "leave": 7 }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(server.rows("leaves").await, 0);

    assert_eq!(leave(member.device.seal(&leave_of(&member, &channel))).await, StatusCode::NO_CONTENT);
    assert_eq!(server.as_phone(&member, DELETE, &path, None).await.0, StatusCode::NOT_FOUND);

    // Remaining members are told who has left, and handed what the leaver signed so
    // that they can check it for themselves.
    let members = format!("/v1/channels/{channel}/members");
    let (_, view) = server.as_phone(&owner, GET, &members, None).await;
    assert_eq!(view["left"], json!([member.id]));
    assert_eq!(view["leaves"][0]["device_id"], json!(member.id));
    let key = unb64(view["leaves"][0]["signing_key"].as_str().unwrap()).unwrap();
    let signed: Signed = serde_json::from_value(view["leaves"][0]["leave"].clone()).unwrap();
    let listed = RosterMember { device: member.id.clone(), name: "Member".into(), since: 2 };
    assert!(open_leave(&channel, &listed, &key, &signed).is_ok());

    // The owner may keep them in the list. That does not start delivery to them again.
    let both = roster(&channel, 3, &[(&owner, "Owner"), (&member, "Member")]);
    assert_eq!(server.put_roster(&owner, &channel, &both).await.0, StatusCode::NO_CONTENT);
    let (_, view) = server.as_phone(&owner, GET, &members, None).await;
    assert_eq!((&view["left"], view["leaves"].as_array().unwrap().len()), (&json!([member.id]), 1));
    assert_eq!(server.send(&owner, &channel, "anyone?").await.1["recipients"], json!(0));

    // Nobody new gets in that way: a list may hold those in the last one and those who asked.
    let stranger = roster(&channel, 4, &[(&owner, "Owner"), (&member, "Member"), (&other, "Other")]);
    assert_eq!(server.put_roster(&owner, &channel, &stranger).await.0, StatusCode::CONFLICT);

    // Dropping them works, and what they signed has then done its job.
    let alone = roster(&channel, 4, &[(&owner, "Owner")]);
    assert_eq!(server.put_roster(&owner, &channel, &alone).await.0, StatusCode::NO_CONTENT);
    assert_eq!((server.rows("members").await, server.rows("leaves").await), (2, 0));
}

#[tokio::test]
async fn a_phone_from_before_signed_leaves_can_still_leave() {
    let server = Server::new().await;
    let (owner, member) = (server.phone().await, server.phone().await);
    let channel = server.channel_with(&owner, 3600, &[(&member, "Member")]).await;

    // Versions up to 0.1.1 send nothing with the request.
    let path = format!("/v1/channels/{channel}/members/me");
    assert_eq!(server.as_phone(&member, DELETE, &path, None).await.0, StatusCode::NO_CONTENT);
    let (_, view) = server.as_phone(&owner, GET, &format!("/v1/channels/{channel}/members"), None).await;
    assert_eq!((&view["left"], &view["leaves"]), (&json!([member.id]), &json!([])));
}

#[tokio::test]
async fn a_device_that_deletes_itself_leaves_its_signed_word_behind() {
    let server = Server::new().await;
    let (owner, member) = (server.phone().await, server.phone().await);
    let channel = server.channel_with(&owner, 3600, &[(&member, "Member")]).await;
    let elsewhere = server.channel(&owner, 3600).await;

    let leaves = json!({ "leaves": [
        member.device.seal(&leave_of(&member, &channel)),
        // Not kept: a channel it is not in, and one signed by somebody else.
        member.device.seal(&leave_of(&member, &elsewhere)),
        owner.device.seal(&leave_of(&member, &channel)),
    ] });
    assert_eq!(server.as_phone(&member, DELETE, "/v1/devices/me", Some(leaves)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(server.rows("leaves").await, 1);

    // The device is gone, keys and all, but its word that it left is there to check.
    let (_, view) = server.as_phone(&owner, GET, &format!("/v1/channels/{channel}/members"), None).await;
    assert_eq!(view["keys"].as_array().unwrap().len(), 1);
    let key = unb64(view["leaves"][0]["signing_key"].as_str().unwrap()).unwrap();
    let signed: Signed = serde_json::from_value(view["leaves"][0]["leave"].clone()).unwrap();
    let listed = RosterMember { device: member.id.clone(), name: "Member".into(), since: 2 };
    assert!(open_leave(&channel, &listed, &key, &signed).is_ok());

    // The owner's list may go on naming it until the owner signs one without it.
    let both = roster(&channel, 3, &[(&owner, "Owner"), (&member, "Member")]);
    assert_eq!(server.put_roster(&owner, &channel, &both).await.0, StatusCode::NO_CONTENT);
    assert_eq!(server.rows("leaves").await, 1);
    let alone = roster(&channel, 4, &[(&owner, "Owner")]);
    assert_eq!(server.put_roster(&owner, &channel, &alone).await.0, StatusCode::NO_CONTENT);
    assert_eq!(server.rows("leaves").await, 0);
}

#[tokio::test]
async fn only_the_owner_changes_or_deletes_a_channel() {
    let server = Server::new().await;
    let (owner, member) = (server.phone().await, server.phone().await);
    let channel = server.channel_with(&owner, 3600, &[(&member, "Member")]).await;
    let path = format!("/v1/channels/{channel}");
    let (server_ref, path_ref) = (&server, &path);
    let update = |phone, signer: &Phone, version| {
        let body = json!({ "channel": signer.device.seal(&channel_record(&owner, &channel, 600, version)) });
        async move { server_ref.as_phone(phone, PUT, path_ref, Some(body)).await.0 }
    };

    assert_eq!(update(&member, &member, 2).await, StatusCode::FORBIDDEN);
    assert_eq!(update(&owner, &member, 2).await, StatusCode::BAD_REQUEST, "signed by the wrong device");
    assert_eq!(update(&owner, &owner, 1).await, StatusCode::CONFLICT, "the version must go up");
    assert_eq!(update(&owner, &owner, 2).await, StatusCode::NO_CONTENT);
    let (_, shown) = server.as_phone(&member, GET, &path, None).await;
    let signed: Signed = serde_json::from_value(shown["channel"].clone()).unwrap();
    assert_eq!(signed.open::<ChannelRecord>(&owner.device.signing_key_der()).unwrap().max_age, 600);

    assert_eq!(server.as_phone(&member, DELETE, &path, None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(server.as_phone(&owner, DELETE, &path, None).await.0, StatusCode::NO_CONTENT);
    for table in ["channels", "members", "tokens"] {
        assert_eq!(server.rows(table).await, 0, "{table}");
    }
}
