#![cfg_attr(not(feature = "std"), no_std)]

use core::{
	fmt::{Debug, Display, Formatter, Result as FmtResult},
	str,
};

use chacha20poly1305::{
	aead::OsRng, AeadCore, AeadInPlace, ChaCha20Poly1305, Error as ChaChaError, KeyInit,
};
use serde::{Deserialize, Serialize};
pub use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret};

#[cfg(feature = "java-ffi")]
pub mod java_ffi;

#[derive(Debug)]
pub struct CryptoError;

impl Display for CryptoError {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		write!(f, "error performing cryptographic operation")
	}
}

impl From<ChaChaError> for CryptoError {
	fn from(_: ChaChaError) -> Self {
		Self
	}
}

#[derive(Clone, Copy)]
pub struct SharedKey([u8; 32]);

impl SharedKey {
	pub fn from_bytes(bytes: [u8; 32]) -> Self {
		Self(bytes)
	}

	pub fn to_bytes(self) -> [u8; 32] {
		self.0
	}
}

impl From<SharedSecret> for SharedKey {
	fn from(value: SharedSecret) -> Self {
		Self::from_bytes(value.to_bytes())
	}
}

impl Debug for SharedKey {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		f.debug_struct("SharedKey").finish_non_exhaustive()
	}
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EncryptedPingInfo(#[serde(with = "serde_encrypted_ping_info")] [u8; 64]);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PingInfo {
	pub ts: Timestamp,
	pub lat: Degrees,
	pub lon: Degrees,
	pub alt: Meters,
	pub err: Meters,
}

impl PingInfo {
	pub fn encrypt(self, key: impl Into<SharedKey>) -> Result<EncryptedPingInfo, CryptoError> {
		let mut encoded = self.encode();
		let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);

		let chacha = ChaCha20Poly1305::new(&key.into().to_bytes().into());
		let tag = chacha.encrypt_in_place_detached(&nonce, b"", &mut encoded)?;

		let mut buf = [0u8; 64];
		buf[0..4].copy_from_slice(b"PING");
		buf[4..16].copy_from_slice(&nonce);
		buf[16..48].copy_from_slice(&encoded);
		buf[48..64].copy_from_slice(&tag);

		Ok(EncryptedPingInfo(buf))
	}

	pub fn decrypt(
		bytes: EncryptedPingInfo,
		key: impl Into<SharedKey>,
	) -> Result<Self, CryptoError> {
		let mut bytes = bytes.0;
		let (&mut ref ping, bytes) = bytes.split_at_mut(4);
		let (&mut ref nonce, bytes) = bytes.split_at_mut(12);
		let (buf, &mut ref tag) = bytes.split_at_mut(32);

		if ping != b"PING" {
			return Err(CryptoError);
		}

		let chacha = ChaCha20Poly1305::new(&key.into().to_bytes().into());

		let () = chacha.decrypt_in_place_detached(nonce.into(), b"", buf, tag.into())?;

		Ok(Self::decode(buf.try_into().unwrap()))
	}

	fn encode(self) -> [u8; 32] {
		let mut buf = [0u8; 32];
		buf[0..8].copy_from_slice(&self.ts.0.to_be_bytes());
		buf[8..16].copy_from_slice(&self.lat.0.to_be_bytes());
		buf[16..24].copy_from_slice(&self.lon.0.to_be_bytes());
		buf[24..28].copy_from_slice(&self.alt.0.to_be_bytes());
		buf[28..32].copy_from_slice(&self.err.0.to_be_bytes());
		buf
	}

	fn decode(bytes: [u8; 32]) -> Self {
		let ts = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
		let lat = f64::from_be_bytes(bytes[8..16].try_into().unwrap());
		let lon = f64::from_be_bytes(bytes[16..24].try_into().unwrap());
		let alt = f32::from_be_bytes(bytes[24..28].try_into().unwrap());
		let err = f32::from_be_bytes(bytes[28..32].try_into().unwrap());

		Self {
			ts: Timestamp(ts),
			lat: Degrees(lat),
			lon: Degrees(lon),
			alt: Meters(alt),
			err: Meters(err),
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Degrees(pub f64);

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Meters(pub f32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(pub u64);

mod serde_encrypted_ping_info {
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

	pub fn serialize<S: Serializer>(val: &[u8; 64], ser: S) -> Result<S::Ok, S::Error> {
		let mut buf = [0u8; 86];

		let n = URL_SAFE_NO_PAD
			.encode_slice(val, &mut buf)
			.map_err(|_| SerError::custom("failed to base64-encode"))?;

		ser.serialize_str(
			str::from_utf8(&buf[..n])
				.map_err(|_| SerError::custom("failed to create base64 string"))?,
		)
	}

	pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<[u8; 64], D::Error> {
		struct Expected64ByteSlice;

		impl Expected for Expected64ByteSlice {
			fn fmt(&self, f: &mut Formatter) -> FmtResult {
				write!(f, "a base64-encoded 64-byte slice")
			}
		}

		let str = <&str as Deserialize>::deserialize(de)?;
		let mut buf = [0u8; 64];

		let n = URL_SAFE_NO_PAD
			.decode_slice(str, &mut buf)
			.map_err(|_| DeError::invalid_value(Unexpected::Str(str), &Expected64ByteSlice))?;

		if n != 64 {
			return Err(DeError::invalid_length(n, &Expected64ByteSlice));
		}

		Ok(buf)
	}
}
