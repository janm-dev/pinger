use std::{
	collections::HashMap,
	ops::RangeInclusive,
	sync::{Arc, RwLock},
};

use axum::{
	extract::{ws::Message as WsMessage, State, WebSocketUpgrade},
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::get,
	Router,
};
use pinger::EncryptedPingInfo;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::{
	net::TcpListener,
	select,
	sync::mpsc::{self, Sender},
};
use tracing::{info, instrument};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
struct Id(pub u16);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Token(Vec<u8>);

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct PublicKey(#[serde(with = "serde_public_key")] pinger::PublicKey);

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Message {
	ServerClient(ServerClientMessage),
	ClientClientIncoming { to: Id, msg: ClientClientMessage },
	ClientClientOutgoing { from: Id, msg: ClientClientMessage },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "msg")]
enum ServerClientMessage {
	Connected { id: Id },
	NoSuchId { id: Id },
	Error { details: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "msg")]
enum ClientClientMessage {
	PingRequest { key: PublicKey },
	AcceptPing { key: PublicKey },
	RejectPing,
	Ping { info: EncryptedPingInfo },
	PingAck,
}

/// A message sent by a client to the server or via the server to another client
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ClientUpMessage {
	to: Id,
	#[serde(flatten)]
	msg: ClientClientMessage,
}

/// A message sent by the server to a client, possibly on behalf of another
/// client
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum ClientDownMessage {
	FromClient {
		from: Id,
		#[serde(flatten)]
		msg: ClientClientMessage,
	},
	FromServer {
		#[serde(flatten)]
		msg: ServerClientMessage,
	},
}

#[derive(Debug, Default)]
struct Ctx {
	connections: RwLock<HashMap<Id, Sender<ClientDownMessage>>>,
}

impl Ctx {
	async fn send(&self, to: Id, from: Id, msg: ClientClientMessage) -> Result<(), &'static str> {
		let Some(dest) = self
			.connections
			.read()
			.expect("lock poisoned")
			.get(&to)
			.cloned()
		else {
			return Err("no such id");
		};

		dest.send(ClientDownMessage::FromClient { from, msg })
			.await
			.map_err(|_| "sending failed (TODO)")?;

		Ok(())
	}

	fn add_connection(&self, sender: Sender<ClientDownMessage>) -> Result<Id, Response> {
		let mut conns = self.connections.write().expect("lock poisoned");

		let id = Self::gen_id(&*conns).map_err(IntoResponse::into_response)?;
		conns.insert(id, sender);
		Ok(id)
	}

	fn drop_connection(&self, id: Id) {
		drop(self.connections.write().expect("lock poisoned").remove(&id));
	}

	fn gen_id<T>(connections: &HashMap<Id, T>) -> Result<Id, impl IntoResponse> {
		const MAX_RETRIES: usize = 100;
		const ID_RANGE: RangeInclusive<u16> = 10..=999;

		let mut rng = rand::thread_rng();
		let mut i = 0;
		let mut id = rng.gen_range(ID_RANGE);

		while connections.contains_key(&Id(id)) {
			id = rng.gen_range(ID_RANGE);
			i += 1;

			if i > MAX_RETRIES {
				return Err(StatusCode::SERVICE_UNAVAILABLE);
			}
		}

		Ok(Id(id))
	}
}

#[tokio::main]
async fn main() {
	tracing_subscriber::registry()
		.with(fmt::layer())
		.with(EnvFilter::from_env("PINGER_LOG"))
		.init();

	let ctx = Arc::default();

	let app = Router::new().route("/", get(pinger)).with_state(ctx);

	let listener = TcpListener::bind("0.0.0.0:8000").await.unwrap();

	info!("Pinger backend starting");
	axum::serve(listener, app).await.unwrap();
}

#[instrument]
async fn pinger(State(ctx): State<Arc<Ctx>>, upgrade: WebSocketUpgrade) -> Response {
	const BUFFER_SIZE: usize = 2;

	let (sender, mut receiver) = mpsc::channel(BUFFER_SIZE);
	let id = match ctx.add_connection(sender.clone()) {
		Ok(id) => id,
		Err(e) => return e,
	};

	sender
		.try_send(ClientDownMessage::FromServer {
			msg: ServerClientMessage::Connected { id },
		})
		.expect("empty buffer is full");

	upgrade.on_upgrade(move |mut ws| async move {
		loop {
			select! {
				opt_msg = ws.recv() => {
					let Some(msg) = opt_msg else {
						break;
					};

					let Ok(msg) = msg else {
						let _ = sender.send(ClientDownMessage::FromServer{ msg: ServerClientMessage::Error{ details: "error while receiving websocket message".to_string() } }).await;
						continue;
					};

					let WsMessage::Text(msg) = msg else {
						let _ = sender.send(ClientDownMessage::FromServer{ msg: ServerClientMessage::Error{ details: "unsupported message type, only text messages are supported".to_string() } }).await;
						continue;
					};

					let Ok(msg) = serde_json::from_str::<ClientUpMessage>(&msg) else {
						let _ = sender.send(ClientDownMessage::FromServer{ msg: ServerClientMessage::Error{ details: "could not deserialize message".to_string() } }).await;
						continue;
					};

					if let Err(e) = ctx.send(msg.to, id, msg.msg).await {
						todo!("error handling of {e}")
					}
				},
				opt_msg = receiver.recv() => {
					let Some(msg) = opt_msg else {
						break;
					};

					if let Err(e) = ws.send(WsMessage::Text(serde_json::to_string(&msg).expect("failed to serialize message"))).await {
						todo!("error handling of {e}");
					}
				},
			}
		}

		ctx.drop_connection(id);
	})
}

mod serde_public_key {
	use core::{
		fmt::{Formatter, Result as FmtResult},
		str,
	};

	use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
	use serde::{
		de::{Error as DeError, Expected, Unexpected},
		ser::Error as SerError,
		Deserialize, Deserializer, Serializer,
	};

	pub fn serialize<S: Serializer>(val: &pinger::PublicKey, ser: S) -> Result<S::Ok, S::Error> {
		let mut buf = [0u8; 43];

		let n = URL_SAFE_NO_PAD
			.encode_slice(val, &mut buf)
			.map_err(|_| SerError::custom("failed to base64-encode"))?;

		ser.serialize_str(
			str::from_utf8(&buf[..n])
				.map_err(|_| SerError::custom("failed to create base64 string"))?,
		)
	}

	pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<pinger::PublicKey, D::Error> {
		struct Expected32ByteSlice;

		impl Expected for Expected32ByteSlice {
			fn fmt(&self, f: &mut Formatter) -> FmtResult {
				write!(f, "a base64-encoded 32-byte slice")
			}
		}

		let str = <&str as Deserialize>::deserialize(de)?;
		let mut buf = [0u8; 32];

		let n = URL_SAFE_NO_PAD
			.decode_slice(str, &mut buf)
			.map_err(|_| DeError::invalid_value(Unexpected::Str(str), &Expected32ByteSlice))?;

		if n != 32 {
			return Err(DeError::invalid_length(n, &Expected32ByteSlice));
		}

		Ok(pinger::PublicKey::from(buf))
	}
}

#[cfg(test)]
mod tests {
	use std::error::Error;

	use pinger::{Degrees, EphemeralSecret, Meters, PingInfo, PublicKey, Timestamp};
	use regex::Regex;

	use super::*;

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
}
