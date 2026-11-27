package com.decentralizeddeaddrop.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class QrCodesTest {
    /** A picture of a QR code as a camera would give it: brightness values, [scale] pixels to a square. */
    private fun picture(text: String, scale: Int): Triple<ByteArray, Int, Int> {
        val matrix = qrMatrix(text)
        val (width, height) = matrix.width * scale to matrix.height * scale
        val pixels = ByteArray(width * height) { index ->
            val (x, y) = index % width to index / width
            if (matrix[x / scale, y / scale]) 0 else 255.toByte()
        }
        return Triple(pixels, width, height)
    }

    @Test
    fun a_link_shown_as_a_code_is_read_back_exactly() {
        val link = "deaddrop://203.0.113.7:7443/#fp=wmcchgovezu6vnuyxkuy72o3bscfb64dqgl7xp4hbptm5rfnyqiq" +
            "&t=abcdefghijklmnopqrstuvwxyz234567&c=abcdefghijklmnopqrstuvwxyz&o=t2n6vrwrnowcmdaw&ct=abcdefghijklmnopqrstuvwxyz234567"
        val (pixels, width, height) = picture(link, 4)
        assertEquals(link, decodeQr(pixels, width, height))
    }

    @Test
    fun a_code_drawn_light_on_dark_is_read_too() {
        val link = "deaddrop://203.0.113.7:7443/#fp=wmcchgovezu6vnuyxkuy72o3bscfb64dqgl7xp4hbptm5rfnyqiq&t=abcdefghijklmnopqrstuvwxyz234567"
        val (pixels, width, height) = picture(link, 4)
        val inverted = ByteArray(pixels.size) { (255 - (pixels[it].toInt() and 255)).toByte() }
        assertEquals(link, decodeQr(inverted, width, height))
    }

    @Test
    fun a_picture_with_no_code_reads_as_nothing() {
        assertNull(decodeQr(ByteArray(200 * 200) { 127 }, 200, 200))
    }

    @Test
    fun a_safety_number_matches_only_itself() {
        val mine = "43792 05729 00507 13378 10572 01825"
        assertEquals("deaddrop-safety:437920572900507133781057201825", safetyCode(mine))
        assertEquals(true, safetyMatches(safetyCode(mine), mine))
        assertEquals(false, safetyMatches(safetyCode("43792 05729 00507 13378 10572 01826"), mine))
        assertNull("a link is not a safety number", safetyMatches("deaddrop://203.0.113.7:7443/#fp=x", mine))
    }
}
