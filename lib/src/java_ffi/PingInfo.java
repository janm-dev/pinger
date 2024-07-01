package dev.janm.pinger;

import dev.janm.pinger.KeyExchange;

public class PingInfo {
	public long ts;
	public double lat;
	public double lon;
	public float alt;
	public float err;

	static {
		System.loadLibrary("pinger-lib");
	}

	public PingInfo(
		long unixTimestamp,
		double latitudeDegrees,
		double longitudeDegrees,
		float altitudeMetersAboveMeanSeaLevel,
		float positionErrorMeters
	) {
		this.ts = unixTimestamp;
		this.lat = latitudeDegrees;
		this.lon = longitudeDegrees;
		this.alt = altitudeMetersAboveMeanSeaLevel;
		this.err = positionErrorMeters;
	}

	public String encrypt(KeyExchange.SharedKey key) {
		return encryptFFI(
			this.ts,
			this.lat,
			this.lon,
			this.alt,
			this.err,
			key.getSharedSecret()
		);
	}

	public static PingInfo decrypt(String str, KeyExchange.SharedKey key) {
		return decryptFFI(str, key.getSharedSecret());
	}

	private static native PingInfo decryptFFI(String str, byte[] sharedKey);

	private static native String encryptFFI(long ts, double lat, double lon, float alt, float err, byte[] sharedKey);

	@Override
	public String toString() {
		return "PingInfo(ts = " + ts + ", lat = " + lat + ", lon = " + lon + ", alt = " + alt + ", err = " + err + ")";
	}
}
