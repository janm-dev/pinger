# Pinger

Ping people to your location.

<div align="center">
<a href="https://github.com/janm-dev/pinger/releases"><img src="https://raw.githubusercontent.com/janm-dev/pinger/main/screenshot-android.webp" width="21%" alt="A screenshot of the Pinger Android app, with a map with markers showing the user's current location and the location of a Ping. UI elements with dark blue backgrounds and white text showing 'Ping ID: 100' on the top left, a button with a pin icon on the top right, and a prompt on the bottom asking 'Accept ping from 420?' with a green checkmark button and a red X button are visible over the map. On the bottom of the screen on a dark blue background is a text input with the placeholder 'Ping ID' on the left and a big light-green button with the text 'Send Ping' on the right. Just below this is a small piece of text saying 'Map © OpenStreetMap contributors (ODbL)'." /></a>
<a href="https://pinger.janm.dev"><img src="https://raw.githubusercontent.com/janm-dev/pinger/main/screenshot-web.webp" width="78%" alt="A screenshot of the Pinger website with a dark blue background and white text, showing a big heading saying 'Pinger is coming soon' with slightly smaller, but still very big text below that saying 'Pinger is not yet available on the web' and 'Maybe try the Android app?'. Most of the second sentence is colored in light green to indicate that it is a link." /></a>
</div>

## Software

- An Android app client is implemented in `./android/` (`pinger.apk` in releases)
- A basic command-line client is implemented in `./cli/` (`cli-*` in releases)
- A web-based client is planned (will be available on <https://pinger.janm.dev>)
- The server is implemented in `./backend/` (`backend-*` in releases)
- Cryptographic operations are implemented in `./lib/` and used by the Android and command-line clients as well as the server

## Protocol

The Pinger protocol works using JSON over WebSockets.
Clients connect to the server at `wss://pinger.janm.dev/api` and the server forwards messages between clients.
Clients are identified using temporary (per-connection) 2- or 3-digit numerical IDs.
Clients send Pings containing their latitude, longitude, and altitude, as well as the (horizontal) position error and the timestamp of the location information.

After accepting a connection from a client, the server sends a "connected" message containing that client's Ping ID (a 2- or 3-digit number): `{ "msg": "connected", "id": 42 }`.

Sending a Ping involves 2 or 4 messages:

1. The sending client sends a Ping request (`{ "to": 42, "msg": "ping_request", "key": "YH9w...FeWk" }`)
2. The receiving client sends an acceptation (`{ "to": 123, "msg": "accept_ping", "key": "aG4I...iACs" }`) or rejection (`{ "to": 123, "msg": "reject_ping" }`)
3. If the Ping was accepted, the sending client sends the Ping (`{ "to": 42, "msg": "ping", "info": "UElO...Ivxg" }`)
4. After receiving the Ping, the receiving client acknowledges it (`{ "to": 123, "msg": "ping_ack" }`)

Ping requests and acceptations each contain a 32-byte base64-encoded (urlsafe, no padding) x25519 public key.
These keys are used to perform a key agreement/exchange to encrypt the information contained within a Ping (see **security** below).

The Ping itself contains (in the `info` field) the 64-byte base64-encoded (urlsafe, no padding) encrypted Ping info.
Ping info is encoded/encrypted as follows:

1. Ping info is encoded into 32 bytes:
	- bytes 0..8 contain the unsigned 64-bit timestamp in seconds since the unix epoch (big endian)
	- bytes 8..16 contain the IEEE 754 binary64 floating-point latitude in degrees north (big endian)
	- bytes 16..24 contain the IEEE 754 binary64 floating-point longitude in degrees east (big endian)
	- bytes 24..28 contain the IEEE 754 binary32 floating-point altitude in meters above mean sea level (big endian)
	- bytes 28..32 contain the IEEE 754 binary32 floating-point horizontal position error in meters (big endian)
2. The encoded Ping info is encrypted and authenticated using ChaCha20-Poly1305 using the shared secret key from the x25519 key agreement
3. The encrypted data in encoded into 64 bytes:
	- bytes 0..4 contain the byte string constant `b"PING"` (`[0x50, 0x49, 0x4e, 0x47]`) for padding
	- bytes 4..16 contain the ChaCha20-Poly1305 nonce
	- bytes 16..48 contain the encrypted ping info
	- bytes 48..64 contain the ChaCha20-Poly1305 authentication tag

Note that a client receives messages `from` but sends messages `to` another client (e.g. a ping acknowledgement will be sent as `{ "to": 123, "msg": "ping_ack" }`, but received as `{ "from": 42, "msg": "ping_ack" }`).
This translation happens in the server, and only applies to messages to/from another client.
Messages sent by the server to a client (with `msg` set to e.g. `connected` or `no_such_id`) do not have a `from` field.

Clients should only send one Ping at a time.
If multiple simultaneous (i.e. non-acknowledged) Ping requests are received from the same ID, clients should ignore all except the most recent one.

Clients must be able to receive multiple simultaneous (i.e. non-acknowledged) Ping requests from multiple different IDs.
These should *all* be handled, not ignored.

## Timeouts

Clients should implement timeouts on certain operations.
If a client implements timeouts, they must have the following durations:

- A 30 second timeout on letting the user decide on whether to accept or reject a Ping request
- A 40 second timeout on waiting for another user to accept or reject a Ping request
- A 10 second timeout on waiting for a Ping after accepting a Ping request
- A 10 second timeout on waiting for a Ping acknowledgement after sending a Ping

### All message types

Messages sent from the server to a client:

- `connected` sent upon connection of a client with their `id`
- `no_such_id` sent when a client attempts to send a message to an unknown `id` (including "response" messages like `ping_ack` if the respondee has disconnected)
- `error` sent from the server to a client when when a miscellaneous error occurs along with the `details` of the error (e.g. the client tries to send an invalid message)
- `rate_limit` sent when a client sends too many messages in too short of a timeframe with the approximate `wait`ing time in seconds before the client may try again (CURRENTLY NOT IMPLEMENTED)

Messages sent from one client to another (containing a `to` field when sent and a `from` field when received):

- `ping_request` with the requester's base64-encoded ephemeral public x25519 `key`
- `accept_ping` with the accepter's base64-encoded ephemeral public x25519 `key`
- `reject_ping` when a Ping request is rejected
- `ping` with base64-encoded encrypted Ping `info`
- `ping_ack` when a Ping has been successfully received and decrypted

Note that the server may not filter messages based on whether it's valid to send them, i.e. a client may receive a (for example) `reject_ping` message from a client they have not sent a Ping request to.
Such messages should be ignored.
Ping messages with cryptographic errors should also be ignored, though a warning should probably be shown to the user if a Ping is expected.
Do not attempt to recover from cryptogaphic errors (e.g. message authentication failure), even if the data looks "decryptable".

### Security

> [!WARNING]
>
> The cryptography discussed below was **not** implemented or reviewed by security professionals (though it is implemented on top of [audited](https://docs.rs/chacha20poly1305), [widely used](https://docs.rs/x25519-dalek) libraries).
> It's entirely possible that it does not provide any protection at all, in which case the TLS of the secure WebSocket connection between the clients and the server is the only protection left.
> Below are the optimistically-stated goals of doing this extra cryptography.

In addition to the usual TLS encryption between each client and the server, Ping info (which contains a client's location) is also encrypted end-to-end while being sent between clients.

In the `ping_request` and `accept_ping` messages, clients exchange ephemeral x25519 public keys, and before the Ping info is sent, the two clients agree on a shared secret key.
That shared secret key, along with a random nonce, is used to encrypt the Ping info using the ChaCha20-Poly1305 AEAD.
Each x25519 key pair and shared secret is only ever used once - clients should generate a new key pair for each Ping request / acceptation.

This extra encryption is intended to prevent accidental Ping info disclosure (by e.g. accidentally logging it on the server) and to provide forward secrecy (i.e. a future server compromise cannot be used to gather past Ping data).

However, such an encryption scheme cannot protect against an actively malicious server.
This is because clients are identified exclusively by server-assigned IDs and messages are routed between clients by the server, so a malicious server could simply reroute all messages to itself (performing the necessary key exchanges) and then forward the data to the original destination.
Such MitM attacks are prevented between a client and the server by the use of TLS-secured WebSockets - only a server compromise[^1] could lead to Ping info being intercepted and decrypted and/or modified.

In the future, a system of "Contacts" may be introduced to prevent such potential issues by authenticating clients when sending a Ping to them using longer-lived preshared keys/certificates.

[^1]: Or TLS private key disclosure, or an implementation bug, or cryptographic weaknesses in one of the used algorithms, or ... .

## Assets

- Logo / ping location icon based on icons from [Material Symbols](https://fonts.google.com/icons) ([Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0.html))
- Other icons from [Material Symbols](https://fonts.google.com/icons) ([Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0.html))

## License

Pinger is available under the [Mozilla Public License Version 2.0](https://www.mozilla.org/en-US/MPL/2.0/).
