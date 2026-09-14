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
4. Use the returned `realtime_url` for OpenAI Realtime voice traffic.

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

Revocation rejects every subsequent API request and interrupts an in-flight
long poll within 250 ms. The current speech transport still connects directly
to the separately hosted OpenAI-compatible realtime server, so it cannot be
terminated by the embedded HTTP server. This is exposed honestly as
`realtime_auth: "direct-backend-unenforced"` and capability
`realtime.openai-direct-v1`; clients should close their local realtime socket
when an API request reports `device_credential_invalid`. Do not advertise
auth-enforced realtime or immediate realtime revocation until the speech socket
is routed through an authenticated proxy.

## Compatibility

The reusable `web_token` remains an admin bearer token and the existing HTML
pairing form remains available. A successful form pairing now receives its own
device cookie, so revoking that browser no longer signs out every other device.
Changing the reusable pairing code affects legacy/admin-token sessions but does
not revoke already issued device credentials; revoke those individually.
