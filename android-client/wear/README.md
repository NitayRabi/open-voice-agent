# Open Voice for Wear OS

Standalone Pixel Watch / Wear OS companion for an Open Voice node. The watch
stores its node URL and pairing code in Android Keystore-backed encrypted
preferences, then connects directly to the node over HTTP and WebSocket.

## Build and install

```sh
cd android-client
./gradlew :wear:assembleDebug
adb install -r wear/build/outputs/apk/debug/wear-debug.apk
```

Open **Open Voice** on the watch, enter the public or LAN node URL and pairing
code, and tap **Pair**. Tap the round microphone control to start a voice
session; tap the stop control or leave the activity to close the microphone,
speaker, network calls, and WebSocket.

Cleartext HTTP is supported for trusted LAN development. For normal use,
prefer HTTPS with a valid certificate. The self-signed option deliberately
relaxes certificate and hostname verification only for the selected node and
should never be enabled on an untrusted network.
