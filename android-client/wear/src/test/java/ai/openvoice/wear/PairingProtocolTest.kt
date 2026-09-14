package ai.openvoice.wear

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class PairingProtocolTest {
    @Test
    fun `pairing body identifies pixel watch`() {
        val body = JSONObject(pairingRequestBody("  one-time-code  "))

        assertEquals("one-time-code", body.getString("code"))
        assertEquals("wear", body.getJSONObject("device").getString("type"))
        assertEquals("Pixel Watch", body.getJSONObject("device").getString("label"))
    }

    @Test
    fun `pair response extracts device credential`() {
        val credential = parsePairingResponse(
            """{"access_token":"device-secret","base_url":"https://voice.test","device":{"id":"device_watch"}}""",
        )

        assertEquals("device-secret", credential.accessToken)
        assertEquals("https://voice.test", credential.baseUrl)
        assertEquals("device_watch", credential.deviceId)
    }

    @Test
    fun `optional response metadata can be absent`() {
        val credential = parsePairingResponse("""{"access_token":"device-secret"}""")

        assertNull(credential.baseUrl)
        assertNull(credential.deviceId)
    }

    @Test
    fun `missing access token is rejected`() {
        assertThrows(IllegalArgumentException::class.java) {
            parsePairingResponse("""{"token_type":"Bearer"}""")
        }
    }
}
