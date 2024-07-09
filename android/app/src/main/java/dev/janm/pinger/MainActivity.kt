package dev.janm.pinger

import android.Manifest
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.system.Os
import android.view.View
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.EditText
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.DrawableRes
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.content.res.AppCompatResources
import androidx.browser.customtabs.CustomTabsIntent
import androidx.constraintlayout.widget.ConstraintLayout
import androidx.core.view.marginBottom
import androidx.dynamicanimation.animation.DynamicAnimation
import androidx.dynamicanimation.animation.SpringAnimation
import org.osmdroid.OsmdroidBuildInfo
import org.osmdroid.config.Configuration.*
import org.osmdroid.tileprovider.tilesource.TileSourcePolicy
import org.osmdroid.tileprovider.tilesource.XYTileSource
import org.osmdroid.util.BoundingBox
import org.osmdroid.util.MapTileIndex
import org.osmdroid.views.MapView
import org.osmdroid.views.overlay.mylocation.GpsMyLocationProvider
import org.osmdroid.views.overlay.mylocation.MyLocationNewOverlay
import java.net.URL
import java.util.logging.Logger
import dev.janm.pinger.PingInfo
import org.osmdroid.util.GeoPoint
import org.osmdroid.views.overlay.Marker
import java.io.InputStreamReader
import java.util.Timer
import java.util.TimerTask
import kotlin.system.exitProcess

private const val MIN_ZOOM_LEVEL = 2.0
private const val MAX_ZOOM_LEVEL = 22.0
private val INITIAL_LOCATION = BoundingBox(75.0, 0.0, -75.0, -0.0)
private val MAP_QUALITY = MapQuality.UNREADABLE

class MainActivity: AppCompatActivity() {
	private lateinit var logger: Logger
	private lateinit var map: MapView
	private lateinit var connection: Pinger
	private val mapUpdateTimer = Timer("PingerMapUpdates", true)

	init {
		ProcessBuilder()
			.command("logcat", "-c")
			.start()
			.waitFor()
	}

	override fun onCreate(savedInstanceState: Bundle?) {
		super.onCreate(savedInstanceState)

		logger = Logger.getLogger(resources.getString(R.string.app_name))

		Thread.setDefaultUncaughtExceptionHandler { t, e ->
			val app = try {
				"${resources.getString(R.string.app_name)}/${packageManager.getPackageInfo(packageName, 0).versionName} (Android ${Build.VERSION.RELEASE})"
			} catch (e: Exception) {
				"Pinger/? (Android ${Build.VERSION.RELEASE})"
			}

			var rawLogs = ""

			try {
				val process = ProcessBuilder()
					.command("logcat", "-d")
					.redirectErrorStream(true)
					.start()

				InputStreamReader(process.inputStream).forEachLine {
					rawLogs = "${it.trim()}\n$rawLogs"
				}
			} catch (_: Exception) {
			}

			try {
				val formattedLogs = "Thread $t threw uncaught exception $e\n${e.stackTraceToString()}\n$rawLogs"

				var uri = "https://pinger.janm.dev/bug?oops&app=${Uri.encode(app)}&logs=${Uri.encode(formattedLogs)}"

				if (uri.length > 35000) {
					uri = uri.slice(0..35000) + "\n[logs truncated]"
				}

				CustomTabsIntent.Builder()
					.setColorScheme(CustomTabsIntent.COLOR_SCHEME_DARK)
					.build()
					.launchUrl(this, Uri.parse(uri))

				logger.severe("Uncaught exception: $e")
				exitProcess(1)
			} catch (_: Exception) {
				exitProcess(2)
			}
		}

		val userAgent = "${resources.getString(R.string.app_name)}/${packageManager.getPackageInfo(packageName, 0).versionName}" +
			" (Android ${Build.VERSION.RELEASE};" +
			" osmdroid/${OsmdroidBuildInfo.VERSION})"

		setContentView(R.layout.activity_main)

		val instance = getInstance()
		val preferences = getSharedPreferences("Pinger", 0)

		instance.load(this, preferences)

		logger.info("Setting user agent to `${userAgent}`")
		instance.userAgentValue = userAgent

		val idIndicator = findViewById<TextView>(R.id.myPingId)
		val slideUpText = findViewById<TextView>(R.id.slideUpText)
		val slideUpLayoutInner = findViewById<LinearLayout>(R.id.slideUpLayoutInner)
		val decisionText = findViewById<TextView>(R.id.decisionText)
		val decisionLayout = findViewById<ConstraintLayout>(R.id.decisionLayout)
		val decisionLayoutInner = findViewById<LinearLayout>(R.id.decisionLayoutInner)
		val getDecisionLayoutInnerOffset = { decisionLayoutInner.height.toFloat() + slideUpLayoutInner.marginBottom.toFloat() }
		val getSlideUpLayoutInnerOffset = { slideUpLayoutInner.height.toFloat() }

		idIndicator.text = getString(R.string.user_s_ping_id).replace("{id}", "000")
		idIndicator.post { idIndicator.text = getString(R.string.user_s_ping_id).replace("{id}", "...") }

		decisionLayout.post { decisionLayout.translationY = getDecisionLayoutInnerOffset() }
		var decidingOn: Pinger.Id? = null
		fun showDecision(id: Pinger.Id) {
			decisionText.post {
				decidingOn = id
				decisionText.text = getString(R.string.accept_ping_questionmark).replace("{id}", id.toString())
				SpringAnimation(decisionLayout, DynamicAnimation.TRANSLATION_Y, 0.0f).start()
			}
		}

		fun hideDecision(id: Pinger.Id) {
			decisionLayout.post {
				if (decidingOn == id) {
					SpringAnimation(
						decisionLayout,
						DynamicAnimation.TRANSLATION_Y,
						getDecisionLayoutInnerOffset()
					).start()
				}
			}
		}

		slideUpLayoutInner.post { slideUpLayoutInner.translationY = getSlideUpLayoutInnerOffset() }
		var slideUpEpoch = 0
		fun showSlideUp(text: String, @DrawableRes icon: Int = 0, showFor: Long = 5000) {
			var currentEpoch: Int? = null
			slideUpText.post {
				slideUpEpoch++
				currentEpoch = slideUpEpoch
				slideUpText.setCompoundDrawablesRelativeWithIntrinsicBounds(icon, 0, 0, 0)
				slideUpText.text = text
				SpringAnimation(slideUpLayoutInner, DynamicAnimation.TRANSLATION_Y, 0.0f).start()
			}

			slideUpLayoutInner.postDelayed({
				if (currentEpoch == slideUpEpoch) {
					SpringAnimation(
						slideUpLayoutInner,
						DynamicAnimation.TRANSLATION_Y,
						getSlideUpLayoutInnerOffset()
					).start()
				}
			}, showFor)
		}

		map = findViewById(R.id.map)
		map.isVerticalMapRepetitionEnabled = false
		map.isVerticalFadingEdgeEnabled = true
		if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
			map.overlayManager.tilesOverlay.loadingBackgroundColor = getColor(R.color.water)
			map.overlayManager.tilesOverlay.loadingLineColor = getColor(R.color.primary)
		}
		map.setScrollableAreaLimitLatitude(MapView.getTileSystem().maxLatitude, MapView.getTileSystem().minLatitude, 0)
		map.minZoomLevel = MIN_ZOOM_LEVEL
		map.maxZoomLevel = MAX_ZOOM_LEVEL
		map.setMultiTouchControls(true)
		map.setTilesScaledToDpi(MAP_QUALITY == MapQuality.BLURRY)
		map.setTileSource(object: XYTileSource(
			"OpenStreetMap",
			0,
			20,
			256,
			"",
			arrayOf(getString(R.string.tile_url)),
			getString(R.string.attribution_text),
			TileSourcePolicy(6, TileSourcePolicy.FLAG_NO_BULK or TileSourcePolicy.FLAG_NO_PREVENTIVE or TileSourcePolicy.FLAG_USER_AGENT_MEANINGFUL)
		) {
			override fun getTileURLString(tileIndex: Long): String {
				return baseUrl
					.replace("{z}", MapTileIndex.getZoom(tileIndex).toString())
					.replace("{x}", MapTileIndex.getX(tileIndex).toString())
					.replace("{y}", MapTileIndex.getY(tileIndex).toString())
			}
		})

		map.addOnFirstLayoutListener { _: View, _: Int, _: Int, _: Int, _: Int ->
			map.zoomToBoundingBox(
				INITIAL_LOCATION,
				false
			)
		}

		val locationService = getSystemService(LOCATION_SERVICE)
		val locationManager = if (locationService is LocationManager) {
			locationService
		} else {
			throw RuntimeException("LOCATION_SERVICE is not a LocationManager")
		}
		val locationProvider = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && locationManager.allProviders.contains(LocationManager.FUSED_PROVIDER)) {
			LocationManager.FUSED_PROVIDER
		} else {
			LocationManager.GPS_PROVIDER
		}

		val locationOverlay = MyLocationNewOverlay(GpsMyLocationProvider(applicationContext), map)
		map.overlays.add(locationOverlay)
		val locationPermissionRequest = registerForActivityResult(
			ActivityResultContracts.RequestMultiplePermissions()
		) { permissions ->
			when {
				permissions[Manifest.permission.ACCESS_FINE_LOCATION] ?: false -> {
					logger.info("got fine location permissions")
				}
				permissions[Manifest.permission.ACCESS_COARSE_LOCATION] ?: false -> {
					logger.info("got coarse location permissions")
				}
				else -> {
					logger.warning("no location permissions")
					return@registerForActivityResult
				}
			}

			locationOverlay.enableMyLocation()
		}

		val showLocationButton = findViewById<ImageButton>(R.id.showLocationButton)
		var showedPermissionExplanation = false
		showLocationButton.setOnClickListener {
			if (checkPermission(Manifest.permission.ACCESS_COARSE_LOCATION, Os.getpid(), Os.getuid()) != PackageManager.PERMISSION_GRANTED) {
				if (!showedPermissionExplanation && Build.VERSION.SDK_INT >= Build.VERSION_CODES.M && shouldShowRequestPermissionRationale(Manifest.permission.ACCESS_FINE_LOCATION)) {
					showSlideUp(getString(R.string.permission_explanation_map))
					showedPermissionExplanation = true
				} else {
					locationPermissionRequest.launch(
						arrayOf(
							Manifest.permission.ACCESS_FINE_LOCATION,
							Manifest.permission.ACCESS_COARSE_LOCATION
						)
					)
				}

				return@setOnClickListener
			}

			val loc = locationOverlay.myLocation
			if (loc != null) {
				val lat = loc.latitude
				val lon = loc.longitude

				val hasPreciseLocation = checkPermission(Manifest.permission.ACCESS_FINE_LOCATION, Os.getpid(), Os.getuid()) == PackageManager.PERMISSION_GRANTED

				val angleDiff = if (hasPreciseLocation) 0.01 else 0.1

				map.zoomToBoundingBox(
					BoundingBox(
						lat + angleDiff,
						lon + angleDiff,
						lat - angleDiff,
						lon - angleDiff
					),
					true
				)
			}
		}

		val pingIdInput = findViewById<EditText>(R.id.sendPingId)
		val pingButton = findViewById<Button>(R.id.sendPingButton)

		pingButton.isEnabled = false

		var firstConnection = true
		connection = Pinger.Builder(URL(getString(R.string.api_url)))
			.userAgent(userAgent)
			.onReconnecting { idIndicator.post { idIndicator.text = getString(R.string.user_s_ping_id).replace("{id}", "...") } }
			.onConnected { logger.info("connected as id $it") }
			.onConnected { idIndicator.post { idIndicator.text = getString(R.string.user_s_ping_id).replace(Regex.fromLiteral("{id}"), it.toString()) } }
			.onConnected { pingButton.post { pingButton.isEnabled = true } }
			.onConnected { if (!firstConnection) { showSlideUp(getString(R.string.reconnected), R.drawable.ping_incoming) } else { firstConnection = false } }
			.onPing { (id, info) -> logger.info("ping received from $id: $info") }
			.onPing { (id, info) -> runOnUiThread {
				showSlideUp(getString(R.string.ping_received).replace("{id}", decidingOn.toString()), R.drawable.ping_accepted)

				val marker = Marker(map)
				val elapsed = System.currentTimeMillis() / 1000 - info.ts
				marker.position = GeoPoint(info.lat, info.lon, info.alt.toDouble())
				marker.icon = AppCompatResources.getDrawable(this, R.drawable.ping_marker)
				marker.title = resources.getQuantityString(R.plurals.ping_from_seconds, elapsed.toInt())
					.replace("{id}", id.toString())
					.replace("{s}", elapsed.toString())
				marker.setAnchor(Marker.ANCHOR_CENTER, Marker.ANCHOR_BOTTOM)
				map.overlays.add(marker)

				val angleDiff = if (info.err < 100) 0.01 else 0.1
				map.zoomToBoundingBox(
					BoundingBox(
						info.lat + angleDiff,
						info.lon + angleDiff,
						info.lat - angleDiff,
						info.lon - angleDiff
					),
					true
				)

				mapUpdateTimer.schedule(object: TimerTask() {
					override fun run() {
						val secondsElapsed = System.currentTimeMillis() / 1000 - info.ts
						logger.info("updating ping label, $secondsElapsed second(s) elapsed")

						if (secondsElapsed < 60) {
							map.post {
								marker.title = resources.getQuantityString(R.plurals.ping_from_seconds, secondsElapsed.toInt())
									.replace("{id}", id.toString())
									.replace("{s}", secondsElapsed.toString())
							}
						} else {
							map.post {
								marker.title = resources.getQuantityString(R.plurals.ping_from_minutes, (secondsElapsed / 60).toInt())
									.replace("{id}", id.toString())
									.replace("{m}", (secondsElapsed / 60).toString())
							}

							this.cancel()

							mapUpdateTimer.schedule(object: TimerTask() {
								override fun run() {
									val minutesElapsed = (System.currentTimeMillis() / 1000 - info.ts) / 60
									logger.info("updating ping label, $minutesElapsed minute(s) elapsed")

									if (minutesElapsed < 60) {
										map.post {
											marker.title = resources.getQuantityString(R.plurals.ping_from_minutes, minutesElapsed.toInt())
												.replace("{id}", id.toString())
												.replace("{m}", minutesElapsed.toString())
										}
									} else {
										map.post {
											marker.title = getString(R.string.ping_from_over_1h)
												.replace("{id}", id.toString())
										}

										this.cancel()
									}
								}
							}, 60000 - System.currentTimeMillis() % 1000, 60000)
						}
					}
				}, 1000 - System.currentTimeMillis() % 1000, 1000)

				map.invalidate()
			} }
			.onIdNotFound { logger.warning("id not found: $it") }
			.onIdNotFound { pingButton.post { pingButton.isEnabled = true } }
			.onIdNotFound { showSlideUp(getString(R.string.id_not_found).replace("{id}", it.toString()), R.drawable.ping_rejected) }
			.onError { logger.warning("api error: $it") }
			.onError { showSlideUp(getString(R.string.ping_error).replace("{msg}", it), R.drawable.ping_rejected) }
			.onError { pingButton.post { pingButton.isEnabled = false } }
			.onRequest { logger.info("new ping request from $it") }
			.onRequest { showDecision(it) }
			.onRequest { showSlideUp(getString(R.string.ping_request_received).replace("{id}", it.toString()), R.drawable.ping_incoming) }
			.onRejected { logger.warning("ping rejected by $it") }
			.onRejected { pingButton.post { pingButton.isEnabled = true } }
			.onRejected { showSlideUp(getString(R.string.ping_request_rejected).replace("{id}", it.toString()), R.drawable.ping_rejected) }
			.onAccepted { showSlideUp(getString(R.string.ping_request_accepted).replace("{id}", it.toString()), R.drawable.ping_accepted) }
			.onAcknowledged { pingButton.post { pingButton.isEnabled = true } }
			.onAcknowledged { showSlideUp(getString(R.string.ping_acknowledged).replace("{id}", it.toString()), R.drawable.ping_accepted) }
			.onDecisionTimeout { logger.warning("decision timeout: $it") }
			.onDecisionTimeout { hideDecision(it) }
			.onResponseTimeout { logger.warning("response timeout: $it") }
			.onResponseTimeout { pingButton.post { pingButton.isEnabled = true } }
			.onResponseTimeout { showSlideUp(getString(R.string.response_timeout).replace("{id}", it.toString()), R.drawable.ping_rejected) }
			.onAcknowledgeTimeout { logger.warning("acknowledge timeout: $it") }
			.onAcknowledgeTimeout { pingButton.post { pingButton.isEnabled = true } }
			.onAcknowledgeTimeout { showSlideUp(getString(R.string.acknowledge_timeout).replace("{id}", it.toString()), R.drawable.ping_rejected) }
			.build()

		val acceptButton = findViewById<ImageButton>(R.id.acceptButton)
		val rejectButton = findViewById<ImageButton>(R.id.rejectButton)

		acceptButton.setOnClickListener {
			if (decidingOn != null) {
				connection.accept(decidingOn!!)
				showSlideUp(getString(R.string.ping_accepted).replace("{id}", decidingOn.toString()), R.drawable.ping_accepted)
				hideDecision(decidingOn!!)
			}
		}

		rejectButton.setOnClickListener {
			if (decidingOn != null) {
				connection.reject(decidingOn!!)
				showSlideUp(getString(R.string.ping_rejected).replace("{id}", decidingOn.toString()), R.drawable.ping_rejected)
				hideDecision(decidingOn!!)
			}
		}

		var askedForFineLocation = 0
		pingButton.setOnClickListener {
			try {
				if (askedForFineLocation < 2 && checkPermission(Manifest.permission.ACCESS_FINE_LOCATION, Os.getpid(), Os.getuid()) != PackageManager.PERMISSION_GRANTED) {
					askedForFineLocation++

					if (askedForFineLocation < 2 && Build.VERSION.SDK_INT >= Build.VERSION_CODES.M && shouldShowRequestPermissionRationale(Manifest.permission.ACCESS_FINE_LOCATION)) {
						showSlideUp(getString(R.string.permission_explanation_ping))
					} else {
						locationPermissionRequest.launch(
							arrayOf(
								Manifest.permission.ACCESS_FINE_LOCATION,
								Manifest.permission.ACCESS_COARSE_LOCATION
							)
						)
					}

					pingButton.isEnabled = true
					return@setOnClickListener
				}

				if (checkPermission(Manifest.permission.ACCESS_COARSE_LOCATION, Os.getpid(), Os.getuid()) != PackageManager.PERMISSION_GRANTED) {
					showSlideUp(getString(R.string.permission_explanation_ping))

					locationPermissionRequest.launch(
						arrayOf(
							Manifest.permission.ACCESS_FINE_LOCATION,
							Manifest.permission.ACCESS_COARSE_LOCATION
						)
					)

					pingButton.isEnabled = true
					return@setOnClickListener
				}

				pingButton.isEnabled = false
				val id = pingIdInput.text.toString()
				pingIdInput.text.clear()

				val view = currentFocus ?: View(this)
				(getSystemService(INPUT_METHOD_SERVICE) as? InputMethodManager)?.hideSoftInputFromWindow(view.windowToken, 0)
				view.clearFocus()

				//TODO: get new location (especially if no last known is available)
				val location: Location = locationManager.getLastKnownLocation(locationProvider) ?: throw RuntimeException("no location available")
				val info = PingInfo(location.time / 1000, location.latitude, location.longitude, location.altitude.toFloat(), location.accuracy)

				connection.send(Pinger.Id(id.toShort()), info)

				showSlideUp(getString(R.string.ping_request_sent).replace("{id}", id), R.drawable.ping_incoming)
			} catch (e: Exception) {
				logger.warning("error sending ping: $e")
				showSlideUp(getString(R.string.ping_error).replace("{msg}", e.message ?: e.toString()), R.drawable.ping_rejected)
				pingButton.isEnabled = true
			}
		}
	}

	override fun onResume() {
		super.onResume()
		map.onResume()
	}

	override fun onPause() {
		super.onPause()
		map.onPause()
	}
}

private enum class MapQuality {
	/** The map is slightly pixelated, but its labels are large and readable
	 *
	 * Corresponds to adjusting the map's scale to DPI (1 tile px >= 1 device px)
	 */
	BLURRY,
	/** The labels on the map are small and may be hard to read, but the map isn't pixelated
	 *
	 * Corresponds to *not* adjusting the map's scale to DPI (1 tile px = 1 device px)
	 */
	UNREADABLE,
}
