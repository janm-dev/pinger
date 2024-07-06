package dev.janm.pinger

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.Transient
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import android.util.Base64
import dev.janm.pinger.PingInfo
import dev.janm.pinger.KeyExchange
import dev.janm.pinger.KeyExchange.SharedKey

private val json = Json {
	classDiscriminator = "msg"
	encodeDefaults = true
}

@Serializable
public sealed class MessageToServer {
	override fun toString(): String {
		return json.encodeToString(this)
	}

	@Serializable
	@SerialName("ping_ack")
	public data class PingAck(@SerialName("to") private val rawTo: Short): MessageToServer() {
		@Transient
		public val to: Pinger.Id = Pinger.Id(rawTo)

		constructor(to: Pinger.Id) : this(to.getValue())

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("reject_ping")
	public data class RejectPing(@SerialName("to") private val rawTo: Short): MessageToServer() {
		@Transient
		public val to: Pinger.Id = Pinger.Id(rawTo)

		constructor(to: Pinger.Id) : this(to.getValue())

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("ping")
	public data class Ping(@SerialName("to") private val rawTo: Short, private val info: String): MessageToServer() {
		@Transient
		public val to: Pinger.Id = Pinger.Id(rawTo)

		init {
			require(Base64.decode(info, Base64.URL_SAFE or Base64.NO_PADDING).size == 64)
		}

		constructor(to: Pinger.Id, info: PingInfo, key: SharedKey) : this(to.getValue(), info.encrypt(key))

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("accept_ping")
	public data class AcceptPing(@SerialName("to") private val rawTo: Short, public val key: String): MessageToServer() {
		@Transient
		public val to: Pinger.Id = Pinger.Id(rawTo)

		init {
			require(Base64.decode(key, Base64.URL_SAFE or Base64.NO_PADDING).size == 32)
		}

		constructor(to: Pinger.Id, keyExchange: KeyExchange) : this(to.getValue(), keyExchange.publicKey)

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("ping_request")
	public data class PingRequest(@SerialName("to") private val rawTo: Short, public val key: String): MessageToServer() {
		@Transient
		public val from: Pinger.Id = Pinger.Id(rawTo)

		init {
			require(Base64.decode(key, Base64.URL_SAFE or Base64.NO_PADDING).size == 32)
		}

		constructor(to: Pinger.Id, keyExchange: KeyExchange) : this(to.getValue(), keyExchange.publicKey)

		override fun toString(): String {
			return super.toString()
		}
	}
}
