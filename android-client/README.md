# Open Voice Agent for Android

A remote-only Android client for an Open Voice Agent node. It does not bundle,
download, or start a model or server. The app fetches the node's client settings
by exchanging the desktop pairing code for a revocable device credential. Voice
traffic uses the node's authenticated realtime proxy, and delegated tasks route
back through the node to its configured ACP provider (OpenClaw by default).

## Use

1. In the desktop app, enable **Settings → Web**, bind it to a reachable address,
   and set a pairing code. Use TLS when accessing it outside a trusted LAN.
2. Enter that web URL (for example `https://voice.example.com`) and pairing code
   in the Android phone or Wear OS app. The code is not retained after pairing.
3. Tap **Connect**, or enable the floating orb and tap it from any app.

When the node has named agent profiles, their alias appears beside the orb.
Tap the alias to cycle profiles. The phone remembers its own selection and the
floating orb reads it for each new delegated task.

If the node uses the desktop app's generated/self-signed TLS certificate,
enable **Trust this node's self-signed certificate**. Keep it off for public
servers with a normal trusted certificate. On recent Android versions a
sideloaded APK may require **App info → ⋮ → Allow restricted settings** before
Android will allow the display-over-other-apps permission.

The floating orb is a foreground service while enabled, so Android shows a
persistent notification. The notification can toggle voice or close the orb.
The microphone is only opened while a voice session is active.

## Build

Open `android-client/` in Android Studio (JDK 17, Android SDK 35) and run the
`app` configuration, or use the included Gradle wrapper. On this machine:

```bash
cd android-client
ANDROID_HOME=/home/nitayrabi/android-sdk ./gradlew :app:assembleDebug
```

The phone APK is written to `app/build/outputs/apk/debug/app-debug.apk`.

To build the Pixel Watch companion as well:

```bash
ANDROID_HOME=/home/nitayrabi/android-sdk ./gradlew :wear:assembleDebug
```

The Wear OS APK is written to `wear/build/outputs/apk/debug/wear-debug.apk`.

For development against a LAN node, cleartext HTTP is enabled. Production
deployments should use HTTPS/WSS with a certificate trusted by the phone.
