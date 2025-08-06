import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import java.time.Instant
import java.time.ZoneOffset
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter

plugins {
	alias(libs.plugins.android.application)
	alias(libs.plugins.jetbrains.kotlin.android)
	alias(libs.plugins.serialization)
}

android {
	namespace = "dev.janm.pinger"
	compileSdk = 36
	buildToolsVersion = "36.0.0"
	ndkVersion = "27.0.12077973"

	defaultConfig {
		val now = System.currentTimeMillis()

		applicationId = "dev.janm.pinger"
		minSdk = 21
		targetSdk = 36
		versionCode = (now / 10000).toInt()
		versionName = DateTimeFormatter.ofPattern("uuuu.MM.dd.A").format(ZonedDateTime.ofInstant(Instant.ofEpochMilli(now), ZoneOffset.UTC))
		signingConfig = signingConfigs.getByName("debug")
	}

	buildTypes {
		release {
			isMinifyEnabled = true
			proguardFiles(
				getDefaultProguardFile("proguard-android-optimize.txt"),
				"proguard-rules.pro"
			)
			isJniDebuggable = false
			isDebuggable = false
		}

		getByName("debug") {
			versionNameSuffix = "-dev"
			isDebuggable = true
			isJniDebuggable = true
			isMinifyEnabled = false
			signingConfig = signingConfigs.getByName("debug")
		}
	}

	compileOptions {
		sourceCompatibility = JavaVersion.VERSION_1_8
		targetCompatibility = JavaVersion.VERSION_1_8
	}

	kotlin {
		compilerOptions {
			jvmTarget = JvmTarget.JVM_1_8
		}
	}

	buildFeatures {
		viewBinding = true
	}

	dependenciesInfo {
		includeInApk = true
		includeInBundle = true
	}

	externalNativeBuild {
		cmake {
			path = file("src/main/rust/CMakeLists.txt")
			version = "3.22.1"
		}
	}

	sourceSets {
		getByName("main") {
			java.srcDir("../../lib/src/java_ffi")
		}
	}
}

dependencies {
	implementation(libs.androidx.core.ktx)
	implementation(libs.androidx.appcompat)
	implementation(libs.material)
	implementation(libs.androidx.constraintlayout)
	implementation(libs.androidx.lifecycle.livedata.ktx)
	implementation(libs.androidx.lifecycle.viewmodel.ktx)
	implementation(libs.androidx.navigation.fragment.ktx)
	implementation(libs.androidx.navigation.ui.ktx)
	implementation(libs.osmdroid.android)
	implementation(libs.okhttp)
	implementation(libs.json)
	implementation(libs.androidx.browser)
}
