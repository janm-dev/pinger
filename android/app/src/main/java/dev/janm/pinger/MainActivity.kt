package dev.janm.pinger

import android.content.pm.PackageManager
import android.location.LocationManager
import android.os.Build
import android.os.Bundle
import android.preference.PreferenceManager
import android.system.Os
import android.view.View
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import dev.janm.pinger.KeyExchange
import dev.janm.pinger.PingInfo
import org.osmdroid.OsmdroidBuildInfo
import org.osmdroid.util.BoundingBox
import org.osmdroid.views.MapView
import org.osmdroid.views.overlay.mylocation.GpsMyLocationProvider
import org.osmdroid.views.overlay.mylocation.MyLocationNewOverlay
import org.osmdroid.config.Configuration.*
import org.osmdroid.tileprovider.tilesource.TileSourcePolicy
import org.osmdroid.tileprovider.tilesource.XYTileSource
import org.osmdroid.util.MapTileIndex
import java.util.logging.Logger

private const val MIN_ZOOM_LEVEL = 2.0
private const val MAX_ZOOM_LEVEL = 22.0
private const val REQUEST_PERMISSIONS_REQUEST_CODE = 1
private val INITIAL_LOCATION = BoundingBox(75.0, 0.0, -75.0, -0.0)
private val MAP_QUALITY = MapQuality.UNREADABLE

class MainActivity: AppCompatActivity() {
	private lateinit var logger: Logger
	private lateinit var map: MapView

	override fun onCreate(savedInstanceState: Bundle?) {
		super.onCreate(savedInstanceState)

		logger = Logger.getLogger(resources.getString(R.string.app_name))

		val userAgent = "${resources.getString(R.string.app_name)}/${packageManager.getPackageInfo(packageName, 0).versionName}" +
			" (Android ${Build.VERSION.RELEASE};" +
			" osmdroid/${OsmdroidBuildInfo.VERSION})"

		setContentView(R.layout.activity_main)

		val instance = getInstance();

		instance.load(this, PreferenceManager.getDefaultSharedPreferences(this))

		logger.info("Setting user agent to `${userAgent}`")
		instance.userAgentValue = userAgent

		map = findViewById(R.id.map)
		map.isVerticalMapRepetitionEnabled = false
		map.isVerticalFadingEdgeEnabled = true
		map.setScrollableAreaLimitLatitude(MapView.getTileSystem().maxLatitude, MapView.getTileSystem().minLatitude, 0)
		map.minZoomLevel = MIN_ZOOM_LEVEL
		map.maxZoomLevel = MAX_ZOOM_LEVEL
		map.setMultiTouchControls(true)
		map.setTileSource(object: XYTileSource(
			"OpenStreetMap",
			0,
			20,
			256,
			"",
			arrayOf("https://tile.openstreetmap.org/{z}/{x}/{y}.png"),
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

		map.setMultiTouchControls(true);
		map.setTilesScaledToDpi(MAP_QUALITY == MapQuality.BLURRY);

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

		if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M && shouldShowRequestPermissionRationale("android.permission.ACCESS_FINE_LOCATION")) {
			logger.warning("TODO: explain perms")
		}

		registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted: Boolean ->
			if (granted && checkPermission("android.permission.ACCESS_FINE_LOCATION", Os.getpid(), Os.getuid()) == PackageManager.PERMISSION_GRANTED) {
				val locationOverlay = MyLocationNewOverlay(GpsMyLocationProvider(applicationContext), map)
				locationOverlay.enableMyLocation()
				map.overlays.add(locationOverlay)

				locationManager.requestLocationUpdates(
					if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && locationManager.allProviders.contains(LocationManager.FUSED_PROVIDER)) {
						LocationManager.FUSED_PROVIDER
					} else {
						LocationManager.GPS_PROVIDER
					},
					1000L,
					1.0F
				) {
					logger.info("New location update: $it")
				}
			} else {
				logger.warning("no perms")
			}
		}

		var info = PingInfo(1, 2.3, 4.5, 6.7f, 8.9f)
	}

	override fun onResume() {
		super.onResume()
		map.onResume()
	}

	override fun onPause() {
		super.onPause()
		map.onPause()
	}

	override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
		super.onRequestPermissionsResult(requestCode, permissions, grantResults)

		val permissionsToRequest = ArrayList<String>()
		var i = 0

		while (i < grantResults.size) {
			permissionsToRequest.add(permissions[i])
			i++
		}

		if (permissionsToRequest.size > 0) {
			ActivityCompat.requestPermissions(
				this,
				permissionsToRequest.toTypedArray(),
				REQUEST_PERMISSIONS_REQUEST_CODE
			)
		}
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
