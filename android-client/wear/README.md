# Open Voice for Wear OS

Standalone Pixel Watch / Wear OS companion for an Open Voice node. The watch
exchanges a pairing code for a per-device access token, stores that token and
the node URL in Android Keystore-backed encrypted preferences, then connects
directly to the node over HTTP and WebSocket.

## Build and install

```sh
cd android-client
./gradlew :wear:assembleDebug
adb install -r wear/build/outputs/apk/debug/wear-debug.apk
```

Open **Open Voice** on the watch, enter the public or LAN node URL and pairing
code, and tap **Pair**. The code is exchanged once for a revocable watch access
token; only that token is encrypted and retained on the watch. Tap the round
microphone control to start a voice session; tap the stop control or leave the
activity to close the microphone, speaker, network calls, and WebSocket. Use
**Forget & re-pair** to erase the saved token and perform a new exchange.
If the node exposes multiple named agents, swipe the round control left or
right to change the current agent; its alias is shown directly below the orb.

## Main Screen Widgets

### 1. Wear OS Tile (Carousel Widget)
Add the **Open Voice Agent** tile to your swipeable carousel:
- Swipe left/right on the watch face to find or add tiles.
- Tap **Add tile** and select **Open Voice Agent**.
- Displays your currently active agent name and a quick-action microphone button.
- Tapping the orb directly launches `MainActivity` and begins voice recording instantly.

### 2. Watch Face Complications (Dial Widget)
Place Open Voice on your watch face dial slots:
- Long press your watch face, tap **Edit** / Customize, and select a complication slot.
- Choose **Open Voice Agent** under providers.
- Supports **Short Text**, **Monochromatic Image**, **Small Image**, and **Long Text** complication slots.
- 1-tap activation: Tapping the complication immediately triggers voice capture with the active agent.

## Security & Connectivity

Cleartext HTTP is supported for trusted LAN development. For normal use,
prefer HTTPS with a valid certificate. The self-signed option deliberately
relaxes certificate and hostname verification only for the selected node and
should never be enabled on an untrusted network.
