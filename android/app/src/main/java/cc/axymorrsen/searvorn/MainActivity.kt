package cc.axymorrsen.searvorn

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.os.Build
import android.os.Bundle
import android.view.MotionEvent
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsController

class MainActivity : Activity() {
    private lateinit var homeView: HomeView
    private lateinit var safTree: SafTree

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false)
            window.insetsController?.systemBarsBehavior =
                WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        } else {
            @Suppress("DEPRECATION")
            window.decorView.systemUiVisibility =
                View.SYSTEM_UI_FLAG_LAYOUT_STABLE or
                    View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or
                    View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
        }

        safTree = SafTree(this)
        homeView = HomeView(this, ::selectFileTree, ::selectBinaryFile)
        setContentView(homeView)
    }

    @Deprecated("Framework callback retained to avoid an AndroidX runtime dependency")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)

        if (resultCode != RESULT_OK) {
            return
        }

        val uri = data?.data ?: return
        val flags = data.flags
        when (requestCode) {
            REQUEST_TREE -> openTree(uri, flags)
            REQUEST_FILE -> openBinaryFile(uri, flags)
        }
    }

    private fun openTree(uri: android.net.Uri, flags: Int) {
        homeView.setFilesStatus("Opening…")
        Thread(
            {
                val status =
                    runCatching {
                        safTree.persist(uri, flags)
                        val snapshot = safTree.snapshot(uri)
                        val suffix = if (snapshot.truncated) "+" else ""
                        "${snapshot.rootName} · ${snapshot.entries}$suffix entries"
                    }.getOrElse {
                        "Unable to open selected tree"
                    }

                runOnUiThread {
                    homeView.setFilesStatus(status)
                }
            },
            "Searvorn-SAF-Tree",
        ).start()
    }

    private fun openBinaryFile(uri: android.net.Uri, flags: Int) {
        homeView.setBinaryStatus("Opening…")
        Thread(
            {
                val status =
                    runCatching {
                        safTree.persist(uri, flags)
                        val name = safTree.displayName(uri)
                        val fd = safTree.detachReadFd(uri)
                        val len = NativeCore.consumeDetachedFdLength(fd)
                        if (len >= 0) {
                            "$name · $len bytes · native"
                        } else {
                            "$name · native unavailable"
                        }
                    }.getOrElse {
                        "Unable to open selected file"
                    }

                runOnUiThread {
                    homeView.setBinaryStatus(status)
                }
            },
            "Searvorn-SAF-File",
        ).start()
    }
    private fun selectFileTree() {
        startActivityForResult(safTree.pickerIntent(), REQUEST_TREE)
    }

    private fun selectBinaryFile() {
        startActivityForResult(safTree.filePickerIntent(), REQUEST_FILE)
    }

    private companion object {
        const val REQUEST_TREE = 0x5301
        const val REQUEST_FILE = 0x5302
    }
}

private class HomeView(
    context: Context,
    private val onFilesClick: () -> Unit,
    private val onBinaryClick: () -> Unit,
) : View(context) {
    private val density = resources.displayMetrics.density
    private val night =
        resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
            Configuration.UI_MODE_NIGHT_YES
    private val background = if (night) Color.rgb(18, 18, 20) else Color.rgb(247, 247, 250)
    private val surface = if (night) Color.rgb(31, 31, 35) else Color.WHITE
    private val primaryText = if (night) Color.rgb(242, 242, 245) else Color.rgb(24, 24, 28)
    private val secondaryText = if (night) Color.rgb(174, 174, 182) else Color.rgb(99, 99, 106)
    private val accent = if (night) Color.rgb(159, 197, 255) else Color.rgb(36, 107, 214)

    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val card = RectF()
    private val filesCard = RectF()
    private val binaryCard = RectF()
    private var insetTop = 0
    private var insetBottom = 0
    private var filesStatus = "Choose a document tree"
    private var binaryStatus = "Choose a file"

    init {
        isClickable = true
        isFocusable = true
        contentDescription = "Searvorn workspace"
        setBackgroundColor(background)
        setOnApplyWindowInsetsListener { _, insets ->
            if (Build.VERSION.SDK_INT >= 30) {
                val systemBars = insets.getInsets(WindowInsets.Type.systemBars())
                insetTop = systemBars.top
                insetBottom = systemBars.bottom
            } else {
                @Suppress("DEPRECATION")
                insetTop = insets.systemWindowInsetTop
                @Suppress("DEPRECATION")
                insetBottom = insets.systemWindowInsetBottom
            }
            invalidate()
            insets
        }
    }

    fun setFilesStatus(status: String) {
        filesStatus = status
        invalidate()
    }

    fun setBinaryStatus(status: String) {
        binaryStatus = status
        invalidate()
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.action == MotionEvent.ACTION_UP) {
            when {
                filesCard.contains(event.x, event.y) -> {
                    performClick()
                    onFilesClick()
                    return true
                }
                binaryCard.contains(event.x, event.y) -> {
                    performClick()
                    onBinaryClick()
                    return true
                }
            }
        }
        return true
    }

    override fun performClick(): Boolean {
        super.performClick()
        return true
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)

        val left = dp(20f)
        var y = insetTop + dp(28f)

        drawText(canvas, "Searvorn", left, y, dp(31f), primaryText, true)
        y += dp(30f)
        drawText(
            canvas,
            "Clean-room Android binary workbench",
            left,
            y,
            dp(14f),
            secondaryText,
            false,
        )

        y += dp(38f)
        drawStatusCard(canvas, left, y)
        y += dp(90f)

        val gap = dp(12f)
        val contentWidth = width - left * 2
        val column = (contentWidth - gap) / 2f

        filesCard.set(left, y, left + column, y + dp(104f))
        drawFeatureCard(canvas, "Files", filesStatus, filesCard)
        binaryCard.set(left + column + gap, y, left + column * 2 + gap, y + dp(104f))
        drawFeatureCard(canvas, "Binary", binaryStatus, binaryCard)
        y += dp(104f) + gap
        drawFeatureCard(
            canvas,
            "Workspace",
            "Diff · patch · audit",
            RectF(left, y, left + column, y + dp(104f)),
        )
        drawFeatureCard(
            canvas,
            "Terminal",
            "Bounded execution",
            RectF(left + column + gap, y, left + column * 2 + gap, y + dp(104f)),
        )

        val footerY = (height - insetBottom - dp(24f)).coerceAtLeast(y + dp(128f))
        drawText(
            canvas,
            "Phase 1 bootstrap · local-first · no telemetry",
            left,
            footerY,
            dp(12f),
            secondaryText,
            false,
        )
    }

    private fun drawStatusCard(canvas: Canvas, left: Float, top: Float) {
        val right = width - left
        card.set(left, top, right, top + dp(72f))
        paint.color = surface
        canvas.drawRoundRect(card, dp(20f), dp(20f), paint)

        drawText(canvas, "Core", left + dp(18f), top + dp(27f), dp(13f), secondaryText, false)
        drawText(
            canvas,
            NativeCore.statusText(),
            left + dp(18f),
            top + dp(51f),
            dp(16f),
            primaryText,
            true,
        )

        paint.color = accent
        canvas.drawCircle(right - dp(24f), top + dp(36f), dp(5f), paint)
    }

    private fun drawFeatureCard(
        canvas: Canvas,
        title: String,
        subtitle: String,
        bounds: RectF,
    ) {
        paint.color = surface
        canvas.drawRoundRect(bounds, dp(20f), dp(20f), paint)

        drawText(
            canvas,
            title,
            bounds.left + dp(16f),
            bounds.top + dp(37f),
            dp(18f),
            primaryText,
            true,
        )
        drawText(
            canvas,
            subtitle.take(28),
            bounds.left + dp(16f),
            bounds.top + dp(65f),
            dp(12f),
            secondaryText,
            false,
        )
    }

    private fun drawText(
        canvas: Canvas,
        text: String,
        x: Float,
        baseline: Float,
        size: Float,
        color: Int,
        bold: Boolean,
    ) {
        paint.color = color
        paint.textSize = size
        paint.typeface =
            android.graphics.Typeface.create(
                "sans",
                if (bold) android.graphics.Typeface.BOLD else android.graphics.Typeface.NORMAL,
            )
        canvas.drawText(text, x, baseline, paint)
    }

    private fun dp(value: Float): Float = value * density
}
