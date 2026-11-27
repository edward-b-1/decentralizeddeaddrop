package com.decentralizeddeaddrop.ui

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Color as AndroidColor
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** A QR code, always black on white so that any phone can read it in either theme. */
@Composable
fun QrCode(text: String, modifier: Modifier = Modifier) {
    // One pixel for each square, enlarged without smoothing so the edges stay sharp.
    val image = remember(text) {
        val matrix = qrMatrix(text)
        val pixels = IntArray(matrix.width * matrix.height) { index ->
            if (matrix[index % matrix.width, index / matrix.width]) AndroidColor.BLACK else AndroidColor.WHITE
        }
        Bitmap.createBitmap(pixels, matrix.width, matrix.height, Bitmap.Config.ARGB_8888).asImageBitmap()
    }
    Image(
        image,
        contentDescription = "QR code",
        modifier = modifier.aspectRatio(1f).clip(RoundedCornerShape(8.dp)),
        contentScale = ContentScale.FillBounds,
        filterQuality = FilterQuality.None,
    )
}

/** A screen that looks for a QR code with the camera and hands back what it says, once. */
@Composable
fun ScanScreen(title: String, hint: String, back: () -> Unit, onScanned: (String) -> Unit) {
    val context = LocalContext.current
    var allowed by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed = it }
    LaunchedEffect(Unit) { if (!allowed) ask.launch(Manifest.permission.CAMERA) }

    Page(title = title, onBack = back, scrolls = false) {
        if (allowed) {
            Note(hint)
            CameraPreview(Modifier.weight(1f).fillMaxWidth().padding(bottom = 16.dp).clip(RoundedCornerShape(16.dp)), onScanned)
        } else {
            Note("The camera is needed to scan a code. Nothing it sees leaves this phone and no pictures are kept.")
            Primary("Allow the camera", Modifier.padding(top = 8.dp)) { ask.launch(Manifest.permission.CAMERA) }
            Note("If nothing happens, allow the camera for Dead Drop in the phone's settings. You can also go back and paste a link instead.")
        }
    }
}

@Composable
private fun CameraPreview(modifier: Modifier, onScanned: (String) -> Unit) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val report by rememberUpdatedState(onScanned)
    val found = remember { AtomicBoolean(false) }
    val view = remember { PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }

    DisposableEffect(owner) {
        val frames = Executors.newSingleThreadExecutor()
        val providers = ProcessCameraProvider.getInstance(context)
        providers.addListener({
            val preview = Preview.Builder().build()
            preview.setSurfaceProvider(view.surfaceProvider)
            val analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
            analysis.setAnalyzer(frames) { image ->
                image.use {
                    if (found.get()) return@use
                    val text = decodeQr(it.brightness(), it.width, it.height)
                    if (text != null && found.compareAndSet(false, true)) view.post { report(text) }
                }
            }
            runCatching {
                val provider = providers.get()
                provider.unbindAll()
                provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
            }
        }, ContextCompat.getMainExecutor(context))
        onDispose {
            runCatching { providers.get().unbindAll() }
            frames.shutdown()
        }
    }
    AndroidView({ view }, modifier)
}

/** The brightness plane of a camera frame, without the padding some cameras add to each row. */
private fun ImageProxy.brightness(): ByteArray {
    val plane = planes[0]
    val buffer = plane.buffer
    val out = ByteArray(width * height)
    for (row in 0 until height) {
        buffer.position(row * plane.rowStride)
        buffer.get(out, row * width, width)
    }
    return out
}
