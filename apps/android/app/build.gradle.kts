plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "io.github.peavey2787.p2pnet"
    compileSdk {
        version = release(37) {
            minorApiLevel = 0
        }
    }
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "io.github.peavey2787.p2pnet"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }

        externalNativeBuild {
            cmake {
                cppFlags += listOf("-std=c++20", "-Wall", "-Wextra", "-Werror")
            }
        }
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
        }
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    buildFeatures {
        compose = true
        buildConfig = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlin {
        compilerOptions {
            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
        }
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
            version = "3.22.1"
        }
    }

    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

val isWindowsHost = System.getProperty("os.name").startsWith("Windows", ignoreCase = true)
val rustupExecutable = if (isWindowsHost) "rustup.exe" else "rustup"
val rustProject = rootProject.projectDir.resolve("native").canonicalFile
val jniOutput = project.projectDir.resolve("src/main/jniLibs").canonicalFile
val toolingVerifier = rootProject.projectDir.resolve(
    if (isWindowsHost) "qa/verify-rust-tooling.ps1" else "qa/verify-rust-tooling.sh",
).canonicalFile

val verifyRustAndroidTooling by tasks.registering(Exec::class) {
    group = "verification"
    description = "Verify the pinned Rust toolchain and cargo-ndk used by Android builds."
    workingDir = rootProject.projectDir
    if (isWindowsHost) {
        commandLine(
            "powershell.exe",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            toolingVerifier.absolutePath,
        )
    } else {
        commandLine("bash", toolingVerifier.absolutePath)
    }
}

val buildRustAndroid by tasks.registering(Exec::class) {
    group = "build"
    description = "Build the p2p-net Rust core for the supported Android ABIs."
    dependsOn(verifyRustAndroidTooling)
    workingDir = rustProject
    environment("CARGO_INCREMENTAL", "0")
    commandLine(
        rustupExecutable,
        "run",
        "1.98.0",
        "cargo",
        "ndk",
        "-t",
        "arm64-v8a",
        "-t",
        "x86_64",
        "-o",
        jniOutput.absolutePath,
        "build",
        "--release",
        "--locked",
    )
}

tasks.named("preBuild").configure {
    dependsOn(buildRustAndroid)
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.08.00"))
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.core:core-ktx:1.17.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.10.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.10.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
    testImplementation("org.jetbrains.kotlin:kotlin-test-junit:2.3.21")
    debugImplementation("androidx.compose.ui:ui-tooling")
}
