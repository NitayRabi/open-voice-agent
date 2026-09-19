package ai.openvoice.wear

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WearConfigSyncTest {
    @Test
    fun `parseJson parses valid configuration payload with credential`() {
        val json = """
            {
                "url": "https://voice.nyr-solutions.org/",
                "credential": "test-device-token-123",
                "trust_self_signed": true,
                "timestamp": 1710000000000
            }
        """.trimIndent()

        val parsed = WearConfigSyncHelper.parseJson(json)
        assertNotNull(parsed)
        assertEquals("https://voice.nyr-solutions.org", parsed?.url)
        assertEquals("test-device-token-123", parsed?.accessToken)
        assertTrue(parsed?.trustSelfSigned == true)
    }

    @Test
    fun `parseJson parses valid configuration payload with access_token`() {
        val json = """
            {
                "url": "https://voice.nyr-solutions.org",
                "access_token": "token-xyz",
                "trust_self_signed": false
            }
        """.trimIndent()

        val parsed = WearConfigSyncHelper.parseJson(json)
        assertNotNull(parsed)
        assertEquals("https://voice.nyr-solutions.org", parsed?.url)
        assertEquals("token-xyz", parsed?.accessToken)
        assertFalse(parsed?.trustSelfSigned == true)
    }

    @Test
    fun `parseJson rejects missing url or token`() {
        assertNull(WearConfigSyncHelper.parseJson("""{"url": "https://voice.nyr-solutions.org"}"""))
        assertNull(WearConfigSyncHelper.parseJson("""{"credential": "token-123"}"""))
        assertNull(WearConfigSyncHelper.parseJson("""invalid json"""))
    }
}
