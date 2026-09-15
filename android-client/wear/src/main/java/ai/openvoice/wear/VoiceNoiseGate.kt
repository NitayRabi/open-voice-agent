package ai.openvoice.wear

import kotlin.math.sqrt

/**
 * A deliberately conservative gate for an always-open watch microphone.
 * Closed frames are still sent as silence so server-side VAD keeps its timing.
 */
internal class VoiceNoiseGate(
    private val minimumRms: Double = 0.012,
    private val noiseMultiplier: Double = 3.0,
    private val holdFrames: Int = 8,
) {
    private var noiseFloor = 0.003
    private var framesToHold = 0

    fun shouldPass(samples: ShortArray, count: Int): Boolean {
        if (count <= 0) return false
        var energy = 0.0
        for (index in 0 until count) {
            val normalized = samples[index] / 32768.0
            energy += normalized * normalized
        }
        val rms = sqrt(energy / count)
        val threshold = maxOf(minimumRms, noiseFloor * noiseMultiplier)
        if (rms >= threshold) {
            framesToHold = holdFrames
            return true
        }

        // Learn only from frames that look like background noise; speech must
        // not drag the threshold upwards and make following words disappear.
        if (framesToHold == 0) noiseFloor = noiseFloor * 0.98 + rms * 0.02
        if (framesToHold > 0) {
            framesToHold--
            return true
        }
        return false
    }
}
