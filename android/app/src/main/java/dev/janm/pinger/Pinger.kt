package dev.janm.pinger

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.net.URL
import androidx.core.util.Consumer
import kotlinx.serialization.json.Json
import java.util.logging.Logger
import dev.janm.pinger.KeyExchange
import dev.janm.pinger.KeyExchange.SharedKey
import java.util.Timer
import java.util.TimerTask
import java.util.concurrent.ConcurrentHashMap

const val BASE_TIMEOUT: Long = 10000
const val DECISION_TIMEOUT: Long = 3000//0

private enum class State {
	Connecting,
	Ready,
	AwaitingAccept,
	AwaitingAck;
}

private fun State?.assertIs(vararg expected: State) {
	if (this == null || expected.all { it != this }) {
		throw IllegalStateException("invalid state: expected one of ${expected.toList()}, but the state is $this")
	}
}

private enum class IncomingState {
	Deciding,
	AwaitingPing;
}

private fun IncomingState?.assertIs(vararg expected: IncomingState) {
	if (this == null || expected.all { it != this }) {
		throw IllegalStateException("invalid state: expected one of ${expected.toList()}, but the state is $this")
	}
}

private class Exchange(
	val theirPublicKey: String,
	val decisionTimeout: TimerTask,
	val pingTimeout: TimerTask,
	var state: IncomingState = IncomingState.Deciding,
	var keyExchange: KeyExchange? = null
)

public class Pinger private constructor(
	private val server: URL,
	private val userAgent: String?,
	private val onPing: Iterable<Consumer<Pair<Id, PingInfo>>>,
	private val onPingTimeout: Iterable<Consumer<Id>>,
	private val onError: Iterable<Consumer<String>>,
	private val onIdNotFound: Iterable<Consumer<Id>>,
	private val onConnected: Iterable<Consumer<Id>>,
	private val onRejected: Iterable<Consumer<Id>>,
	private val onResponseTimeout: Iterable<Consumer<Id>>,
	private val onAcknowledged: Iterable<Consumer<Id>>,
	private val onAcknowledgeTimeout: Iterable<Consumer<Id>>,
	private val onRequest: Iterable<Consumer<Id>>,
	private val onDecisionTimeout: Iterable<Consumer<Id>>,
): WebSocketListener() {
	private val logger = Logger.getLogger("PingerConnection")
	private var timeoutTimer = Timer("PingerTimeouts", true)
	private var state = State.Connecting
	private var connection = connect()
	private var sendingTo: Id? = null
	private var keyExchange: KeyExchange? = null
	private var infoToSend: PingInfo? = null
	private var responseTimeout: TimerTask? = null
	private var ackTimeout: TimerTask? = null
	private var reconnecting = false
	private var exchanges = ConcurrentHashMap<Id, Exchange>()
	private val json = Json {
		classDiscriminator = "msg"
		encodeDefaults = true
	}

	private fun connect(): WebSocket {
		state.assertIs(State.Connecting)

		val request = Request.Builder().url(server)

		if (userAgent != null) {
			request.header("User-Agent", userAgent)
			logger.info("setting API user agent to `$userAgent`")
		}

		logger.info("connecting to API at `$server`")
		reconnecting = false

		return OkHttpClient().newWebSocket(request.build(), this)
	}

	private fun reconnect() {
		if (reconnecting) {
			return
		}

		reconnecting = true

		if (connection.close(1001, "reconnecting")) {
			logger.warning("reconnecting an open connection")
		}

		timeoutTimer.cancel()
		Thread.sleep(1000)

		connection.cancel()
		state = State.Connecting
		sendingTo = null
		keyExchange = null
		infoToSend = null
		timeoutTimer = Timer("PingerTimeouts", true)
		exchanges.clear()

		connection = connect()
	}

	public fun send(to: Id, info: PingInfo) {
		if (state != State.Ready) {
			throw IllegalStateException("not ready to send ping request")
		}

		responseTimeout = object: TimerTask() {
			override fun run() {
				val id = sendingTo
				logger.info("decision response timeout while pinging $id")

				if (id == null || state != State.AwaitingAccept) {
					logger.warning("decision response timeout triggered while not pinging anyone")
					return
				}

				state = State.Ready
				sendingTo = null
				infoToSend = null
				keyExchange = null
				onResponseTimeout.forEach { it.accept(id) }
			}
		}

		logger.info("sending ping request to $to")
		keyExchange = KeyExchange()
		state = State.AwaitingAccept
		infoToSend = info
		sendingTo = to
		connection.send(MessageToServer.PingRequest(to, keyExchange!!).toString())

		timeoutTimer.schedule(responseTimeout, BASE_TIMEOUT + DECISION_TIMEOUT)
	}

	public fun accept(from: Id) {
		logger.info("accepting ping from $from")
		val exchange = exchanges[from] ?: throw IllegalArgumentException("no ongoing exchange with $from")
		exchange.decisionTimeout.cancel()
		exchange.keyExchange = KeyExchange()
		exchange.state = IncomingState.AwaitingPing
		connection.send(MessageToServer.AcceptPing(from, exchange.keyExchange!!).toString())
		timeoutTimer.schedule(exchange.pingTimeout, BASE_TIMEOUT)
	}

	public fun reject(from: Id) {
		logger.info("rejecting ping from $from")
		exchanges[from]?.decisionTimeout?.cancel()
		exchanges.remove(from) ?: throw IllegalArgumentException("no ongoing exchange with $from")
		connection.send(MessageToServer.RejectPing(from).toString())
	}

	override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
		super.onClosing(webSocket, code, reason)

		state = State.Connecting
		onError.forEach { it.accept(reason) }
		reconnect()
	}

	override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
		super.onFailure(webSocket, t, response)

		state = State.Connecting
		onError.forEach { it.accept(t.toString()) }
		reconnect()
	}

	override fun onMessage(webSocket: WebSocket, text: String) {
		super.onMessage(webSocket, text)

		val message = try {
			json.decodeFromString<MessageFromServer>(text)
		} catch (e: Exception) {
			logger.severe("invalid data received from API: $e")
			return
		}

		logger.info("new websocket message received: `$message`")

		try {
			when (message) {
				is MessageFromServer.Connected -> {
					state.assertIs(State.Connecting)
					state = State.Ready
					onConnected.forEach { it.accept(message.id) }
				}
				is MessageFromServer.Error -> {
					logger.warning("error message received: ${message.details}")
					onError.forEach { it.accept(message.details) }
					reconnect()
				}
				is MessageFromServer.NoSuchId -> if (message.id == sendingTo) {
					state.assertIs(State.AwaitingAccept)
					state = State.Ready
					keyExchange = null
					infoToSend = null
					logger.warning("id not found: ${message.id}")
					onIdNotFound.forEach { it.accept(message.id) }
				} else {
					logger.warning("incoming ping from ${message.id} can't be replied to")
					exchanges.remove(message.id)
					onError.forEach { it.accept("Unable to reply to ${message.id} - id not found") }
				}
				is MessageFromServer.Ping -> if (exchanges.containsKey(message.from)) {
					val exchange = exchanges[message.from]!!
					exchange.state.assertIs(IncomingState.AwaitingPing)
					logger.info("ping received from ${message.from}")
					exchange.pingTimeout.cancel()

					try {
						val pingInfo = message.getInfo(exchange.keyExchange!!.diffieHellman(exchange.theirPublicKey))
						onPing.forEach { it.accept(message.from to pingInfo) }
						exchanges.remove(message.from)
						connection.send(MessageToServer.PingAck(message.from).toString())
					} catch (e: Exception) {
						logger.warning("error getting ping info from incoming ping from ${message.from}: $e")
					}
				} else {
					logger.warning("got ping from ${message.from}, but there is no ongoing exchange with that id")
				}
				is MessageFromServer.PingAck -> if (sendingTo == message.from) {
					state.assertIs(State.AwaitingAck)
					state = State.Ready
					ackTimeout?.cancel()
					ackTimeout = null
					sendingTo = null
					logger.info("ping acknowledged by ${message.from}")
					onAcknowledged.forEach { it.accept(message.from) }
				} else {
					logger.warning("received unexpected ping acknowledgement (from ${message.from}, expected ack from $sendingTo)")
				}
				is MessageFromServer.AcceptPing -> if (sendingTo == message.from) {
					state.assertIs(State.AwaitingAccept)
					state = State.AwaitingAck
					responseTimeout?.cancel()
					responseTimeout = null
					ackTimeout = object: TimerTask() {
						override fun run() {
							logger.info("ping ack timeout while pinging $sendingTo")

							if (sendingTo == null || state != State.AwaitingAck) {
								logger.warning("ack timeout triggered while not awaiting acknowledgement (pinging $sendingTo)")
								return
							}

							state = State.Ready
							sendingTo = null
							onAcknowledgeTimeout.forEach { it.accept(message.from) }
						}
					}

					logger.info("ping accepted by ${message.from}")
					connection.send(MessageToServer.Ping(message.from, infoToSend!!, keyExchange!!.diffieHellman(message.key)).toString())
					infoToSend = null
					timeoutTimer.schedule(ackTimeout, BASE_TIMEOUT)
				} else {
					logger.warning("received unexpected ping acceptation (from ${message.from}, expected accept from $sendingTo)")
				}
				is MessageFromServer.RejectPing -> if (sendingTo == message.from) {
					state.assertIs(State.AwaitingAccept)
					state = State.Ready
					sendingTo = null
					infoToSend = null
					responseTimeout?.cancel()
					responseTimeout = null
					logger.info("ping rejected by ${message.from}")
					onRejected.forEach { it.accept(message.from) }
				} else {
					logger.warning("received unexpected ping rejection (from ${message.from}, expected reject from $sendingTo)")
				}
				is MessageFromServer.PingRequest -> {
					val decisionTimeout = object: TimerTask() {
						override fun run() {
							logger.info("decision timeout while getting pinged by ${message.from}")

							if (exchanges[message.from] == null || exchanges[message.from]?.state != IncomingState.Deciding) {
								logger.warning("decision timeout triggered while not deciding or without exchange (deciding on ping from ${message.from})")
							}

							exchanges.remove(message.from)
							onDecisionTimeout.forEach { it.accept(message.from) }
						}
					}

					val pingTimeout = object: TimerTask() {
						override fun run() {
							logger.info("ping timeout while getting pinged by ${message.from}")

							if (exchanges[message.from] == null || exchanges[message.from]?.state != IncomingState.AwaitingPing) {
								logger.warning("ping timeout triggered while not awaiting ping or without exchange (awaiting ping from ${message.from})")
							}

							exchanges.remove(message.from)
							onPingTimeout.forEach { it.accept(message.from) }
						}
					}

					timeoutTimer.schedule(decisionTimeout, DECISION_TIMEOUT)
					exchanges[message.from] = Exchange(message.key, decisionTimeout, pingTimeout)
					logger.info("new ping request from ${message.from}")
					onRequest.forEach { it.accept(message.from) }
				}
			}
		} catch (e: IllegalStateException) {
			throw RuntimeException("connection state error", e)
		} catch (e: Exception) {
			throw RuntimeException("exception in pinger connection", e)
		}
	}

	public class Builder(private val server: URL) {
		private var userAgent: String? = null
		private var onPing: ArrayList<Consumer<Pair<Id, PingInfo>>> = ArrayList()
		private var onPingTimeout: ArrayList<Consumer<Id>> = ArrayList()
		private var onError: ArrayList<Consumer<String>> = ArrayList()
		private var onIdNotFound: ArrayList<Consumer<Id>> = ArrayList()
		private var onConnected: ArrayList<Consumer<Id>> = ArrayList()
		private var onRejected: ArrayList<Consumer<Id>> = ArrayList()
		private var onResponseTimeout: ArrayList<Consumer<Id>> = ArrayList()
		private var onAcknowledged: ArrayList<Consumer<Id>> = ArrayList()
		private var onAcknowledgeTimeout: ArrayList<Consumer<Id>> = ArrayList()
		private var onRequest: ArrayList<Consumer<Id>> = ArrayList()
		private var onDecisionTimeout: ArrayList<Consumer<Id>> = ArrayList()

		public fun userAgent(userAgentString: String): Builder {
			userAgent = userAgentString
			return this
		}

		public fun onPing(callback: Consumer<Pair<Id, PingInfo>>): Builder {
			onPing.add { it -> try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onPing handler", e)
			} }
			return this
		}

		public fun onPingTimeout(callback: Consumer<Id>): Builder {
			onPingTimeout.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onPingTimeout handler", e)
			} }
			return this
		}

		public fun onError(callback: Consumer<String>): Builder {
			onError.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onError handler", e)
			} }
			return this
		}

		public fun onIdNotFound(callback: Consumer<Id>): Builder {
			onIdNotFound.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onIdNotFound handler", e)
			} }
			return this
		}

		public fun onConnected(callback: Consumer<Id>): Builder {
			onConnected.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onConnected handler", e)
			} }
			return this
		}

		public fun onRejected(callback: Consumer<Id>): Builder {
			onRejected.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onRejected handler", e)
			} }
			return this
		}

		public fun onResponseTimeout(callback: Consumer<Id>): Builder {
			onResponseTimeout.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onResponseTimeout handler", e)
			} }
			return this
		}

		public fun onAcknowledged(callback: Consumer<Id>): Builder {
			onAcknowledged.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onAcknowledged handler", e)
			} }
			return this
		}

		public fun onAcknowledgeTimeout(callback: Consumer<Id>): Builder {
			onAcknowledgeTimeout.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onAcknowledgeTimeout handler", e)
			} }
			return this
		}

		public fun onRequest(callback: Consumer<Id>): Builder {
			onRequest.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onRequest handler", e)
			} }
			return this
		}

		public fun onDecisionTimeout(callback: Consumer<Id>): Builder {
			onDecisionTimeout.add { try {
				callback.accept(it)
			} catch (e: Exception) {
				throw RuntimeException("Exception in onDecisionTimeout handler", e)
			} }
			return this
		}

		public fun build(): Pinger {
			return Pinger(
				server,
				userAgent,
				onPing,
				onPingTimeout,
				onError,
				onIdNotFound,
				onConnected,
				onRejected,
				onResponseTimeout,
				onAcknowledged,
				onAcknowledgeTimeout,
				onRequest,
				onDecisionTimeout
			)
		}
	}

	public class Id(private val value: Short) {
		init {
			if (value < 10 || value > 999) {
				throw IllegalArgumentException("$value is not a valid ping id")
			}
		}

		public fun getValue(): Short {
			return value
		}

		override fun toString(): String {
			return value.toString()
		}

		override fun equals(other: Any?): Boolean {
			if (other is Id) {
				return value == other.value
			}

			return super.equals(other)
		}

		override fun hashCode(): Int {
			return value.hashCode()
		}
	}
}
