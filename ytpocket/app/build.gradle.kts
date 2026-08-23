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

// Which ABIs the native library is built for. arm64 is the phone; x86_64 is
// the emulator, built on demand with `-PrustAbis=x86_64`.
val rustAbis: List<String> =
    (findProperty("rustAbis") as String? ?: "arm64-v8a").split(",").map(String::trim)
android {
    namespace = "dev.lucasmoy.ytpocket"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.lucasmoy.ytpocket"
        // Android 10: MediaStore's IS_PENDING flow (which is how a download
        // lands in Music/ or Movies/ without any storage permission at all)
        // and scoped storage as the only model worth targeting.
        minSdk = 29
        targetSdk = 35
        versionCode = 8
        versionName = "0.1.7"
        // arm64 only: every phone worth running this on has been arm64 for
        // years, and a second ABI doubles build time and APK size.
        // A phone is arm64; an emulator is x86_64. Release ships arm64 only
        // (the native library is most of the APK), and
        // `-PrustAbis=x86_64` builds an emulator APK -- which is the only
        // way to run the real JNI library on a real Android runtime here,
        // with no device attached.
        ndk { abiFilters += rustAbis }
        // For the on-device test below: it runs the real native library, the
        // real MediaStore and the real network on an emulator, which is the
        // only place the whole download path can be exercised without a phone.
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    signingConfigs {
        if (keystoreProperties.containsKey("storeFile")) {
            create("release") {
                storeFile = rootProject.file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
        debug { isMinifyEnabled = false }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    // buildConfig: the updater compares against BuildConfig.VERSION_NAME, so
    // the version the APK reports is the one declared above rather than a
    // string duplicated in Kotlin that can drift from it.
    buildFeatures {
        compose = true
        buildConfig = true
    }
    testOptions {
        unitTests.isReturnDefaultValues = true
        // Robolectric needs the real resources (themes, strings) to inflate
        // an activity.
        unitTests.isIncludeAndroidResources = true
    }

    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")
    packaging { resources.excludes += "/META-INF/{AL2.0,LGPL2.1}" }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    // Thumbnails: a search result without one is a wall of text, and hand
    // rolling an image cache for a list that scrolls is not worth it.
    implementation(libs.coil.compose)

    testImplementation("junit:junit:4.13.2")
    // Robolectric runs the real Activity on the JVM, which is the only way
    // to catch "the app closes the moment you open it" without a device --
    // exactly the class of bug that shipped in 0.1.0.
    testImplementation("org.robolectric:robolectric:4.14.1")
    testImplementation("androidx.test:core:1.6.1")
    // The real org.json: the one in android.jar is a stub that throws in
    // unit tests, and parsing what the native side returns is exactly what
    // those tests are for.
    testImplementation("org.json:json:20240303")

    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
}

// ---- Rust ------------------------------------------------------------
//
// ../jni holds the parts the platform cannot do: YouTube search and stream
// resolution (rustypipe), safe filenames, and MP3 encoding (symphonia +
// LAME). Gradle builds it, so there is one build command for the app.
//
// NDK r26 on purpose: `whisper-rs-sys`' sibling problem bites here too --
// r27 dropped the reverse processor->ABI map from its `abis.cmake`, and any
// dependency that hands CMake a `CMAKE_SYSTEM_PROCESSOR` then fails with
// "Unknown processor 'aarch64'".
val ndkVersionForRust = "26.3.11579264"
val jniLibsDir = layout.projectDirectory.dir("src/main/jniLibs")

val cargoNdkBuild by tasks.registering(Exec::class) {
    group = "build"
    description = "Cross-compiles ytpocket-jni (YouTube + MP3) for $rustAbis"
    workingDir = rootProject.layout.projectDirectory.dir("jni").asFile
    val sdkDir = android.sdkDirectory.absolutePath
    environment("ANDROID_HOME", sdkDir)
    environment("ANDROID_NDK_HOME", "$sdkDir/ndk/$ndkVersionForRust")
    // CMake's own Android support reads the NDK location from this one.
    environment("ANDROID_NDK_ROOT", "$sdkDir/ndk/$ndkVersionForRust")
    commandLine(
        buildList {
            add("cargo"); add("ndk")
            rustAbis.forEach { add("-t"); add(it) }
            add("--platform"); add("29")
            add("-o"); add(jniLibsDir.asFile.absolutePath)
            add("build"); add("--release")
        }
    )
    inputs.property("abis", rustAbis)
    inputs.dir(rootProject.layout.projectDirectory.dir("jni/src"))
    inputs.file(rootProject.layout.projectDirectory.file("jni/Cargo.toml"))
    outputs.dir(jniLibsDir)
}

tasks.named("preBuild") { dependsOn(cargoNdkBuild) }
