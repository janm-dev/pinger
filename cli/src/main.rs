//! A command-line interface for Pinger, intended mainly for testing the server
//!
//! Run with `./executable-name [SERVER]`, where `[SERVER]` is the optional
//! websocket URI of the Pinger API (`wss://pinger.janm.dev/api` by default if
//! not specified)

use std::{
	collections::HashMap,
	env,
	fmt::{Debug, Display, Error as FmtError, Formatter, Result as FmtResult},
	io, mem,
	process::ExitCode,
	sync::{Condvar, Mutex},
	thread,
	time::{Duration, SystemTime},
};

use base64::engine::{Engine, general_purpose::URL_SAFE_NO_PAD};
use colored::Colorize;
use derive_more::Display;
use futures_util::{Sink, SinkExt, StreamExt};
use inquire::{
	CustomType,
	validator::{ErrorMessage, Validation},
};
use pinger::{Degrees, EphemeralSecret, Meters, PingInfo, SharedKey, Timestamp};
use serde::{Deserialize, Serialize};
use tokio::{select, signal, sync::mpsc};
use tokio_tungstenite::tungstenite::Message;

const DEFAULT_URL: &str = "wss://pinger.janm.dev/api";

/// A Ping ID, a 2- or 3-digit number
#[derive(Clone, Copy, Debug, Display, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[display("{_0}")]
struct Id(pub u16);

/// A public key for the Pinger key exchange
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct PublicKey(#[serde(with = "serde_public_key")] pinger::PublicKey);

impl Display for PublicKey {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		let mut buf = [0u8; 43];

		let n = URL_SAFE_NO_PAD
			.encode_slice(self.0, &mut buf)
			.map_err(|_| FmtError)?;

		f.write_fmt(format_args!(
			"\"{}\"",
			std::str::from_utf8(&buf[..n]).map_err(|_| FmtError)?
		))
	}
}

/// Encrypted Ping info
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(transparent)]
struct EncryptedPingInfo(pinger::EncryptedPingInfo);

impl Display for EncryptedPingInfo {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		let mut buf = [0u8; 86];

		let n = URL_SAFE_NO_PAD
			.encode_slice(self.0, &mut buf)
			.map_err(|_| FmtError)?;

		f.write_fmt(format_args!(
			"\"{}\"",
			std::str::from_utf8(&buf[..n]).map_err(|_| FmtError)?
		))
	}
}

/// A message sent from the server to a client
#[derive(Clone, Debug, Display, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "msg")]
enum ServerClientMessage {
	#[display("Connected as {id}")]
	Connected { id: Id },
	#[display("Id {id} not found")]
	NoSuchId { id: Id },
	#[display("Error: {details}")]
	Error { details: String },
}

/// A message sent from one client to another
#[derive(Clone, Debug, Display, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "msg")]
enum ClientClientMessage {
	#[display("Ping requested with key {key}")]
	PingRequest { key: PublicKey },
	#[display("Ping accepted with key {key}")]
	AcceptPing { key: PublicKey },
	#[display("Ping rejected")]
	RejectPing,
	#[display("Ping received (ping info is encrypted)")]
	Ping { info: EncryptedPingInfo },
	#[display("Ping acknowledged")]
	PingAck,
}

/// A message sent by a client to the server or via the server to another client
#[derive(Clone, Debug, Display, Serialize, Deserialize)]
#[display("{msg} to {to}")]
struct ClientUpMessage {
	to: Id,
	#[serde(flatten)]
	msg: ClientClientMessage,
}

/// A message sent by the server to a client, possibly on behalf of another
/// client
#[derive(Clone, Debug, Display, Serialize, Deserialize)]
#[serde(untagged)]
enum ClientDownMessage {
	#[display("{msg} by {from}")]
	FromClient {
		from: Id,
		#[serde(flatten)]
		msg: ClientClientMessage,
	},
	#[display("{msg}")]
	FromServer {
		#[serde(flatten)]
		msg: ServerClientMessage,
	},
}

/// Implement `Debug` and `Display` for the wrapped value by writing `...`
struct OpaqueFmt<T>(pub T);

impl<T> Debug for OpaqueFmt<T> {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		f.write_str("...")
	}
}

impl<T> Display for OpaqueFmt<T> {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		f.write_str("...")
	}
}

/// An incoming Ping info exchange, either waiting for a user decision or the
/// encrypted Ping info
#[derive(Debug)]
enum IncomingExchange {
	Deciding(PublicKey),
	AwaitingPing(SharedKey),
}

/// The outgoing Ping info exchange, either none (if the user hasn't Pinged
/// anyone yet), waiting for the remote user's decision, or waiting for the
/// remote user's acknowledgment
#[derive(Debug, Default)]
enum OutgoingExchange {
	#[default]
	None,
	AwaitingDecision(Id, PingInfo, OpaqueFmt<EphemeralSecret>),
	AwaitingAck(Id),
}

impl OutgoingExchange {
	/// Check if there is no outgoing Ping info exchange
	const fn is_none(&self) -> bool {
		matches!(self, Self::None)
	}
}

/// An open server connection
#[derive(Debug, Default)]
struct Connection {
	/// The outgoing Ping info exchange
	outgoing: OutgoingExchange,
	/// The incoming Ping info exchanges from each ID
	incoming: HashMap<Id, IncomingExchange>,
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> ExitCode {
	let url = env::args()
		.nth(1)
		.unwrap_or_else(|| DEFAULT_URL.to_string());

	let (mut write, mut read) = tokio_tungstenite::connect_async(url)
		.await
		.expect("can't connect to server")
		.0
		.split();

	let mut conn = Connection::default();

	let (line_tx, mut line_rx) = mpsc::unbounded_channel();
	let (stdin_locked, stdin_cv) = &*Box::leak(Box::new((Mutex::new(false), Condvar::new())));

	thread::spawn(move || {
		loop {
			let mut buf = String::new();

			let mut g = stdin_locked.lock().expect("lock poisoned");

			if *g {
				g = stdin_cv.wait_while(g, |l| *l).expect("lock poisoned");
			}

			*g = true;
			drop(g);

			if line_tx
				.send(
					io::stdin()
						.read_line(&mut buf)
						.map(move |_| buf.trim().to_string()),
				)
				.is_err()
			{
				return;
			}
		}
	});

	println!("{}", "To send a ping to an ID, type that ID".blue());

	loop {
		select! {
			biased;
			msg = read.next() => {
				let Some(msg) = msg else {
					println!("{}", "Disconnected from server".bold());
					break;
				};

				let Ok(msg) = msg else {
					println!(
						"{}\n{}",
						"Error while reading websocket:".red().bold(),
						format!("{}", msg.unwrap_err()).red()
					);

					return ExitCode::FAILURE;
				};

				let Message::Text(json) = msg else {
					println!(
						"{} {}",
						"Received unexpected message type".red().bold(),
						format!("({})", match msg {
							Message::Binary(_) => "binary",
							Message::Close(_) => "close",
							Message::Frame(_) => "frame",
							Message::Ping(_) => "ping",
							Message::Pong(_) => "pong",
							Message::Text(_) => "text",
						})
						.red()
					);

					continue;
				};

				let Ok(msg) = serde_json::from_str::<ClientDownMessage>(&json) else {
					println!("{} {}", "Couldn't parse message from server".red().bold(), format!("({json})").red().dimmed());
					continue;
				};

				println!(
					"{} {}",
					format!("{msg} ").bold(),
					format!("({json})").dimmed()
				);

				handle_message(msg, &mut conn, &mut write).await;
			},
			Some(line) = line_rx.recv() => {
				let Ok(line) = line else {
					println!("{}", "Error sending ping: IO error".red().bold());
					break;
				};

				let action = if line.starts_with('a') {
					PingAction::Accept
				} else if line.starts_with('r') {
					PingAction::Reject
				} else {
					PingAction::New
				};

				match line.strip_prefix(['a', 'r']).unwrap_or(&line).parse::<u16>().map(Id) {
					Ok(id) => {
						action.perform(id, &mut conn, &mut write).await;
						*stdin_locked.lock().expect("lock poisoned") = false;
						stdin_cv.notify_all();
					},
					Err(e) => {
						println!("{} {}", "Error sending ping: invalid ID".red().bold(), format!("({e})").dimmed());
						*stdin_locked.lock().expect("lock poisoned") = false;
						stdin_cv.notify_all();
					},
				}
			}
			_ = signal::ctrl_c() => {
				break;
			}
		}
	}

	if let Err(e) = write.close().await {
		println!(
			"{}\n{}",
			"Couldn't disconnect from server:".red().bold(),
			format!("{e}").red()
		);

		ExitCode::FAILURE
	} else {
		println!("{}", "Disconnected from server".bold());

		ExitCode::SUCCESS
	}
}

/// A user action relating to a Ping
#[derive(Debug)]
enum PingAction {
	/// Send a Ping
	New,
	/// Accept an incoming Ping
	Accept,
	/// Reject an incoming Ping
	Reject,
}

impl PingAction {
	/// Perform this action
	async fn perform<W>(self, id: Id, conn: &mut Connection, write: &mut W)
	where
		W: Sink<Message> + Unpin,
		W::Error: ToString,
	{
		match self {
			Self::New => send_ping(id, conn, write).await,
			Self::Accept => {
				let my_key = EphemeralSecret::random();
				let pubkey = PublicKey((&my_key).into());

				let Some(exch) = conn.incoming.get_mut(&id) else {
					println!(
						"{} {}",
						format!("Cannot accept ping from {id}:").red().bold(),
						"No ongoing ping exchange with that ID".red()
					);
					return;
				};

				let IncomingExchange::Deciding(key) = exch else {
					println!(
						"{} {}",
						format!("Cannot accept ping from {id}:").red().bold(),
						"Not awaiting a decision on the exchange with that ID".red()
					);
					return;
				};

				*exch = IncomingExchange::AwaitingPing(my_key.diffie_hellman(&key.0).into());

				let Ok(acc) = serde_json::to_string(&ClientUpMessage {
					to: id,
					msg: ClientClientMessage::AcceptPing { key: pubkey },
				}) else {
					println!("{}", "Error serializing message".red().bold());
					return;
				};

				if let Err(e) = write.send(Message::Text(acc.into())).await {
					println!(
						"{} {}",
						"Error sending acceptation".red().bold(),
						e.to_string().dimmed()
					);
				}
			}
			Self::Reject => {
				let Some(exch) = conn.incoming.get_mut(&id) else {
					println!(
						"{} {}",
						format!("Cannot reject ping from {id}:").red().bold(),
						"No ongoing ping exchange with that ID".red()
					);
					return;
				};

				let IncomingExchange::Deciding(_) = exch else {
					println!(
						"{} {}",
						format!("Cannot reject ping from {id}:").red().bold(),
						"Not awaiting a decision on the exchange with that ID".red()
					);
					return;
				};

				let Ok(rej) = serde_json::to_string(&ClientUpMessage {
					to: id,
					msg: ClientClientMessage::RejectPing,
				}) else {
					println!("{}", "Error serializing message".red().bold());
					return;
				};

				if let Err(e) = write.send(Message::Text(rej.into())).await {
					println!(
						"{} {}",
						"Error sending rejection".red().bold(),
						e.to_string().dimmed()
					);
				}
			}
		}
	}
}

/// Send a Ping to `id`
async fn send_ping<W>(id: Id, conn: &mut Connection, write: &mut W)
where
	W: Sink<Message> + Unpin,
	W::Error: ToString,
{
	let ts = SystemTime::UNIX_EPOCH
		.elapsed()
		.expect("it's after 1970")
		.as_secs();

	let Ok(lat) = CustomType::new("Latitude: ")
		.with_help_message("Enter your latitude in degrees (between -90 and 90)")
		.with_parser(&|s| s.trim().parse().map_err(|_| ()))
		.with_validator(|v: &f64| {
			Ok(if (-90.0..=90.0).contains(v) {
				Validation::Valid
			} else {
				Validation::Invalid(ErrorMessage::Custom(
					"The latitude must be between -90 and 90 degrees".to_string(),
				))
			})
		})
		.prompt()
	else {
		println!("{}", "IO error while sending ping".red().bold());
		return;
	};

	let Ok(lon) = CustomType::new("Longitude: ")
		.with_help_message("Enter your longitude in degrees (between -180 and 180)")
		.with_parser(&|s| s.trim().parse().map_err(|_| ()))
		.with_validator(|v: &f64| {
			Ok(if (-180.0..=180.0).contains(v) {
				Validation::Valid
			} else {
				Validation::Invalid(ErrorMessage::Custom(
					"The longitude must be between -180 and 180 degrees".to_string(),
				))
			})
		})
		.prompt()
	else {
		println!("{}", "IO error while sending ping".red().bold());
		return;
	};

	let Ok(alt) = CustomType::new("Altitude: ")
		.with_help_message("Enter your altitude in meters above mean sea level")
		.with_parser(&|s| s.trim().parse().map_err(|_| ()))
		.with_validator(|v: &f32| {
			Ok(if v.is_finite() {
				Validation::Valid
			} else {
				Validation::Invalid(ErrorMessage::Custom(
					"The altitude must be finite".to_string(),
				))
			})
		})
		.prompt()
	else {
		println!("{}", "IO error while sending ping".red().bold());
		return;
	};

	let Ok(err) = CustomType::new("Position Error: ")
		.with_help_message("Enter your position error in meters")
		.with_parser(&|s| s.trim().parse().map_err(|_| ()))
		.with_validator(|v: &f32| {
			Ok(if v.is_finite() && v.is_sign_positive() {
				Validation::Valid
			} else {
				Validation::Invalid(ErrorMessage::Custom(
					"The position error must be finite and positive".to_string(),
				))
			})
		})
		.prompt()
	else {
		println!("{}", "IO error while sending ping".red().bold());
		return;
	};

	let info = PingInfo {
		ts: Timestamp(ts),
		lat: Degrees(lat),
		lon: Degrees(lon),
		alt: Meters(alt),
		err: Meters(err),
	};

	let secret = EphemeralSecret::random();

	let Ok(req) = serde_json::to_string(&ClientUpMessage {
		to: id,
		msg: ClientClientMessage::PingRequest {
			key: PublicKey((&secret).into()),
		},
	}) else {
		println!("{}", "Error serializing message".red().bold());
		return;
	};

	if let Err(e) = write.send(Message::Text(req.into())).await {
		println!(
			"{} {}",
			"Error sending ping request".red().bold(),
			e.to_string().dimmed()
		);
		return;
	}

	conn.outgoing = OutgoingExchange::AwaitingDecision(id, info, OpaqueFmt(secret));
}

/// Handle an incoming websocket message
#[expect(
	clippy::too_many_lines,
	reason = "there are a lot of messages to handle"
)]
async fn handle_message<W>(msg: ClientDownMessage, conn: &mut Connection, write: &mut W)
where
	W: Sink<Message> + Unpin,
	W::Error: ToString,
{
	match msg {
		ClientDownMessage::FromClient {
			from,
			msg: ClientClientMessage::PingRequest { key },
		} => {
			println!(
				"{}",
				format!(
					"To accept the ping from {from}, type {}, to reject it, type {}",
					format!("a{from}").blue().italic(),
					format!("r{from}").blue().italic()
				)
				.bold()
			);

			conn.incoming.insert(from, IncomingExchange::Deciding(key));
		}
		ClientDownMessage::FromClient {
			from,
			msg: ClientClientMessage::AcceptPing { key },
		} => match conn.outgoing {
			OutgoingExchange::AwaitingDecision(id, ..) if id == from => {
				let OutgoingExchange::AwaitingDecision(_, info, OpaqueFmt(my_key)) =
					mem::replace(&mut conn.outgoing, OutgoingExchange::AwaitingAck(id))
				else {
					unreachable!()
				};
				let key = my_key.diffie_hellman(&key.0);

				match (|| {
					Ok::<_, &str>(
						write.send(Message::Text(
							serde_json::to_string(&ClientUpMessage {
								to: from,
								msg: ClientClientMessage::Ping {
									info: EncryptedPingInfo(
										info.encrypt(key)
											.map_err(|_| "error encrypting ping info")?,
									),
								},
							})
							.map_err(|_| "failed to serialize message")?
							.into(),
						)),
					)
				})() {
					Ok(fut) => {
						if let Err(e) = fut.await {
							println!(
								"{} {}",
								"Error sending ping".red().bold(),
								e.to_string().dimmed()
							);
							conn.outgoing = OutgoingExchange::None;
						}
					}
					Err(e) => {
						println!("{} {}", "Error sending ping".red().bold(), e.dimmed());
						conn.outgoing = OutgoingExchange::None;
					}
				}
			}
			OutgoingExchange::AwaitingDecision(id, ..) => println!(
				"{} {}",
				format!("Received unexpected acceptation from {from}")
					.red()
					.bold(),
				format!("(a decision is expected from {id})").dimmed()
			),
			OutgoingExchange::AwaitingAck(_) => println!(
				"{} {}",
				format!("Received unexpected acceptation from {from}")
					.red()
					.bold(),
				"(a ping is being sent, but a decision is not expected)".dimmed()
			),
			OutgoingExchange::None => println!(
				"{} {}",
				format!("Received unexpected acknowledgement from {from}")
					.red()
					.bold(),
				"(no ping is being sent)".dimmed()
			),
		},
		ClientDownMessage::FromClient {
			from,
			msg: ClientClientMessage::RejectPing,
		} => match conn.outgoing {
			OutgoingExchange::AwaitingDecision(id, ..) if id == from => {
				conn.outgoing = OutgoingExchange::None;
			}
			OutgoingExchange::AwaitingDecision(id, ..) => println!(
				"{} {}",
				format!("Received unexpected rejection from {from}")
					.red()
					.bold(),
				format!("(a decision is expected from {id})").dimmed()
			),
			OutgoingExchange::AwaitingAck(_) => println!(
				"{} {}",
				format!("Received unexpected rejection from {from}")
					.red()
					.bold(),
				"(a ping is being sent, but a decision is not expected)".dimmed()
			),
			OutgoingExchange::None => println!(
				"{} {}",
				format!("Received unexpected acknowledgement from {from}")
					.red()
					.bold(),
				"(no ping is being sent)".dimmed()
			),
		},
		ClientDownMessage::FromClient {
			from,
			msg: ClientClientMessage::PingAck,
		} => match conn.outgoing {
			OutgoingExchange::AwaitingAck(id) if id == from => {
				conn.outgoing = OutgoingExchange::None;
			}
			OutgoingExchange::AwaitingAck(id) => println!(
				"{} {}",
				format!("Received unexpected acknowledgement from {from}")
					.red()
					.bold(),
				format!("(an acknowledgement is expected from {id})").dimmed()
			),
			OutgoingExchange::AwaitingDecision(..) => println!(
				"{} {}",
				format!("Received unexpected acknowledgement from {from}")
					.red()
					.bold(),
				"(a ping is being sent, but an acknowledgement is not expected)".dimmed()
			),
			OutgoingExchange::None => println!(
				"{} {}",
				format!("Received unexpected acknowledgement from {from}")
					.red()
					.bold(),
				"(no ping is being sent)".dimmed()
			),
		},
		ClientDownMessage::FromClient {
			from,
			msg: ClientClientMessage::Ping { info },
		} => {
			let key = match conn.incoming.get(&from) {
				Some(IncomingExchange::AwaitingPing(key)) => *key,
				Some(IncomingExchange::Deciding(_)) => {
					println!(
						"{} {}",
						format!("Received unexpected ping from {from}").red().bold(),
						"(a ping exchange is ongoing with that id, but a ping was not expected)"
							.dimmed()
					);
					return;
				}
				None => {
					println!(
						"{} {}",
						format!("Received unexpected ping from {from}").red().bold(),
						"(no ongoing ping exchange with that id)".dimmed()
					);
					return;
				}
			};

			conn.incoming.remove(&from);

			let Ok(info) = PingInfo::decrypt(info.0, key) else {
				println!("{}", "Could not decrypt ping info".red().bold());
				return;
			};

			println!(
				"{} {}",
				format!(
					"{from} was at {:.4}°, {:.4}° {} second(s) ago",
					info.lat.0,
					info.lon.0,
					(SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(info.ts.0)))
						.unwrap_or_else(|| SystemTime::now() + Duration::from_secs(60))
						.elapsed()
						.map_or_else(
							|err| format!("-{}", err.duration().as_secs()),
							|d| d.as_secs().to_string(),
						)
				)
				.bold(),
				format!(
					"(ts = {} s, lat = {}°, lon = {}°, alt = {} mAMSL, err = {} m)",
					info.ts.0, info.lat.0, info.lon.0, info.alt.0, info.err.0
				)
				.dimmed()
			);

			let Ok(ack) = serde_json::to_string(&ClientUpMessage {
				to: from,
				msg: ClientClientMessage::PingAck,
			}) else {
				println!("{}", "Error serializing message".red().bold());
				return;
			};

			if let Err(e) = write.send(Message::Text(ack.into())).await {
				println!(
					"{} {}",
					"Error sending acknowledgement".red().bold(),
					e.to_string().dimmed()
				);
			}
		}
		ClientDownMessage::FromServer {
			msg: ServerClientMessage::NoSuchId { id },
		} => {
			if !conn.outgoing.is_none() {
				conn.outgoing = OutgoingExchange::None;
				println!(
					"{}",
					format!("Id {id} not found, stopping outgoing ping").bold()
				);
			}
		}
		_ => (),
	}
}

/// Serde support for the public key
mod serde_public_key {
	use core::{
		fmt::{Formatter, Result as FmtResult},
		str,
	};

	use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
	use serde::{
		Deserialize, Deserializer, Serializer,
		de::{Error as DeError, Expected, Unexpected},
		ser::Error as SerError,
	};

	/// Serialize the public key
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

	/// Deserialize a public key
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
