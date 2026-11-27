package com.decentralizeddeaddrop.ui

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.EncodeHintType
import com.google.zxing.LuminanceSource
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.BitMatrix
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

// QR codes are made and read on the phone itself (the ZXing library). Nothing the
// camera sees leaves the phone, and no Google service is involved.

/** The squares of a QR code holding [text], with its quiet border. */
fun qrMatrix(text: String): BitMatrix = QRCodeWriter().encode(
    text,
    BarcodeFormat.QR_CODE,
    0,
    0,
    mapOf(EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M, EncodeHintType.MARGIN to 2),
)

private val READ_HINTS = mapOf(DecodeHintType.TRY_HARDER to true)

private fun read(source: LuminanceSource): String? = try {
    QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source)), READ_HINTS).text
} catch (_: ReaderException) {
    null
}

/**
 * Reads a QR code out of a picture given as brightness values, row by row. Null if there is none.
 * A code drawn light on dark is read too: a server prints its link that way in a terminal.
 */
fun decodeQr(luminance: ByteArray, width: Int, height: Int): String? {
    val source = PlanarYUVLuminanceSource(luminance, width, height, 0, 0, width, height, false)
    return read(source) ?: read(source.invert())
}

/** What a safety number's QR code says: a prefix, so it is never mistaken for a link, and the thirty digits. */
const val SAFETY_PREFIX = "deaddrop-safety:"

fun safetyCode(number: String): String = SAFETY_PREFIX + number.filter { it.isDigit() }

/** Whether a scanned code is the safety number [mine]. Null if what was scanned is not a safety number at all. */
fun safetyMatches(scanned: String, mine: String): Boolean? =
    if (scanned.startsWith(SAFETY_PREFIX)) scanned == safetyCode(mine) else null
