# Searvorn Android shell

The Android shell is intentionally thin. It uses framework APIs and a custom Canvas view rather than Compose/AppCompat/AndroidX runtime dependencies.

Current baseline:

- Android Gradle Plugin 9.4.0
- compileSdk / targetSdk 37 (Android 17)
- minSdk 26
- built-in Kotlin from AGP 9.x
- arm64-v8a native core built from `searvorn-core`
- NDK 29.0.14206865
- no app runtime dependencies

The Gradle `preBuild` task invokes `scripts/build-android-native.sh`. The script takes no command-line parameters; optional configuration is supplied through environment variables.
