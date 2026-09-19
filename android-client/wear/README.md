# Open Voice for Wear OS

Pixel Watch / Wear OS companion for an Open Voice node. The watch can be
configured directly by syncing with the phone app over the Wearable Data Layer,
or paired standalone by entering a node URL and pairing code. It stores the
per-device access token and node URL in Android Keystore-backed encrypted
preferences, then connects directly to the node over HTTP and WebSocket.

## Build and install

```sh
cd android-client
./gradlew :wear:assembleDebug
adb install -r wear/build/outputs/apk/debug/wear-debug.apk
```

## Setup & Pairing

1. **Automatic sync via Phone App (Recommended)**:
   - Pair the phone app with your node.
   - Open **Open Voice** on your watch; it will automatically request and receive the node URL and credentials from the paired phone companion.
2. **Manual standalone pairing**:
   - Open **Open Voice** on the watch and tap **Enter manually**.
   - Enter your public or LAN node URL (e.g. `https://voice.nyr-solutions.org`) and pairing code, and tap **Pair**.

Tap the round microphone control to start a voice session; tap the stop control or leave the activity to close the microphone, speaker, network calls, and WebSocket. Use **Forget & re-pair** to erase the saved token and automatically attempt to re-sync with the phone companion before falling back to manual entry. If the node exposes multiple named agents, swipe the round control left or right to change the current agent.

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
