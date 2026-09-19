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
