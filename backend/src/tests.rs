#![cfg(test)]

use std::error::Error;

use pinger::{Degrees, EphemeralSecret, Meters, PingInfo, PublicKey, Timestamp};
use regex::Regex;

use crate::*;

#[test]
fn ser_down() -> Result<(), Box<dyn Error>> {
	let alices_secret = EphemeralSecret::random();
	let bobs_secret = EphemeralSecret::random();
	let alices_public_key = PublicKey::from(&alices_secret);
	let bobs_public_key = PublicKey::from(&bobs_secret);
	let alices_shared_secret = alices_secret.diffie_hellman(&bobs_public_key);
	let bobs_shared_secret = bobs_secret.diffie_hellman(&alices_public_key);
	let apk_str = serde_json::to_string(&crate::PublicKey(alices_public_key))?;
	let bpk_str = serde_json::to_string(&crate::PublicKey(bobs_public_key))?;

	assert_eq!(
		alices_shared_secret.as_bytes(),
		bobs_shared_secret.as_bytes()
	);

	let ping_info = PingInfo {
		ts: Timestamp(0x1234567890),
		lat: Degrees(1.2),
		lon: Degrees(3.4),
		alt: Meters(5.6),
		err: Meters(7.8),
	};

	let info = ping_info.encrypt(alices_shared_secret).unwrap();
	let info_str = serde_json::to_string(&info)?;

	assert_eq!(
		ping_info,
		PingInfo::decrypt(info, bobs_shared_secret).unwrap()
	);

	assert!(Regex::new(r#""[A-Za-z0-9\-_]{86}""#)?.is_match(&info_str));
	assert!(Regex::new(r#""[A-Za-z0-9\-_]{43}""#)?.is_match(&apk_str));
	assert!(Regex::new(r#""[A-Za-z0-9\-_]{43}""#)?.is_match(&bpk_str));

	assert_eq!(
		serde_json::to_string(&ClientDownMessage::FromServer {
			msg: ServerClientMessage::Connected { id: Id(42) }
		})?,
		r#"{"msg":"connected","id":42}"#
	);

	assert_eq!(
		serde_json::to_string(&ClientDownMessage::FromServer {
			msg: ServerClientMessage::Error {
				details: "error details".to_string()
			}
		})?,
		r#"{"msg":"error","details":"error details"}"#
	);

	assert_eq!(
		serde_json::to_string(&ClientDownMessage::FromServer {
			msg: ServerClientMessage::NoSuchId { id: Id(42) }
		})?,
		r#"{"msg":"no_such_id","id":42}"#
	);

	assert_eq!(
		serde_json::to_string(&ClientDownMessage::FromClient {
			from: Id(42),
			msg: ClientClientMessage::PingAck
		})?,
		r#"{"from":42,"msg":"ping_ack"}"#
	);

	assert_eq!(
		serde_json::to_string(&ClientDownMessage::FromClient {
			from: Id(42),
			msg: ClientClientMessage::Ping { info }
		})?,
		format!(r#"{{"from":42,"msg":"ping","info":{}}}"#, info_str)
	);

	assert_eq!(
		serde_json::to_string(&ClientDownMessage::FromClient {
			from: Id(42),
			msg: ClientClientMessage::AcceptPing {
				key: PublicKey(alices_public_key)
			}
		})?,
		format!(r#"{{"from":42,"msg":"accept_ping","key":{apk_str}}}"#)
	);

	assert_eq!(
		serde_json::to_string(&ClientDownMessage::FromClient {
			from: Id(42),
			msg: ClientClientMessage::PingRequest {
				key: PublicKey(bobs_public_key)
			}
		})?,
		format!(r#"{{"from":42,"msg":"ping_request","key":{bpk_str}}}"#)
	);

	assert_eq!(
		serde_json::to_string(&ClientDownMessage::FromClient {
			from: Id(42),
			msg: ClientClientMessage::RejectPing
		})?,
		r#"{"from":42,"msg":"reject_ping"}"#
	);

	Ok(())
}

#[test]
fn ser_up() -> Result<(), Box<dyn Error>> {
	let alices_secret = EphemeralSecret::random();
	let bobs_secret = EphemeralSecret::random();
	let alices_public_key = PublicKey::from(&alices_secret);
	let bobs_public_key = PublicKey::from(&bobs_secret);
	let alices_shared_secret = alices_secret.diffie_hellman(&bobs_public_key);
	let bobs_shared_secret = bobs_secret.diffie_hellman(&alices_public_key);
	let apk_str = serde_json::to_string(&crate::PublicKey(alices_public_key))?;
	let bpk_str = serde_json::to_string(&crate::PublicKey(bobs_public_key))?;

	assert_eq!(
		alices_shared_secret.as_bytes(),
		bobs_shared_secret.as_bytes()
	);

	let ping_info = PingInfo {
		ts: Timestamp(0x1234567890),
		lat: Degrees(1.2),
		lon: Degrees(3.4),
		alt: Meters(5.6),
		err: Meters(7.8),
	};

	let info = ping_info.encrypt(alices_shared_secret).unwrap();
	let info_str = serde_json::to_string(&info)?;

	assert_eq!(
		ping_info,
		PingInfo::decrypt(info, bobs_shared_secret).unwrap()
	);

	assert!(Regex::new(r#""[A-Za-z0-9\-_]{86}""#)?.is_match(&info_str));

	assert_eq!(
		serde_json::to_string(&ClientUpMessage {
			to: Id(42),
			msg: ClientClientMessage::RejectPing
		})?,
		r#"{"to":42,"msg":"reject_ping"}"#
	);

	assert_eq!(
		serde_json::to_string(&ClientUpMessage {
			to: Id(42),
			msg: ClientClientMessage::AcceptPing {
				key: PublicKey(alices_public_key)
			}
		})?,
		format!(r#"{{"to":42,"msg":"accept_ping","key":{apk_str}}}"#)
	);

	assert_eq!(
		serde_json::to_string(&ClientUpMessage {
			to: Id(42),
			msg: ClientClientMessage::Ping { info }
		})?,
		format!(r#"{{"to":42,"msg":"ping","info":{info_str}}}"#)
	);

	assert_eq!(
		serde_json::to_string(&ClientUpMessage {
			to: Id(42),
			msg: ClientClientMessage::PingAck
		})?,
		r#"{"to":42,"msg":"ping_ack"}"#
	);

	assert_eq!(
		serde_json::to_string(&ClientUpMessage {
			to: Id(42),
			msg: ClientClientMessage::PingRequest {
				key: PublicKey(bobs_public_key)
			}
		})?,
		format!(r#"{{"to":42,"msg":"ping_request","key":{bpk_str}}}"#)
	);

	Ok(())
}
