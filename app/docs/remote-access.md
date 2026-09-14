# Remote access and device pairing

The embedded server supports a separate, revocable bearer credential for every
browser, phone, and watch. Plaintext device tokens are returned once; only their
SHA-256 digests are stored in `paired-devices.json` in the app config directory.

## Client flow

1. Discover an unauthenticated server with `GET /api/health`.
2. Exchange the desktop pairing code:

   ```http
   POST /api/access/pair
   Content-Type: application/json

   {"code":"…","device":{"type":"wear","label":"Pixel Watch"}}
   ```

3. Save `access_token` in the platform's secure credential storage. Send it as
   `Authorization: Bearer <token>` on API requests. `X-OVA-Token` remains
   supported for older clients.
4. Open the returned same-origin `realtime_url` (`wss://host/api/realtime` on
   HTTPS) and authenticate its WebSocket upgrade with the same Authorization
   header. Browser clients authenticate with their HttpOnly cookie.

The response also contains `base_url`, a fragment-safe `connection_url`,
`protocol_version`, and `capabilities`. Browser connection links have the shape
`https://host/c#<device-token>`: the fragment never reaches HTTP logs. `/c`
exchanges it through `POST /api/access/session`, installs an HttpOnly,
SameSite=Strict cookie, and removes it from the address bar.

## Management

Device management is intentionally loopback-only:

- `GET /api/access/devices`
- `POST /api/access/devices` with `{"device":{"type":"android","label":"Pixel"}}`
- `DELETE /api/access/devices/<id>`

Equivalent `paired_devices`, `issue_device`, and `revoke_device` Tauri commands
back the desktop Settings UI. A device credential may read settings, receive
events, and delegate work, but it cannot change settings, control the backend,
download/delete models, emit privileged events, or manage other devices.

Revocation rejects every subsequent API request, interrupts an in-flight long
poll within 250 ms, and marks every active realtime proxy session for that
device to close on its next input frame. Voice clients continuously send audio
frames, so live sessions close promptly in normal operation. The proxy is
advertised as `realtime.authenticated-proxy-v1` and
`realtime.live-revocation-v1`; the loopback speech backend is never exposed as
the client endpoint.

## Compatibility

The reusable `web_token` remains an admin bearer token and the existing HTML
pairing form remains available. A successful form pairing now receives its own
device cookie, so revoking that browser no longer signs out every other device.
Changing the reusable pairing code affects legacy/admin-token sessions but does
not revoke already issued device credentials; revoke those individually.

## Tailscale Serve

Enable **Publish privately with Tailscale Serve** in desktop Settings to expose
the complete same-origin UI, API, and `/api/realtime` proxy to the tailnet over
HTTPS. The app runs `tailscale serve --yes http://127.0.0.1:<web-port>` in the
foreground and does not report the `.ts.net` URL until `tailscale serve status
--json` confirms both that hostname and exact proxy target. Stopping the web
server or quitting the app kills the owned foreground process, removing its
claim without resetting unrelated Serve configuration. Set the executable in
Settings or with `OVA_TAILSCALE_BINARY` when it is not on `PATH`.
