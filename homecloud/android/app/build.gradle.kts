import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

/**
 * Release signing, read from `../keystore.properties` (gitignored, along with
 * the .jks). Same convention as the sibling apps: a sideloaded APK can only be
 * *updated* in place by one signed with the identical key, so the key has to
 * outlive any single build. Without the file, release builds fall back to the
 * debug key -- installable, but a dead end for updates.
 */
val keystoreProperties = Properties().apply {
    val file = rootProject.file("keystore.properties")
    if (file.exists()) file.inputStream().use { stream -> load(stream) }
}

// Which ABIs the native pieces are built for. arm64 is a phone; x86_64 is the
// emulator, built on demand with `-PrustAbis=x86_64`.
val rustAbis: List<String> =
    (findProperty("rustAbis") as String? ?: "arm64-v8a").split(",").map(String::trim)

android {
    namespace = "dev.lucasmoy.homecloud"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.lucasmoy.homecloud"
        // Android 10 is where execution from the app's own data directory was
        // blocked, which is exactly why the engine ships as a library and is run
        // from nativeLibraryDir. Below that the layout would have to differ.
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += rustAbis }
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
            isMinifyEnabled = false
        }
        debug { isMinifyEnabled = false }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true }

    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")
    // The engine is a real executable, not a library Android should ever try to
    // load. It has to stay a separate file on disk for the app to exec it, so it
    // must not be compressed into the APK and left there.
    packaging {
        jniLibs.useLegacyPackaging = true
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
    testOptions {
        unitTests.isReturnDefaultValues = true
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
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
}

// ---- Rust ------------------------------------------------------------
//
// ../jni wraps homecore, the same crate the desktop app links against, so a
// pairing code written by one is read by the other by construction rather than
// by two implementations agreeing.
val ndkVersionForRust = "26.3.11579264"
val jniLibsDir = layout.projectDirectory.dir("src/main/jniLibs")

val cargoNdkBuild by tasks.registering(Exec::class) {
    group = "build"
    description = "Cross-compiles the homecore JNI bridge for $rustAbis"
    workingDir = rootProject.layout.projectDirectory.dir("jni").asFile
    val sdkDir = android.sdkDirectory.absolutePath
    environment("ANDROID_HOME", sdkDir)
    environment("ANDROID_NDK_HOME", "$sdkDir/ndk/$ndkVersionForRust")
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
    inputs.dir(rootProject.layout.projectDirectory.dir("../core/src"))
    outputs.dir(jniLibsDir)
}

tasks.named("preBuild") { dependsOn(cargoNdkBuild) }
