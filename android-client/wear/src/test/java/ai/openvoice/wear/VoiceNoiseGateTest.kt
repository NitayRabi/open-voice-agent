package ai.openvoice.wear

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VoiceNoiseGateTest {
    @Test
    fun `rejects quiet background and passes speech`() {
        val gate = VoiceNoiseGate()
        assertFalse(gate.shouldPass(ShortArray(640) { 100 }, 640))
        assertTrue(gate.shouldPass(ShortArray(640) { if (it % 2 == 0) 2_000 else -2_000 }, 640))
    }

    @Test
    fun `holds open briefly between speech frames`() {
        val gate = VoiceNoiseGate(holdFrames = 2)
        val speech = ShortArray(640) { 2_000 }
        val silence = ShortArray(640)
        assertTrue(gate.shouldPass(speech, speech.size))
        assertTrue(gate.shouldPass(silence, silence.size))
        assertTrue(gate.shouldPass(silence, silence.size))
        assertFalse(gate.shouldPass(silence, silence.size))
    }
}
