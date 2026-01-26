plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.jetbrains.kotlin.android) apply false
    alias(libs.plugins.serialization) apply false
}
val compileSdkVersion by extra(36)
