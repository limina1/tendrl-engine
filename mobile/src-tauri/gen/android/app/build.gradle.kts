import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// Release signing config lives OUTSIDE git: gen/android/keystore.properties
// (gitignored) points at the keystore. scripts/build-android.sh --init-signing
// creates both. Absent the file, release builds come out unsigned (debug
// builds are untouched either way).
val keystoreProperties = Properties().apply {
    val propFile = rootProject.file("keystore.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 36
    namespace = "io.github.limina1.tendrl"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "io.github.limina1.tendrl"
        minSdk = 24
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    signingConfigs {
        if (keystoreProperties.containsKey("storeFile")) {
            create("release") {
                keyAlias = keystoreProperties["keyAlias"] as String
                keyPassword = keystoreProperties["password"] as String
                storeFile = file(keystoreProperties["storeFile"] as String)
                storePassword = keystoreProperties["password"] as String
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            // null when keystore.properties is absent → unsigned, as before.
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    // Modern packaging (the AGP default, restated for intent): native libs
    // stay uncompressed and page-aligned inside the APK, which is what the
    // Android 15+ 16 KB check wants — and ort dlopens libonnxruntime.so by
    // soname through the app's linker namespace, so no extraction is needed
    // (an earlier useLegacyPackaging=true here compressed the libs and
    // tripped the 16 KB "not aligned" dialog).
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    // ONNX Runtime for on-device embeddings: ort (Rust) is built with
    // load-dynamic and dlopens the libonnxruntime.so this AAR packs into
    // the APK. 1.22.0 (not the 1.20.0 that ort-sys nominally targets)
    // because 1.20.0's prebuilt .so predates Android 15's 16 KB page
    // alignment and fails the install check; ONNX Runtime's GetApi is
    // backward compatible, so a newer runtime still serves ort's
    // ORT_API_VERSION=20 request. Do not downgrade below 1.21 (alignment).
    implementation("com.microsoft.onnxruntime:onnxruntime-android:1.22.0")
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")