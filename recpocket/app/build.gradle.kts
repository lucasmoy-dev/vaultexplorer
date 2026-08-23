import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

/**
 * Release signing, read from `../keystore.properties` (gitignored, along
 * with the .jks). Same convention as the sibling apps: a sideloaded APK can
 * only be *updated* in place by one signed with the identical key, so the
 * key has to outlive any single build. Without the file, release builds fall
 * back to the debug key -- installable, but a dead end for updates.
 */
val keystoreProperties = Properties().apply {
    val file = rootProject.file("keystore.properties")
    if (file.exists()) file.inputStream().use { stream -> load(stream) }
}

android {
    namespace = "dev.lucasmoy.recpocket"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.lucasmoy.recpocket"
        // Android 10 is the floor for a reason: `AudioPlaybackCapture` (how
        // the *other* side of a call, a video or a meeting is recorded at
        // all) arrived in 29, and so did MediaStore's IS_PENDING flow that
        // lets a finished recording appear in the gallery with no storage
        // permission whatsoever.
        minSdk = 29
        targetSdk = 35
        versionCode = 3
        versionName = "0.1.2"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    signingConfigs {
        create("release") {
            val storeFilePath = keystoreProperties.getProperty("storeFile")
            if (storeFilePath != null) {
                storeFile = rootProject.file(storeFilePath)
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig =
                if (keystoreProperties.getProperty("storeFile") != null) {
                    signingConfigs.getByName("release")
                } else {
                    signingConfigs.getByName("debug")
                }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    testOptions {
        unitTests.isReturnDefaultValues = true
        // Robolectric needs the real resources (themes, strings) and the
        // real manifest to inflate an activity -- without this it cannot
        // even resolve MainActivity.
        unitTests.isIncludeAndroidResources = true
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.material3)

    testImplementation("junit:junit:4.13.2")
    // The rules worth testing here are arithmetic and text: file names,
    // mixing two PCM streams, quality presets, and deciding from a
    // notification whether a call is voice or video. All of them run on the
    // JVM; Robolectric is only needed for the one that touches Compose.
    testImplementation("org.robolectric:robolectric:4.14.1")
    testImplementation("androidx.test:core:1.6.1")

    // The recording pipeline itself only exists on a device: AudioRecord,
    // MediaCodec and MediaMuxer have no JVM stand-ins worth testing against.
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    // GrantPermissionRule: the microphone permission has to be granted by
    // the test itself -- Gradle uninstalls the app after every connected
    // run, which takes any grant with it.
    androidTestImplementation("androidx.test:rules:1.6.1")
}
