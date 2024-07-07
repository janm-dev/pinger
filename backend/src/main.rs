use std::{
	collections::HashMap,
	env,
	fmt::{Display, Formatter, Result as FmtResult},
	net::{Ipv6Addr, SocketAddrV6},
	ops::RangeInclusive,
	sync::{Arc, RwLock},
};

use axum::{
	extract::{ws::Message as WsMessage, State, WebSocketUpgrade},
	http::StatusCode,
	response::{Html, IntoResponse, Response},
	routing::get,
	Router,
};
use pinger::EncryptedPingInfo;
use rand::Rng;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
	net::TcpListener,
	select,
	sync::mpsc::{self, error::SendError as ChannelSendError, Sender},
};
use tracing::{debug, error, info, instrument};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod serde_support;
mod tests;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
struct Id(pub u16);

impl Display for Id {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		write!(f, "{}", self.0)
	}
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct PublicKey(#[serde(with = "serde_support::public_key")] pinger::PublicKey);

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

#[derive(Debug, Clone, Error)]
enum SendError {
	#[error("id {0} not found")]
	NoSuchId(Id),
	#[error(transparent)]
	ChannelError(#[from] ChannelSendError<ClientDownMessage>),
}

#[derive(Debug, Default)]
struct Ctx {
	connections: RwLock<HashMap<Id, Sender<ClientDownMessage>>>,
}

impl Ctx {
	async fn send(&self, to: Id, from: Id, msg: ClientClientMessage) -> Result<(), SendError> {
		let Some(dest) = self
			.connections
			.read()
			.expect("lock poisoned")
			.get(&to)
			.cloned()
		else {
			return Err(SendError::NoSuchId(to));
		};

		dest.send(ClientDownMessage::FromClient { from, msg })
			.await?;

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

	let app = Router::new()
		.route("/api", get(pinger))
		.route("/bug", get(bug_report))
		.with_state(ctx);

	let listener = TcpListener::bind(SocketAddrV6::new(
		Ipv6Addr::UNSPECIFIED,
		env::var("PORT")
			.ok()
			.and_then(|v| v.parse().ok())
			.unwrap_or(8000),
		0,
		0,
	))
	.await
	.unwrap();

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
						let _ = sender.send(ClientDownMessage::FromServer {
							msg: ServerClientMessage::Error {
								details: "error while receiving websocket message".to_string()
							}
						}).await;
						continue;
					};

					let WsMessage::Text(msg) = msg else {
						let _ = sender.send(ClientDownMessage::FromServer {
							msg: ServerClientMessage::Error {
								details: "unsupported message type, only text messages are supported".to_string()
							}
						}).await;
						continue;
					};

					let Ok(msg) = serde_json::from_str::<ClientUpMessage>(&msg) else {
						let _ = sender.send(ClientDownMessage::FromServer {
							msg: ServerClientMessage::Error {
								details: "could not deserialize message".to_string()
							}
						}).await;
						continue;
					};

					if let Err(e) = ctx.send(msg.to, id, msg.msg).await {
						match e {
							SendError::NoSuchId(id) => {
								let _ = sender.send(ClientDownMessage::FromServer {
									msg: ServerClientMessage::NoSuchId { id }
								}).await;
							},
							SendError::ChannelError(err) => error!("Error sending websocket message: {err}")
						}
					}
				},
				opt_msg = receiver.recv() => {
					let Some(msg) = opt_msg else {
						break;
					};

					if let Err(e) = ws.send(WsMessage::Text(serde_json::to_string(&msg).expect("failed to serialize message"))).await {
						debug!("error sending websocket message: {e}");
					}
				},
			}
		}

		ctx.drop_connection(id);
	})
}

#[instrument]
async fn bug_report() -> Html<&'static str> {
	Html(include_html!("bug"))
}

/// Include a generated minified html file as a `&'static str`. The file must
/// be generated by the build script and located in the `OUT_DIR` directory.
#[macro_export]
macro_rules! include_html {
	($name:literal) => {
		include_str!(concat!(env!("OUT_DIR"), concat!("/", $name, ".html")))
	};
}
