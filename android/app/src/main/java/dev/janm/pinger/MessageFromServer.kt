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
public sealed class MessageFromServer {
	override fun toString(): String {
		return json.encodeToString(this)
	}

	@Serializable
	@SerialName("connected")
	public data class Connected(@SerialName("id") private val rawId: Short): MessageFromServer() {
		@Transient
		public val id: Pinger.Id = Pinger.Id(rawId)

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("error")
	public data class Error(public val details: String): MessageFromServer() {
		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("no_such_id")
	public data class NoSuchId(@SerialName("id") private val rawId: Short): MessageFromServer() {
		@Transient
		public val id: Pinger.Id = Pinger.Id(rawId)

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("ping_ack")
	public data class PingAck(@SerialName("from") private val rawFrom: Short): MessageFromServer() {
		@Transient
		public val from: Pinger.Id = Pinger.Id(rawFrom)

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("reject_ping")
	public data class RejectPing(@SerialName("from") private val rawFrom: Short): MessageFromServer() {
		@Transient
		public val from: Pinger.Id = Pinger.Id(rawFrom)

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("ping")
	public data class Ping(@SerialName("from") private val rawFrom: Short, private val info: String): MessageFromServer() {
		@Transient
		public val from: Pinger.Id = Pinger.Id(rawFrom)

		init {
			require(Base64.decode(info, Base64.URL_SAFE or Base64.NO_PADDING).size == 64)
		}

		public fun getInfo(key: SharedKey): PingInfo {
			return PingInfo.decrypt(info, key)
		}

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("accept_ping")
	public data class AcceptPing(@SerialName("from") private val rawFrom: Short, public val key: String): MessageFromServer() {
		@Transient
		public val from: Pinger.Id = Pinger.Id(rawFrom)

		init {
			require(Base64.decode(key, Base64.URL_SAFE or Base64.NO_PADDING).size == 32)
		}

		override fun toString(): String {
			return super.toString()
		}
	}

	@Serializable
	@SerialName("ping_request")
	public data class PingRequest(@SerialName("from") private val rawFrom: Short, public val key: String): MessageFromServer() {
		@Transient
		public val from: Pinger.Id = Pinger.Id(rawFrom)

		init {
			require(Base64.decode(key, Base64.URL_SAFE or Base64.NO_PADDING).size == 32)
		}

		override fun toString(): String {
			return super.toString()
		}
	}
}
