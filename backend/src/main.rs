//! The Pinger backend server

use std::{
	collections::HashMap,
	env,
	fmt::{Display, Formatter, Result as FmtResult},
	net::{Ipv6Addr, SocketAddrV6},
	ops::RangeInclusive,
	sync::{Arc, RwLock},
};

use axum::{
	Router,
	extract::{State, WebSocketUpgrade, ws::Message as WsMessage},
	http::{HeaderName, HeaderValue, StatusCode},
	response::{IntoResponse, Response},
	routing::get,
};
use pinger::EncryptedPingInfo;
use rand::Rng;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
	net::TcpListener,
	select,
	sync::mpsc::{self, Sender, error::SendError as ChannelSendError},
};
use tracing::{debug, error, info, instrument};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

mod serde_support;
mod tests;

/// Serve a static asset named `name` with a `Content-Type` of `type` over HTTP
macro_rules! serve_asset {
	($name:literal, $type:literal) => {{
		#[instrument(name = $name)]
		async fn serve() -> impl IntoResponse {
			(
				Response::builder()
					.header(
						HeaderName::from_static("content-type"),
						HeaderValue::from_static($type),
					)
					.body(())
					.unwrap(),
				include_bytes!(concat!("../assets/", $name)),
			)
		}

		get(serve)
	}};
}

/// Serve a minified HTML file named `name`
///
/// This file must have been minified in the build script
macro_rules! serve_html {
	($name:literal) => {{
		#[instrument(name = $name)]
		async fn serve_html() -> impl IntoResponse {
			(
				Response::builder()
					.header(
						HeaderName::from_static("content-type"),
						HeaderValue::from_static("text/html"),
					)
					.body(())
					.unwrap(),
				include_str!(concat!(env!("OUT_DIR"), concat!("/", $name, ".html"))),
			)
		}

		get(serve_html)
	}};
}

/// A Ping ID, a 2- or 3-digit number
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
struct Id(pub u16);

impl Display for Id {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		write!(f, "{}", self.0)
	}
}

/// A Pinger public key
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct PublicKey(#[serde(with = "serde_support::public_key")] pinger::PublicKey);

/// A websocket message sent by the server to a client
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "msg")]
enum ServerClientMessage {
	Connected { id: Id },
	NoSuchId { id: Id },
	Error { details: String },
}

/// A websocket message sent a client to another
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

/// An error when sending a Pinger websocket message
#[derive(Debug, Clone, Error)]
enum SendError {
	#[error("id {0} not found")]
	NoSuchId(Id),
	#[error(transparent)]
	ChannelError(#[from] ChannelSendError<ClientDownMessage>),
}

/// The server context containing a map of all connections
#[derive(Debug, Default)]
struct Ctx {
	connections: RwLock<HashMap<Id, Sender<ClientDownMessage>>>,
}

impl Ctx {
	/// Relay a client-client message
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

	/// Add a new connection to the map
	fn add_connection(&self, sender: Sender<ClientDownMessage>) -> Result<Id, impl IntoResponse> {
		let mut conns = self.connections.write().expect("lock poisoned");

		#[expect(clippy::question_mark, reason = "type inference")]
		let id = match Self::gen_id(&*conns) {
			Ok(id) => id,
			Err(err) => return Err(err),
		};

		conns.insert(id, sender);

		drop(conns);
		Ok(id)
	}

	/// Drop a connection from the map
	fn drop_connection(&self, id: Id) {
		drop(self.connections.write().expect("lock poisoned").remove(&id));
	}

	/// Generate a random, unused Ping ID
	fn gen_id<T>(connections: &HashMap<Id, T>) -> Result<Id, impl IntoResponse + use<T>> {
		const MAX_RETRIES: usize = 100;
		const ID_RANGE: RangeInclusive<u16> = 10..=999;

		let mut rng = rand::rng();
		let mut i = 0;
		let mut id = rng.random_range(ID_RANGE);

		while connections.contains_key(&Id(id)) {
			id = rng.random_range(ID_RANGE);
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
		.route("/", serve_html!("index"))
		.route("/api", get(pinger))
		.route("/bug", serve_html!("bug"))
		.route("/favicon.ico", serve_asset!("favicon.ico", "image/x-icon"))
		.route("/icon.svg", serve_asset!("icon.svg", "image/svg+xml"))
		.route("/icon.png", serve_asset!("icon.png", "image/png"))
		.route(
			"/icon-maskable.png",
			serve_asset!("icon-maskable.png", "image/png"),
		)
		.route(
			"/icon-maskable.svg",
			serve_asset!("icon-maskable.svg", "image/svg+xml"),
		)
		.route(
			"/icon-monochrome.svg",
			serve_asset!("icon-monochrome.svg", "image/svg+xml"),
		)
		.route(
			"/pinger.webmanifest",
			serve_asset!("pinger.webmanifest", "application/manifest+json"),
		)
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

/// The Pinger API server
#[instrument]
async fn pinger(State(ctx): State<Arc<Ctx>>, upgrade: WebSocketUpgrade) -> Response {
	const BUFFER_SIZE: usize = 2;

	let (sender, mut receiver) = mpsc::channel(BUFFER_SIZE);
	let id = match ctx.add_connection(sender.clone()) {
		Ok(id) => id,
		Err(e) => return e.into_response(),
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

					if let Err(e) = ws.send(WsMessage::Text(serde_json::to_string(&msg).expect("failed to serialize message").into())).await {
						debug!("error sending websocket message: {e}");
					}
				},
			}
		}

		ctx.drop_connection(id);
	})
}
