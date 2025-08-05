//! Serde support for types in this crate

/// Serde support for Pinger public keys
pub mod public_key {
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
