package ai.openvoice.wear

import android.app.PendingIntent
import android.content.Intent
import android.graphics.drawable.Icon
import androidx.wear.watchface.complications.data.ComplicationData
import androidx.wear.watchface.complications.data.ComplicationType
import androidx.wear.watchface.complications.data.LongTextComplicationData
import androidx.wear.watchface.complications.data.MonochromaticImage
import androidx.wear.watchface.complications.data.MonochromaticImageComplicationData
import androidx.wear.watchface.complications.data.PlainComplicationText
import androidx.wear.watchface.complications.data.ShortTextComplicationData
import androidx.wear.watchface.complications.data.SmallImage
import androidx.wear.watchface.complications.data.SmallImageComplicationData
import androidx.wear.watchface.complications.data.SmallImageType
import androidx.wear.watchface.complications.datasource.ComplicationRequest
import androidx.wear.watchface.complications.datasource.SuspendingComplicationDataSourceService

class AgentComplicationService : SuspendingComplicationDataSourceService() {
    private val store by lazy { SecureNodeStore(this) }

    override suspend fun onComplicationRequest(request: ComplicationRequest): ComplicationData? {
        val saved = store.load()
        val isPaired = saved != null
        val agentAlias = store.selectedAgent()?.second ?: "Agent"

        val tapIntent = createTapIntent(isPaired)

        return when (request.complicationType) {
            ComplicationType.SHORT_TEXT -> {
                ShortTextComplicationData.Builder(
                    text = PlainComplicationText.Builder(if (isPaired) agentAlias.take(6) else "Pair").build(),
                    contentDescription = PlainComplicationText.Builder(getString(R.string.complication_description)).build(),
                )
                    .setMonochromaticImage(
                        MonochromaticImage.Builder(Icon.createWithResource(this, R.drawable.ic_mic)).build(),
                    )
                    .setTapAction(tapIntent)
                    .build()
            }
            ComplicationType.MONOCHROMATIC_IMAGE -> {
                MonochromaticImageComplicationData.Builder(
                    monochromaticImage = MonochromaticImage.Builder(
                        Icon.createWithResource(this, R.drawable.ic_mic),
                    ).build(),
                    contentDescription = PlainComplicationText.Builder(getString(R.string.complication_description)).build(),
                )
                    .setTapAction(tapIntent)
                    .build()
            }
            ComplicationType.SMALL_IMAGE -> {
                SmallImageComplicationData.Builder(
                    smallImage = SmallImage.Builder(
                        Icon.createWithResource(this, R.mipmap.ic_launcher),
                        SmallImageType.ICON,
                    ).build(),
                    contentDescription = PlainComplicationText.Builder(getString(R.string.complication_description)).build(),
                )
                    .setTapAction(tapIntent)
                    .build()
            }
            ComplicationType.LONG_TEXT -> {
                LongTextComplicationData.Builder(
                    text = PlainComplicationText.Builder(
                        if (isPaired) "Talk to $agentAlias" else getString(R.string.open_app_to_pair),
                    ).build(),
                    contentDescription = PlainComplicationText.Builder(getString(R.string.complication_description)).build(),
                )
                    .setTitle(PlainComplicationText.Builder(getString(R.string.app_name)).build())
                    .setMonochromaticImage(
                        MonochromaticImage.Builder(Icon.createWithResource(this, R.drawable.ic_mic)).build(),
                    )
                    .setTapAction(tapIntent)
                    .build()
            }
            else -> null
        }
    }

    override fun getPreviewData(type: ComplicationType): ComplicationData? {
        val tapIntent = createTapIntent(true)
        return when (type) {
            ComplicationType.SHORT_TEXT -> {
                ShortTextComplicationData.Builder(
                    text = PlainComplicationText.Builder("Agent").build(),
                    contentDescription = PlainComplicationText.Builder(getString(R.string.complication_description)).build(),
                )
                    .setMonochromaticImage(
                        MonochromaticImage.Builder(Icon.createWithResource(this, R.drawable.ic_mic)).build(),
                    )
                    .setTapAction(tapIntent)
                    .build()
            }
            ComplicationType.MONOCHROMATIC_IMAGE -> {
                MonochromaticImageComplicationData.Builder(
                    monochromaticImage = MonochromaticImage.Builder(
                        Icon.createWithResource(this, R.drawable.ic_mic),
                    ).build(),
                    contentDescription = PlainComplicationText.Builder(getString(R.string.complication_description)).build(),
                )
                    .setTapAction(tapIntent)
                    .build()
            }
            ComplicationType.SMALL_IMAGE -> {
                SmallImageComplicationData.Builder(
                    smallImage = SmallImage.Builder(
                        Icon.createWithResource(this, R.mipmap.ic_launcher),
                        SmallImageType.ICON,
                    ).build(),
                    contentDescription = PlainComplicationText.Builder(getString(R.string.complication_description)).build(),
                )
                    .setTapAction(tapIntent)
                    .build()
            }
            ComplicationType.LONG_TEXT -> {
                LongTextComplicationData.Builder(
                    text = PlainComplicationText.Builder("Talk to Agent").build(),
                    contentDescription = PlainComplicationText.Builder(getString(R.string.complication_description)).build(),
                )
                    .setTitle(PlainComplicationText.Builder(getString(R.string.app_name)).build())
                    .setMonochromaticImage(
                        MonochromaticImage.Builder(Icon.createWithResource(this, R.drawable.ic_mic)).build(),
                    )
                    .setTapAction(tapIntent)
                    .build()
            }
            else -> null
        }
    }

    private fun createTapIntent(isPaired: Boolean): PendingIntent {
        val intent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            if (isPaired) {
                action = MainActivity.ACTION_START_VOICE
                putExtra(MainActivity.EXTRA_AUTO_START, true)
            }
        }
        return PendingIntent.getActivity(
            this,
            0,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }
}
