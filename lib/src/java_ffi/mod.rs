//! Java/Kotlin FFI for this library

use core::{array, str};
use std::{
	backtrace::Backtrace,
	panic::{self, AssertUnwindSafe},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jni::{
	JNIEnv,
	objects::{JByteArray, JClass, JObject, JString, JValueGen},
	sys::{jdouble, jfloat, jlong},
};
use x25519_dalek::StaticSecret;

use crate::{Degrees, EncryptedPingInfo, Meters, PingInfo, PublicKey, SharedKey, Timestamp};

trait ErrStr {
	type Ok;

	fn str(self) -> Result<Self::Ok, String>;
}

impl<T, E: ToString> ErrStr for Result<T, E> {
	type Ok = T;

	fn str(self) -> Result<T, String> {
		self.map_err(|e| e.to_string())
	}
}

fn panic() -> String {
	format!("Panic!\n\n{}", Backtrace::force_capture())
}

macro_rules! handle_err {
	($env:ident -> $ret:ty : | $param:pat_param = $param_ty:ty | $x:block) => {
		#[forbid(unsafe_code)]
		{
			let ref_env = &mut $env;
			match panic::catch_unwind(AssertUnwindSafe(move || -> Result<$ret, String> {
				(|$param: $param_ty| $x)(ref_env)
			})) {
				Ok(Ok(res)) => res,
				Ok(Err(err)) => {
					let _ = $env.throw_new("java/lang/RuntimeException", err);
					<$ret>::default()
				}
				Err(_) => {
					let _ = $env.throw_new("java/lang/RuntimeException", panic());
					<$ret>::default()
				}
			}
		}
	};
}

/// **`String dev.janm.pinger.PingInfo.encryptFFI(long ts, double lat, double
/// lon, float alt, float err, byte[] sharedKey)`**
///
/// Encrypt and base64-encode the given Ping info using the given shared key
#[expect(
	unsafe_code,
	reason = "no_mangle is required for Java FFI, and the user is expected to uphold the \
	          soundness requirements of the attribute"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_janm_pinger_PingInfo_encryptFFI<'e>(
	mut env: JNIEnv<'e>,
	_class: JClass<'e>,
	ts: jlong,
	lat: jdouble,
	lon: jdouble,
	alt: jfloat,
	err: jfloat,
	key: JByteArray,
) -> JString<'e> {
	handle_err! { env -> JString<'e>: |env = &mut JNIEnv<'e>| {
		let info = PingInfo {
			ts: Timestamp(java_u64_to_rust(ts)),
			lat: Degrees(lat),
			lon: Degrees(lon),
			alt: Meters(alt),
			err: Meters(err),
		};

		let mut buf = [0i8; 32];
		let () = env.get_byte_array_region(key, 0, &mut buf).str()?;
		let buf = java_u8_array_to_rust(buf);
		let encrypted = info.encrypt(SharedKey::from_bytes(buf)).str()?;

		let mut buf = [0u8; 86];
		let n = URL_SAFE_NO_PAD.encode_slice(encrypted.0, &mut buf).str()?;
		env.new_string(str::from_utf8(&buf[..n]).str()?).str()
	}}
}

/// **`PingInfo dev.janm.pinger.PingInfo.decryptFFI(String str, byte[]
/// sharedKey)`**
///
/// Decrypt the given (base64-encoded) encrypted Ping info using the given
/// shared key
#[expect(
	unsafe_code,
	reason = "no_mangle is required for Java FFI, and the user is expected to uphold the \
	          soundness requirements of the attribute"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_janm_pinger_PingInfo_decryptFFI<'e>(
	mut env: JNIEnv<'e>,
	class: JClass,
	str: JString<'e>,
	key: JByteArray<'e>,
) -> JObject<'e> {
	handle_err! { env -> JObject<'e>: |env = &mut JNIEnv<'e>| {
		let mut buf = [0i8; 32];
		let () = env.get_byte_array_region(key, 0, &mut buf).str()?;
		let key = SharedKey::from_bytes(java_u8_array_to_rust(buf));

		let mut buf = [0u8; 64];
		let n = URL_SAFE_NO_PAD
			.decode_slice(env.get_string(&str).str()?.to_str().str()?, &mut buf)
			.str()?;
		let info = PingInfo::decrypt(EncryptedPingInfo(buf[..n].try_into().str()?), key).str()?;

		env.new_object(class, "(JDDFF)V", &[
				JValueGen::Long(rust_u64_to_java(info.ts.0)),
				JValueGen::Double(info.lat.0),
				JValueGen::Double(info.lon.0),
				JValueGen::Float(info.alt.0),
				JValueGen::Float(info.err.0),
			])
			.str()
	}}
}

/// **`String dev.janm.pinger.KeyExchange.calculatePublicKey(byte[] secret)`**
///
/// Calculate the public key (as a base64 string) for the given private key
#[expect(
	unsafe_code,
	reason = "no_mangle is required for Java FFI, and the user is expected to uphold the \
	          soundness requirements of the attribute"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_janm_pinger_KeyExchange_calculatePublicKey<'e>(
	mut env: JNIEnv<'e>,
	_class: JClass<'e>,
	secret: JByteArray<'e>,
) -> JString<'e> {
	handle_err! { env -> JString<'e>: |env = &mut JNIEnv<'e>| {
		let mut buf = [0i8; 32];
		let () = env.get_byte_array_region(secret, 0, &mut buf).str()?;
		let buf = java_u8_array_to_rust(buf);
		let secret = StaticSecret::from(buf);

		let mut buf = [0u8; 43];
		let n = URL_SAFE_NO_PAD
			.encode_slice(PublicKey::from(&secret).to_bytes(), &mut buf)
			.str()?;

		let str: &str = str::from_utf8(&buf[..n]).str()?;
		env.new_string(str).str()
	}}
}

/// **`byte[] dev.janm.pinger.KeyExchange.performDiffieHellman(byte[] ourSecret,
/// String theirPublicKey)`**
///
/// Perform the key exchange with our private key and the other party's
/// (base64-encoded) public key
#[expect(
	unsafe_code,
	reason = "no_mangle is required for Java FFI, and the user is expected to uphold the \
	          soundness requirements of the attribute"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_janm_pinger_KeyExchange_performDiffieHellman<'e>(
	mut env: JNIEnv<'e>,
	_class: JClass<'e>,
	secret: JByteArray<'e>,
	public_key: JString<'e>,
) -> JByteArray<'e> {
	handle_err! { env -> JByteArray<'e>: |env = &mut JNIEnv<'e>| {
		let mut buf = [0i8; 32];
		let () = env.get_byte_array_region(secret, 0, &mut buf).str()?;
		let secret = StaticSecret::from(java_u8_array_to_rust(buf));

		let mut buf = [0u8; 32];
		let n = URL_SAFE_NO_PAD
			.decode_slice(env.get_string(&public_key).str()?.to_str().str()?, &mut buf)
			.str()?;
		let public_key = PublicKey::from(<[u8; 32]>::try_from(&buf[..n]).str()?);

		let shared_secret = secret.diffie_hellman(&public_key);
		env.byte_array_from_slice(&shared_secret.to_bytes()).str()
	}}
}

/// **`byte[] dev.janm.pinger.KeyExchange.generateEphemeralSecret()`**
///
/// Generate a random ephemeral secret key for the key exchange as a byte array
#[expect(
	unsafe_code,
	reason = "no_mangle is required for Java FFI, and the user is expected to uphold the \
	          soundness requirements of the attribute"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_janm_pinger_KeyExchange_generateEphemeralSecret<'e>(
	mut env: JNIEnv<'e>,
	_class: JClass<'e>,
) -> JByteArray<'e> {
	handle_err! { env -> JByteArray<'e>: |env = &mut JNIEnv<'e>| {
		let secret = StaticSecret::random();
		env.byte_array_from_slice(&secret.to_bytes()).str()
	}}
}

/// **`String dev.janm.pinger.KeyExchange.SharedKey.base64Encode(byte[]
/// sharedSecret)`**
///
/// Encode the given byte array into a base64 string
#[expect(
	unsafe_code,
	reason = "no_mangle is required for Java FFI, and the user is expected to uphold the \
	          soundness requirements of the attribute"
)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_janm_pinger_KeyExchange_00024SharedKey_base64Encode<'e>(
	mut env: JNIEnv<'e>,
	_class: JClass<'e>,
	shared_secret: JByteArray<'e>,
) -> JString<'e> {
	handle_err! { env -> JString<'e>: |env = &mut JNIEnv<'e>| {
		let mut buf = [0i8; 32];
		let () = env.get_byte_array_region(shared_secret, 0, &mut buf).str()?;
		let shared_secret = java_u8_array_to_rust(buf);

		let mut buf = [0u8; 43];
		let n = URL_SAFE_NO_PAD
			.encode_slice(shared_secret, &mut buf)
			.str()?;

		let str: &str = str::from_utf8(&buf[..n]).str()?;
		env.new_string(str).str()
	}}
}

/// Cast a Java byte array (`[i8; _]` in Rust) bitwise to a Rust byte array
/// (`[u8; _]`)
#[must_use]
fn java_u8_array_to_rust<const N: usize>(array: [i8; N]) -> [u8; N] {
	array::from_fn(|i| u8::from_ne_bytes(array[i].to_ne_bytes()))
}

/// Cast a Java 64-bit integer (`i64` in Rust) bitwise to a Rust `u64`
#[must_use]
const fn java_u64_to_rust(u64: i64) -> u64 {
	u64::from_ne_bytes(u64.to_ne_bytes())
}

/// Cast a Rust `u64` bitwise to a Java 64-bit integer (`i64` in Rust)
#[must_use]
const fn rust_u64_to_java(u64: u64) -> i64 {
	i64::from_ne_bytes(u64.to_ne_bytes())
}
