package cc.axymorrsen.searvorn

import android.os.ParcelFileDescriptor

internal object NativeCore {
    private val loaded: Boolean = runCatching {
        System.loadLibrary("searvorn_core")
        true
    }.getOrDefault(false)

    external fun nativeAbiVersion(): Int

    private external fun nativeAdoptFd(fd: Int): Long

    private external fun nativeHandleLen(handle: Long): Long

    private external fun nativeReleaseHandle(handle: Long): Int

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

    fun consumeDetachedFdLength(fd: Int): Long {
        if (!loaded) {
            ParcelFileDescriptor.adoptFd(fd).close()
            return -1
        }

        val handle = nativeAdoptFd(fd)
        if (handle == 0L) {
            return -1
        }

        return try {
            nativeHandleLen(handle)
        } finally {
            nativeReleaseHandle(handle)
        }
    }
}
