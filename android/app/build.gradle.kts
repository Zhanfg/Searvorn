plugins {
    id("com.android.application")
}

val searvornNdkVersion = "29.0.14206865"
val repositoryRoot = rootProject.projectDir.parentFile
val generatedJniLibs = layout.buildDirectory.dir("generated/jniLibs")
val skipNative = providers.gradleProperty("searvornSkipNative").isPresent
val androidSdkLevel = providers.gradleProperty("searvornSdkLevel").orNull?.toInt() ?: 37

android {
    namespace = "cc.axymorrsen.searvorn"
    compileSdk = androidSdkLevel
    ndkVersion = searvornNdkVersion

    defaultConfig {
        applicationId = "cc.axymorrsen.searvorn"
        minSdk = 26
        targetSdk = androidSdkLevel
        versionCode = 1
        versionName = "0.0.1"
    }

    buildFeatures {
        buildConfig = false
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDir(generatedJniLibs)
        }
    }

    buildTypes {
        getByName("debug") {
            isMinifyEnabled = false
        }

        getByName("release") {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    lint {
        abortOnError = true
        checkReleaseBuilds = true
    }
}

val buildRustArm64 by tasks.registering(Exec::class) {
    workingDir(repositoryRoot)
    environment("SEARVORN_ANDROID_OUT", generatedJniLibs.get().asFile.absolutePath)
    environment("SEARVORN_NDK_VERSION", searvornNdkVersion)
    commandLine("sh", "scripts/build-android-native.sh")

    inputs.files(
        fileTree(repositoryRoot.resolve("crates/searvorn-core/src")) {
            include("**/*.rs")
        },
    )
    inputs.file(repositoryRoot.resolve("crates/searvorn-core/Cargo.toml"))
    inputs.file(repositoryRoot.resolve("Cargo.toml"))
    inputs.file(repositoryRoot.resolve("Cargo.lock"))
    inputs.file(repositoryRoot.resolve("rust-toolchain.toml"))
    outputs.dir(generatedJniLibs)
}

if (!skipNative) {
    tasks.named("preBuild") {
        dependsOn(buildRustArm64)
    }
}
