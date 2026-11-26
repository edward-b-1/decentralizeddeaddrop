plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "com.decentralizeddeaddrop"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.decentralizeddeaddrop"
        // Android 12+: hardware-backed key agreement keys, as in Pocket Radio.
        minSdk = 31
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        // The engine (Rust) is built for phones and for the emulator: see build-engine.sh.
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation("${libs.jna.get()}@aar")

    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons.core)
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
}
