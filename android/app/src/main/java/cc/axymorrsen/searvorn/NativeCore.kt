package cc.axymorrsen.searvorn

internal object NativeCore {
    private val loaded: Boolean = runCatching {
        System.loadLibrary("searvorn_core")
        true
    }.getOrDefault(false)

    external fun nativeAbiVersion(): Int

    fun statusText(): String {
        if (!loaded) {
            return "Native core unavailable"
        }

        return runCatching {
            "Native core ABI ${nativeAbiVersion()}"
        }.getOrElse {
            "Native core handshake failed"
        }
    }
}
