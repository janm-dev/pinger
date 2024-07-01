package dev.janm.pinger;

public class KeyExchange {
	private final byte[] ephemeralSecret;

	static {
		System.loadLibrary("pinger-lib");
	}

	public KeyExchange() {
		ephemeralSecret = generateEphemeralSecret();
	}

	public String getPublicKey() {
		return calculatePublicKey(ephemeralSecret);
	}

	public SharedKey diffieHellman(String otherPublicKey) {
		return new SharedKey(performDiffieHellman(ephemeralSecret, otherPublicKey));
	}

	private static native byte[] generateEphemeralSecret();

	private static native String calculatePublicKey(byte[] secret);

	private static native byte[] performDiffieHellman(byte[] ourSecret, String theirPublicKey);
	
	public static class SharedKey {
		private final byte[] sharedSecret;

		private SharedKey(byte[] sharedSecret) {
			this.sharedSecret = sharedSecret;
		}

		private static native String base64Encode(byte[] sharedSecret);

		byte[] getSharedSecret() {
			return this.sharedSecret;
		}

		@Override
		public String toString() {
			return base64Encode(sharedSecret);
		}
	}
}
