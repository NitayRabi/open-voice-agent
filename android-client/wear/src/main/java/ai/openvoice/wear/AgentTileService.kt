package ai.openvoice.wear

import androidx.wear.protolayout.ActionBuilders
import androidx.wear.protolayout.ColorBuilders
import androidx.wear.protolayout.DimensionBuilders.dp
import androidx.wear.protolayout.LayoutElementBuilders
import androidx.wear.protolayout.ModifiersBuilders
import androidx.wear.protolayout.ResourceBuilders
import androidx.wear.protolayout.TimelineBuilders
import androidx.wear.protolayout.material.Button
import androidx.wear.protolayout.material.ButtonColors
import androidx.wear.protolayout.material.CompactChip
import androidx.wear.protolayout.material.Text
import androidx.wear.protolayout.material.Typography
import androidx.wear.protolayout.material.layouts.PrimaryLayout
import androidx.wear.tiles.RequestBuilders
import androidx.wear.tiles.TileBuilders
import androidx.wear.tiles.TileService
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture

class AgentTileService : TileService() {
    private val store by lazy { SecureNodeStore(this) }

    override fun onTileRequest(requestParams: RequestBuilders.TileRequest): ListenableFuture<TileBuilders.Tile> {
        val saved = store.load()
        val deviceParams = requestParams.deviceConfiguration
        val isPaired = saved != null
        val agentAlias = store.selectedAgent()?.second ?: "Agent"

        val launchAction = ActionBuilders.LaunchAction.Builder()
            .setAndroidActivity(
                ActionBuilders.AndroidActivity.Builder()
                    .setPackageName(packageName)
                    .setClassName(MainActivity::class.java.name)
                    .build(),
            )
            .build()

        val clickModifier = ModifiersBuilders.Clickable.Builder()
            .setId("open_agent")
            .setOnClick(launchAction)
            .build()

        val primaryLayout = PrimaryLayout.Builder(deviceParams)
            .setPrimaryLabelTextContent(
                Text.Builder(this, getString(R.string.app_name))
                    .setTypography(Typography.TYPOGRAPHY_CAPTION1)
                    .setColor(ColorBuilders.argb(COLOR_MUTED))
                    .build(),
            )
            .setContent(
                Button.Builder(this, clickModifier)
                    .setIconContent(ID_MIC_ICON)
                    .setSize(dp(64f))
                    .setButtonColors(
                        ButtonColors(
                            ColorBuilders.argb(if (isPaired) COLOR_PRIMARY else COLOR_PAIRED_BG),
                            ColorBuilders.argb(COLOR_WHITE),
                        ),
                    )
                    .build(),
            )
            .setSecondaryLabelTextContent(
                Text.Builder(this, if (isPaired) agentAlias else getString(R.string.pair_watch))
                    .setTypography(Typography.TYPOGRAPHY_TITLE3)
                    .setColor(ColorBuilders.argb(COLOR_WHITE))
                    .build(),
            )
            .setPrimaryChipContent(
                CompactChip.Builder(
                    this,
                    if (isPaired) getString(R.string.tap_to_talk) else getString(R.string.open_app_to_pair),
                    clickModifier,
                    deviceParams,
                ).build(),
            )
            .build()

        val timelineEntry = TimelineBuilders.TimelineEntry.Builder()
            .setLayout(LayoutElementBuilders.Layout.Builder().setRoot(primaryLayout).build())
            .build()

        val timeline = TimelineBuilders.Timeline.Builder()
            .addTimelineEntry(timelineEntry)
            .build()

        val tile = TileBuilders.Tile.Builder()
            .setResourcesVersion(RESOURCES_VERSION)
            .setTileTimeline(timeline)
            .build()

        return Futures.immediateFuture(tile)
    }

    override fun onTileResourcesRequest(requestParams: RequestBuilders.ResourcesRequest): ListenableFuture<ResourceBuilders.Resources> {
        val resources = ResourceBuilders.Resources.Builder()
            .setVersion(RESOURCES_VERSION)
            .addIdToImageMapping(
                ID_MIC_ICON,
                ResourceBuilders.ImageResource.Builder()
                    .setAndroidResourceByResId(
                        ResourceBuilders.AndroidImageResourceByResId.Builder()
                            .setResourceId(R.drawable.ic_mic)
                            .build(),
                    )
                    .build(),
            )
            .build()

        return Futures.immediateFuture(resources)
    }

    private companion object {
        const val RESOURCES_VERSION = "1"
        const val ID_MIC_ICON = "mic_icon"
        const val COLOR_PRIMARY = 0xFF14B8A6.toInt() // Vibrant cyan/teal
        const val COLOR_PAIRED_BG = 0xFF374151.toInt()
        const val COLOR_MUTED = 0xFF9CA3AF.toInt()
        const val COLOR_WHITE = 0xFFFFFFFF.toInt()
    }
}
