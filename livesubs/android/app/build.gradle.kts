import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

/**
 * Release signing, read from `../keystore.properties` (gitignored, along
 * with the .jks itself). Same convention as vaultexplorer, and for the same
 * reason: an APK sideloaded onto a phone can only be *updated* in place by
 * an APK signed with the identical key, so the key has to outlive any one
 * build. Without the file, release builds fall back to the debug key --
 * installable, but a dead end for updates.
 */
val keystoreProperties = Properties().apply {
    val file = rootProject.file("keystore.properties")
    if (file.exists()) file.inputStream().use { stream -> load(stream) }
}

android {
    namespace = "dev.lucasmoy.livesubs"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.lucasmoy.livesubs"
        // Android 10: the first version with AudioPlaybackCapture, which is
        // the only way an app can hear what another app is playing. Below
        // that, half the app could not exist.
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        // Only arm64 is built: every Android phone worth running whisper on
        // has been arm64 for years, and a second ABI doubles both the build
        // time and the APK for nothing.
        ndk { abiFilters += "arm64-v8a" }
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
        debug {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    // buildConfig: the updater compares against BuildConfig.VERSION_NAME, so
    // the version the APK reports is the one declared above -- not a string
    // duplicated in Kotlin that can drift from it.
    buildFeatures {
        compose = true
        buildConfig = true
    }
    testOptions { unitTests.isReturnDefaultValues = true }

    // The Rust library is built into src/main/jniLibs by the task below.
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")

    packaging {
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
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
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    // On-device translation, offline once the ~30MB model per language is
    // downloaded. Needs Play Services on the phone -- see the README.
    implementation(libs.mlkit.translate)

    testImplementation("junit:junit:4.13.2")
    // The real org.json, because the one in android.jar is a stub that
    // throws in unit tests -- and parsing what the native side returns is
    // exactly what these tests are for.
    testImplementation("org.json:json:20240303")
}

// ---- Rust ------------------------------------------------------------
//
// The VAD and whisper.cpp live in ../jni (which wraps ../../core, shared
// with the desktop app). Gradle builds them with cargo-ndk before it
// assembles anything, so there is one build command for the whole app
// rather than "remember to run cargo first".
//
// NDK r26 on purpose, not the newest: `whisper-rs-sys` passes
// `CMAKE_SYSTEM_PROCESSOR` to CMake, and r27 dropped the reverse
// processor->ABI map (`NDK_PROC_aarch64_ABI`) from its `abis.cmake`, so
// CMake fails with "Unknown processor 'aarch64'". r26 still ships it.
val ndkVersionForRust = "26.3.11579264"
val jniLibsDir = layout.projectDirectory.dir("src/main/jniLibs")

val cargoNdkBuild by tasks.registering(Exec::class) {
    group = "build"
    description = "Cross-compiles livesubs-jni (VAD + whisper.cpp) for arm64-v8a"
    workingDir = rootProject.layout.projectDirectory.dir("jni").asFile
    val sdkDir = android.sdkDirectory.absolutePath
    environment("ANDROID_HOME", sdkDir)
    environment("ANDROID_NDK_HOME", "$sdkDir/ndk/$ndkVersionForRust")
    // CMake's own Android support reads the NDK location from this one.
    environment("ANDROID_NDK_ROOT", "$sdkDir/ndk/$ndkVersionForRust")
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a",
        "--platform", "29",
        "-o", jniLibsDir.asFile.absolutePath,
        "build", "--release"
    )
    // Rebuild when the Rust sources change, not on every Gradle run.
    inputs.dir(rootProject.layout.projectDirectory.dir("jni/src"))
    inputs.file(rootProject.layout.projectDirectory.file("jni/Cargo.toml"))
    inputs.dir(rootProject.layout.projectDirectory.dir("../core/src"))
    outputs.dir(jniLibsDir)
}

/**
 * `whisper-rs-sys` links the NDK's shared C++ runtime, so the .so declares a
 * dependency on `libc++_shared.so` that nothing else in the APK provides --
 * without this the app dies on `System.loadLibrary` with "library
 * libc++_shared.so not found". Copying it out of the NDK sysroot is the
 * documented fix; the alternative (a statically linked libc++) means
 * patching whisper.cpp's build.
 */
val copyLibCxxShared by tasks.registering(Copy::class) {
    group = "build"
    description = "Bundles the NDK's libc++_shared.so, which liblivesubs.so links against"
    val sdkDir = android.sdkDirectory.absolutePath
    from("$sdkDir/ndk/$ndkVersionForRust/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so")
    into(jniLibsDir.dir("arm64-v8a"))
    dependsOn(cargoNdkBuild)
}

tasks.named("preBuild") { dependsOn(copyLibCxxShared) }
